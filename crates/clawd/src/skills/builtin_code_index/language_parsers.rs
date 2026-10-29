use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use tree_sitter::{Language, Node, ParseOptions, Parser};

use super::{SymbolDefinition, SymbolReference};

const PARSE_BUDGET: Duration = Duration::from_millis(250);
const MAX_ENTRIES: usize = 50_000;

pub(super) struct ParsedSource {
    pub(super) status: &'static str,
    pub(super) symbols: Vec<SymbolDefinition>,
    pub(super) references: Vec<SymbolReference>,
}

pub(super) fn parse(source: &str, path: &Path, language: &str) -> Option<ParsedSource> {
    let grammar: Language = match language {
        "python" => tree_sitter_python::LANGUAGE.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript"
            if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("tsx")) =>
        {
            tree_sitter_typescript::LANGUAGE_TSX.into()
        }
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        _ => return None,
    };
    let mut output = ParsedSource {
        status: "parsed",
        symbols: Vec::new(),
        references: Vec::new(),
    };
    let mut parser = Parser::new();
    if parser.set_language(&grammar).is_err() {
        output.status = "parser_unavailable";
        return Some(output);
    }
    let started = Instant::now();
    let mut cancel = |_: &tree_sitter::ParseState| started.elapsed() >= PARSE_BUDGET;
    let bytes = source.as_bytes();
    let mut input = |offset, _| &bytes[offset..];
    let Some(tree) = parser.parse_with_options(
        &mut input,
        None,
        Some(ParseOptions::new().progress_callback(&mut cancel)),
    ) else {
        output.status = "parse_budget_exceeded";
        return Some(output);
    };
    if tree.root_node().has_error() {
        output.status = "parsed_with_errors";
    }
    let mut collector = Collector {
        source: bytes,
        language,
        scope: Vec::new(),
        binding_nodes: BTreeSet::new(),
        output,
    };
    // Iterative traversal keeps deeply nested untrusted source off the Rust stack.
    let mut cursor = tree.walk();
    loop {
        if started.elapsed() >= PARSE_BUDGET
            || collector.output.symbols.len() + collector.output.references.len() >= MAX_ENTRIES
        {
            collector.output.status = "index_budget_exceeded";
            return Some(collector.output);
        }
        let node = cursor.node();
        let scoped = collector.enter(node);
        let skip = matches!(
            node.kind(),
            "string" | "string_fragment" | "comment" | "jsx_text"
        );
        if !skip && cursor.goto_first_child() {
            continue;
        }
        if scoped {
            collector.scope.pop();
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                collector.output.symbols.sort_by(|a, b| {
                    a.line
                        .cmp(&b.line)
                        .then(a.qualified_name.cmp(&b.qualified_name))
                });
                collector.output.symbols.dedup();
                collector.output.references.sort();
                collector.output.references.dedup();
                return Some(collector.output);
            }
            if collector.is_scope(cursor.node()) {
                collector.scope.pop();
            }
        }
    }
}

struct Collector<'a> {
    source: &'a [u8],
    language: &'a str,
    scope: Vec<String>,
    binding_nodes: BTreeSet<usize>,
    output: ParsedSource,
}

