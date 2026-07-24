use super::*;
use crate::model::OfficeFormat;
use crate::package::OfficePackage;
use crate::test_support::{temp_path, xlsx_fixture};

#[test]
fn reads_sheet_types_formulas_and_structure() {
    let path = temp_path("xlsx");
    xlsx_fixture(&path);
    let package = OfficePackage::open(&path, Some(OfficeFormat::Xlsx)).expect("package");
    let workbook = read_workbook(&package).expect("workbook");
    assert_eq!(workbook.sheets[0].name, "数据");
    assert_eq!(workbook.sheets[0].cells.len(), 4);
    assert_eq!(
        workbook.sheets[0].cells[3].formula.as_deref(),
        Some("SUM(40,2)")
    );
    assert_eq!(workbook.sheets[0].auto_filter.as_deref(), Some("A1:B2"));
    std::fs::remove_file(path).ok();
}
