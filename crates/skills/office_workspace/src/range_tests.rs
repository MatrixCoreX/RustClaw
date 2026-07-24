use super::*;

#[test]
fn parses_and_formats_cell_coordinates() {
    assert_eq!(
        parse_coordinate("$AA$42").expect("coordinate"),
        CellCoordinate {
            row: 42,
            column: 27
        }
    );
    assert_eq!(
        format_coordinate(CellCoordinate {
            row: 42,
            column: 27
        }),
        "AA42"
    );
}

#[test]
fn range_contains_only_bounded_cells() {
    let range = CellRange::parse("B2:D4").expect("range");
    assert!(range.contains(parse_coordinate("C3").expect("coordinate")));
    assert!(!range.contains(parse_coordinate("A1").expect("coordinate")));
}
