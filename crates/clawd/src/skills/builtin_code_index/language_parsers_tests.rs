use super::*;

fn parsed(source: &str, filename: &str, language: &str) -> ParsedSource {
    let result = parse(source, Path::new(filename), language).expect("supported grammar");
    assert_eq!(result.status, "parsed");
    result
}

#[test]
fn python_aliases_nested_scopes_unicode_and_tests_are_parser_observations() {
    let result = parsed("from math import sqrt as root\n\nclass Example:\n    def test_value(self):\n        value = root(4)\n        return value\n\ndef 外层(值):\n    def inner():\n        return 值\n    return inner()\n", "sample.py", "python");
    for (name, kind) in [
        ("root", "import"),
        ("Example::test_value", "function"),
        ("外层::inner", "function"),
        ("外层::值", "parameter"),
    ] {
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.qualified_name == name && s.kind == kind),
            "missing {name}: {:?}",
            result.symbols
        );
    }
    assert!(result
        .symbols
        .iter()
        .any(|s| s.name == "test_value" && s.is_test));
    assert!(result
        .references
        .iter()
        .any(|r| r.name == "root" && r.scope == "Example::test_value"));
    assert!(result
        .references
        .iter()
        .any(|r| r.name == "值" && r.scope == "外层::inner"));
}

#[test]
fn javascript_and_jsx_include_aliases_arrow_scopes_tests_and_member_candidates() {
    let result = parsed("import { helper as run } from './helper';\nexport const render = (input) => {\n  const value = run(input);\n  return <View title={value}/>;\n};\ntest('renders correctly', () => { render('hello'); });\nclass Client { execute(item) { return item.run(); } }\n", "sample.jsx", "javascript");
    assert!(result
        .symbols
        .iter()
        .any(|s| s.name == "run" && s.kind == "import"));
    assert!(result
        .symbols
        .iter()
        .any(|s| s.qualified_name == "render::input" && s.kind == "parameter"));
    assert!(result
        .symbols
        .iter()
        .any(|s| s.qualified_name == "Client::execute"));
    assert!(result.symbols.iter().any(|s| s.kind == "test" && s.is_test));
    assert!(result
        .references
        .iter()
        .any(|r| r.name == "run" && r.scope == "render"));
    assert!(result
        .references
        .iter()
        .any(|r| r.name == "run" && r.kind == "member" && r.scope == "Client::execute"));
}

#[test]
fn typescript_and_tsx_use_their_own_grammars() {
    for (name, source) in [("sample.ts", "export interface Item { value: number }\nexport type Key = string;\nexport function read(item: Item): number { return item.value; }"),
        ("sample.tsx", "interface Props { title: string }\nexport function Card(props: Props) { return <div>{props.title}</div>; }")] {
        let result = parsed(source, name, "typescript");
        assert!(result.symbols.iter().any(|s| s.kind == "interface"));
        assert!(result.symbols.iter().any(|s| s.kind == "function"));
    }
}

#[test]
fn comments_and_strings_are_not_symbol_reference_evidence() {
    for (name, language, source) in [
        (
            "sample.py",
            "python",
            "# phantom()\ntext = 'phantom()'\ndef actual():\n    return 1\n",
        ),
        (
            "sample.ts",
            "typescript",
            "// phantom();\nconst text = 'phantom()';\nfunction actual() { return 1; }",
        ),
    ] {
        let result = parsed(source, name, language);
        assert!(!result.references.iter().any(|r| r.name == "phantom"));
        assert!(!result.symbols.iter().any(|s| s.name == "phantom"));
    }
}

#[test]
fn shadowed_names_remain_scope_labelled_candidates_not_false_resolutions() {
    let result = parsed(
        "function outer(value) { function inner(value) { return value; } return value; }",
        "sample.js",
        "javascript",
    );
    for scope in ["outer", "outer::inner"] {
        assert!(result
            .references
            .iter()
            .any(|r| r.name == "value" && r.scope == scope));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.qualified_name == format!("{scope}::value")));
    }
}

#[test]
fn invalid_source_and_unsupported_languages_do_not_claim_full_parsing() {
    assert_ne!(
        parse("def broken(:\n", Path::new("broken.py"), "python")
            .unwrap()
            .status,
        "parsed"
    );
    assert!(parse("package main", Path::new("main.go"), "go").is_none());
}

#[test]
fn uppercase_tsx_extension_keeps_the_correct_dialect() {
    let result = parsed(
        "export const View = () => <div>text</div>;",
        "VIEW.TSX",
        "typescript",
    );
    assert!(result.symbols.iter().any(|symbol| symbol.name == "View"));
}

#[test]
fn high_density_source_has_an_explicit_bounded_partial_state() {
    let source = "item();\n".repeat(MAX_ENTRIES + 1);
    let result = parse(&source, Path::new("dense.js"), "javascript").unwrap();
    assert!(matches!(
        result.status,
        "index_budget_exceeded" | "parse_budget_exceeded"
    ));
    assert!(result.references.len() <= MAX_ENTRIES);
}
