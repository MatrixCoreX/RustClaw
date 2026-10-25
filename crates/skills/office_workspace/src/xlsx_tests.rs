use super::*;
use crate::model::OfficeFormat;
use crate::package::OfficePackage;
use crate::test_support::{temp_path, xlsx_fixture};

#[test]
fn workbook_references_keep_formulas_strings_ranges_and_comments_intact() {
    assert_eq!(
        parse_shared_strings("<sst><si><t> A &amp; B &#x1F642; </t></si></sst>"),
        [" A & B 🙂 "]
    );
    assert_eq!(
        parse_named_ranges(
            "<definedNames><definedName name=\"Joined\"> A &amp; B </definedName></definedNames>"
        ),
        ["Joined=A & B"]
    );
    assert_eq!(collect_text("<t> A &amp; B </t>"), ["A & B"]);
    assert!(collect_text("<t>A & B</t>").is_empty());
    assert!(collect_text("<t>A &unknown; B</t>").is_empty());

    let path = temp_path("xlsx");
    xlsx_fixture(&path);
    let mut package = OfficePackage::open(&path, Some(OfficeFormat::Xlsx)).expect("package");
    package.members.insert(
        "xl/worksheets/_rels/sheet1.xml.rels".into(),
        b"<Relationships><Relationship Id=\"comment\" Type=\"test/comments\" Target=\"../comments1.xml\"/></Relationships>".to_vec(),
    );
    package.members.insert(
        "xl/comments1.xml".into(),
        b"<comments><comment ref=\"A1\"><text><t>A &amp; B</t></text></comment></comments>"
            .to_vec(),
    );
    let sheet = SheetRef {
        id: "1".into(),
        name: "Data".into(),
        state: "visible".into(),
        path: "xl/worksheets/sheet1.xml".into(),
    };
    let evidence = parse_worksheet(
        &package,
        &sheet,
        "<worksheet><sheetData><row><c r=\"A1\" t=\"inlineStr\"><is><t>A &amp; B</t></is></c><c r=\"B1\" t=\"str\"><f>IF(A1&lt;2,&quot;A &amp; B&quot;,&quot;&quot;)</f><v>A &amp; B</v></c></row></sheetData></worksheet>",
        &[],
        &BTreeSet::new(),
    )
    .expect("worksheet reference text");
    assert_eq!(evidence.cells[0].value, Some(json!("A & B")));
    assert_eq!(evidence.cells[0].comment.as_deref(), Some("A & B"));
    assert_eq!(evidence.cells[1].value, Some(json!("A & B")));
    assert_eq!(
        evidence.cells[1].formula.as_deref(),
        Some("IF(A1<2,\"A & B\",\"\")")
    );
    std::fs::remove_file(path).ok();
}

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
