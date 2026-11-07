use std::cell::{Cell, RefCell};

use super::*;

/// Bound bytes consumed while decoding one key/metadata value/item, including
/// separators/lookahead. Long arrays remain streaming instead of one giant value.
pub(super) struct Budget {
    max_value_bytes: u64,
    position: Cell<u64>,
    end: Cell<Option<u64>>,
    exceeded: Cell<bool>,
    io_error: RefCell<Option<io::Error>>,
}

impl Budget {
    pub(super) fn new(max_value_bytes: u64) -> Self {
        Self {
            max_value_bytes,
            position: Cell::new(0),
            end: Cell::new(Some(max_value_bytes)),
            exceeded: Cell::new(false),
            io_error: RefCell::new(None),
        }
    }
    pub(super) fn begin_value(&self) {
        self.end.set(Some(
            self.position.get().saturating_add(self.max_value_bytes),
        ));
    }
    pub(super) fn disable_value_limit(&self) {
        self.end.set(None);
    }
    pub(super) fn exhausted(&self) -> bool {
        self.exceeded.get()
    }
    pub(super) fn take_io_error(&self) -> Option<io::Error> {
        self.io_error.borrow_mut().take()
    }
}

pub(super) struct BoundedReader<'a, R> {
    inner: R,
    budget: &'a Budget,
}
impl<'a, R> BoundedReader<'a, R> {
    pub(super) fn new(inner: R, budget: &'a Budget) -> Self {
        Self { inner, budget }
    }
}
impl<R: Read> Read for BoundedReader<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let remaining = self
            .budget
            .end
            .get()
            .map(|end| end.saturating_sub(self.budget.position.get()));
        let result = if remaining == Some(0) {
            let mut probe = [0u8; 1];
            match self.inner.read(&mut probe) {
                Ok(0) => Ok(0),
                Ok(_) => {
                    self.budget.exceeded.set(true);
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        UploadError::ValueTooLarge,
                    ));
                }
                Err(error) => Err(error),
            }
        } else {
            let count = remaining.map_or(output.len(), |n| {
                output.len().min(usize::try_from(n).unwrap_or(usize::MAX))
            });
            self.inner.read(&mut output[..count])
        };
        match result {
            Ok(count) => {
                self.budget
                    .position
                    .set(self.budget.position.get().saturating_add(count as u64));
                Ok(count)
            }
            Err(error) => {
                let kind = error.kind();
                *self.budget.io_error.borrow_mut() = Some(error);
                Err(io::Error::new(kind, "memory_import_upload_read_failed"))
            }
        }
    }
}
