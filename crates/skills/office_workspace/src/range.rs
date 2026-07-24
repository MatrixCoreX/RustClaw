use crate::error::{OfficeError, OfficeResult};
use serde_json::json;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellCoordinate {
    pub row: u32,
    pub column: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellRange {
    pub start: CellCoordinate,
    pub end: CellCoordinate,
}

impl CellRange {
    pub fn parse(value: &str) -> OfficeResult<Self> {
        let without_sheet = value
            .rsplit_once('!')
            .map(|(_, range)| range)
            .unwrap_or(value);
        let mut parts = without_sheet.split(':');
        let start = parse_coordinate(parts.next().unwrap_or_default())?;
        let end = match parts.next() {
            Some(value) => parse_coordinate(value)?,
            None => start,
        };
        if parts.next().is_some() {
            return Err(invalid_range(value));
        }
        Ok(Self {
            start: CellCoordinate {
                row: start.row.min(end.row),
                column: start.column.min(end.column),
            },
            end: CellCoordinate {
                row: start.row.max(end.row),
                column: start.column.max(end.column),
            },
        })
    }

    pub fn contains(self, coordinate: CellCoordinate) -> bool {
        coordinate.row >= self.start.row
            && coordinate.row <= self.end.row
            && coordinate.column >= self.start.column
            && coordinate.column <= self.end.column
    }
}

pub fn parse_coordinate(value: &str) -> OfficeResult<CellCoordinate> {
    let value = value.trim().replace('$', "");
    if value.is_empty() {
        return Err(invalid_range(value));
    }
    let split = value
        .find(|character: char| character.is_ascii_digit())
        .ok_or_else(|| invalid_range(&value))?;
    let (column, row) = value.split_at(split);
    if column.is_empty()
        || column.len() > 3
        || !column
            .chars()
            .all(|character| character.is_ascii_alphabetic())
        || row.is_empty()
        || !row.chars().all(|character| character.is_ascii_digit())
    {
        return Err(invalid_range(&value));
    }
    let mut column_number = 0u32;
    for character in column.to_ascii_uppercase().bytes() {
        column_number = column_number
            .checked_mul(26)
            .and_then(|value| value.checked_add((character - b'A' + 1) as u32))
            .ok_or_else(|| invalid_range(&value))?;
    }
    let row_number = row.parse::<u32>().map_err(|_| invalid_range(&value))?;
    if row_number == 0 || row_number > 1_048_576 || column_number == 0 || column_number > 16_384 {
        return Err(invalid_range(&value));
    }
    Ok(CellCoordinate {
        row: row_number,
        column: column_number,
    })
}

pub fn format_coordinate(coordinate: CellCoordinate) -> String {
    format!("{}{}", format_column(coordinate.column), coordinate.row)
}

pub fn format_column(mut column: u32) -> String {
    let mut bytes = Vec::new();
    while column > 0 {
        column -= 1;
        bytes.push(b'A' + (column % 26) as u8);
        column /= 26;
    }
    bytes.reverse();
    String::from_utf8(bytes).unwrap_or_default()
}

fn invalid_range(value: impl AsRef<str>) -> OfficeError {
    OfficeError::new(
        "invalid_cell_range",
        "cell range must use A1 notation",
        json!({"range": value.as_ref()}),
    )
}

#[cfg(test)]
#[path = "range_tests.rs"]
mod tests;