impl Collector<'_> {
    fn text(&self, node: Node<'_>) -> String {
        node.utf8_text(self.source).unwrap_or_default().to_string()
    }

    fn definition(&mut self, name: Node<'_>, node: Node<'_>, kind: &str, is_test: bool) -> String {
        self.binding_nodes.insert(name.id());
        let name = self.text(name);
        let qualified_name = self
            .scope
            .iter()
            .map(String::as_str)
            .chain([name.as_str()])
            .collect::<Vec<_>>()
            .join("::");
        self.output.symbols.push(SymbolDefinition {
            name: name.clone(),
            qualified_name,
            kind: kind.to_string(),
            line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            visibility: "unspecified".to_string(),
            is_test,
            source: "tree_sitter_ast".to_string(),
        });
        name
    }

    fn definition_kind(&self, node: Node<'_>) -> Option<&'static str> {
        match node.kind() {
            "function_definition"
            | "function_declaration"
            | "generator_function_declaration"
            | "function_expression" => Some("function"),
            "method_definition" | "method_signature" | "abstract_method_signature" => {
                Some("method")
            }
            "class_definition" | "class_declaration" | "abstract_class_declaration" => {
                Some("class")
            }
            "interface_declaration" => Some("interface"),
            "type_alias_declaration" => Some("type"),
            "enum_declaration" => Some("enum"),
            "variable_declarator" => Some(
                if node.child_by_field_name("value").is_some_and(|value| {
                    matches!(value.kind(), "arrow_function" | "function_expression")
                }) {
                    "function"
                } else {
                    "variable"
                },
            ),
            _ => None,
        }
    }

    fn is_scope(&self, node: Node<'_>) -> bool {
        matches!(
            self.definition_kind(node),
            Some("function" | "method" | "class" | "interface")
        ) && node.child_by_field_name("name").is_some_and(|name| {
            matches!(
                name.kind(),
                "identifier" | "property_identifier" | "type_identifier"
            )
        })
    }

    fn enter(&mut self, node: Node<'_>) -> bool {
        let mut scoped = false;
        if let (Some(kind), Some(name)) =
            (self.definition_kind(node), node.child_by_field_name("name"))
        {
            if matches!(
                name.kind(),
                "identifier" | "property_identifier" | "type_identifier"
            ) {
                let is_test = self.language == "python" && self.text(name).starts_with("test_");
                let name = self.definition(name, node, kind, is_test);
                scoped = self.is_scope(node);
                if scoped {
                    self.scope.push(name);
                }
            }
        }
        if self.language == "python" && node.kind() == "assignment" {
            if let Some(name) = node
                .child_by_field_name("left")
                .filter(|n| n.kind() == "identifier")
            {
                self.definition(name, node, "variable", false);
            }
        }
        if matches!(node.kind(), "import_specifier" | "aliased_import") {
            if let Some(name) = node
                .child_by_field_name("alias")
                .or_else(|| node.child_by_field_name("name"))
            {
                if name.kind() == "identifier" {
                    self.definition(name, node, "import", false);
                }
            }
        }
        if node.kind() == "identifier" && self.parameter_binding(node) {
            self.definition(node, node, "parameter", false);
        }
        if matches!(
            node.kind(),
            "identifier"
                | "type_identifier"
                | "property_identifier"
                | "shorthand_property_identifier"
        ) && !self.binding_nodes.contains(&node.id())
            && !self.nonreference_property(node)
        {
            let kind = if node.parent().is_some_and(|n| {
                matches!(
                    n.kind(),
                    "import_statement"
                        | "import_from_statement"
                        | "import_specifier"
                        | "dotted_name"
                        | "aliased_import"
                )
            }) {
                "import"
            } else if node.kind() == "property_identifier" {
                "member"
            } else {
                "identifier"
            };
            self.output.references.push(SymbolReference {
                name: self.text(node),
                line: node.start_position().row + 1,
                kind: kind.to_string(),
                scope: self.scope.join("::"),
            });
        }
        if matches!(node.kind(), "call_expression") {
            self.test_call(node);
        }
        scoped
    }

    fn parameter_binding(&self, node: Node<'_>) -> bool {
        let Some(parent) = node.parent() else {
            return false;
        };
        matches!(parent.kind(), "parameters" | "formal_parameters")
            || (matches!(
                parent.kind(),
                "required_parameter"
                    | "optional_parameter"
                    | "default_parameter"
                    | "typed_parameter"
            ) && (parent
                .child_by_field_name("pattern")
                .or_else(|| parent.child_by_field_name("name"))
                .is_some_and(|n| n.id() == node.id())))
    }

    fn nonreference_property(&self, node: Node<'_>) -> bool {
        node.parent().is_some_and(|parent| {
            matches!(parent.kind(), "pair" | "pair_pattern")
                && parent
                    .child_by_field_name("key")
                    .is_some_and(|key| key.id() == node.id())
        })
    }

    fn test_call(&mut self, node: Node<'_>) {
        let Some(function) = node.child_by_field_name("function") else {
            return;
        };
        let function = self.text(function);
        if !matches!(
            function.as_str(),
            "it" | "test"
                | "describe"
                | "it.only"
                | "test.only"
                | "it.skip"
                | "test.skip"
                | "describe.only"
        ) {
            return;
        }
        let Some(args) = node.child_by_field_name("arguments") else {
            return;
        };
        if let Some(name) = args.named_child(0).filter(|name| name.kind() == "string") {
            self.definition(
                name,
                node,
                if function.starts_with("describe") {
                    "test_suite"
                } else {
                    "test"
                },
                !function.starts_with("describe"),
            );
        }
    }
}

#[cfg(test)]
#[path = "language_parsers_tests.rs"]
mod tests;
