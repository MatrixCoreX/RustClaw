use serde::Serialize;

#[derive(Debug)]
pub(super) enum EncodeError {
    LimitExceeded { limit_bytes: usize },
    Serialization(serde_json::Error),
}

pub(super) fn encode_pretty<T: Serialize>(
    value: &T,
    limit_bytes: usize,
) -> Result<Vec<u8>, EncodeError> {
    struct Buffer {
        bytes: Vec<u8>,
        limit_bytes: usize,
        exceeded: bool,
    }

    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit_bytes.saturating_sub(self.bytes.len()) {
                self.exceeded = true;
                return Err(std::io::Error::other("checkpoint_metadata_size_limit"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut output = Buffer {
        bytes: Vec::new(),
        limit_bytes,
        exceeded: false,
    };
    let result = serde_json::to_writer_pretty(&mut output, value);
    if output.exceeded {
        return Err(EncodeError::LimitExceeded { limit_bytes });
    }
    result.map_err(EncodeError::Serialization)?;
    Ok(output.bytes)
}

#[cfg(test)]
#[path = "builtin_workspace_checkpoint_json_tests.rs"]
mod tests;
