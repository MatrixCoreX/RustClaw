use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use proc_macro2::{Span, TokenStream, TokenTree};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value};
use sha2::{Digest, Sha256};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

const INDEX_SCHEMA_VERSION: u32 = 1;
const INDEX_RELATIVE_PATH: &str = ".rustclaw/index/repository-v1.json";
const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;
const DEFAULT_MAX_FILES: usize = 20_000;
const HARD_MAX_FILES: usize = 50_000;
const DEFAULT_MAX_RESULTS: usize = 40;
const HARD_MAX_RESULTS: usize = 200;
const DEFAULT_CONTEXT_LINES: usize = 2;
const MAX_CONTEXT_LINES: usize = 20;
const MAX_SNIPPET_BYTES: usize = 24 * 1024;

static INDEX_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug)]
pub(super) struct CodeIndexError {
    pub(super) code: &'static str,
    pub(super) detail: String,
    pub(super) extra: Option<Value>,
}

impl CodeIndexError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
            extra: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RepositoryIndex {
    schema_version: u32,
    #[serde(default)]
    generated_at: u64,
    #[serde(default)]
    scan_complete: bool,
    files: BTreeMap<String, IndexedFile>,
}

impl Default for RepositoryIndex {
    fn default() -> Self {
        Self {
            schema_version: INDEX_SCHEMA_VERSION,
            generated_at: 0,
            scan_complete: false,
            files: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexedFile {
    language: String,
    size_bytes: u64,
    modified_ns: u64,
    sha256: String,
    parse_status: String,
    symbols: Vec<SymbolDefinition>,
    references: Vec<SymbolReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SymbolDefinition {
    name: String,
    qualified_name: String,
    kind: String,
    line: usize,
    end_line: usize,
    visibility: String,
    is_test: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct SymbolReference {
    name: String,
    line: usize,
    kind: String,
}

#[derive(Debug, Default)]
struct RefreshStats {
    scanned_files: usize,
    parsed_files: usize,
    reused_files: usize,
    removed_files: usize,
    skipped_files: usize,
    scan_truncated: bool,
    refreshed_at: u64,
}

#[derive(Debug, Clone)]
struct SourceCandidate {
    path: PathBuf,
    relative_path: String,
    language: &'static str,
    size_bytes: u64,
    modified_ns: u64,
}

pub(super) fn execute(workspace_root: &Path, args: &Value) -> Result<String, CodeIndexError> {
    let object = args
        .as_object()
        .ok_or_else(|| CodeIndexError::new("invalid_args", "code_index.args_not_object"))?;
    let action = required_machine_string(object, "action")?;
    let max_files = bounded_usize(
        object.get("max_files"),
        DEFAULT_MAX_FILES,
        1,
        HARD_MAX_FILES,
    )?;

    let lock = INDEX_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock
        .lock()
        .map_err(|_| CodeIndexError::new("index_lock_failed", "code_index.index_lock_failed"))?;
    let (index, refresh) = refresh_index(workspace_root, max_files)?;
    let result = match action {
        "refresh" => json!({
            "schema_version": INDEX_SCHEMA_VERSION,
            "kind": "repository_index",
            "action": action,
            "status_code": "ok",
            "summary": index_summary(&index, &refresh),
        }),
        "search_symbols" => search_symbols(&index, object, &refresh)?,
        "find_definitions" => find_definitions(&index, object, &refresh)?,
        "find_references" => find_references(&index, object, &refresh)?,
        "list_tests" => list_tests(&index, object, &refresh)?,
        "changed_impact" => changed_impact(workspace_root, &index, object, &refresh)?,
        "retrieve_context" => retrieve_context(workspace_root, &index, object, &refresh)?,
        _ => {
            return Err(CodeIndexError::new(
                "unsupported_action",
                format!("code_index.unsupported_action action={action}"),
            ))
        }
    };
    serde_json::to_string(&result)
        .map_err(|error| CodeIndexError::new("serialize_failed", error.to_string()))
}

fn refresh_index(
    workspace_root: &Path,
    max_files: usize,
) -> Result<(RepositoryIndex, RefreshStats), CodeIndexError> {
    let index_path = workspace_root.join(INDEX_RELATIVE_PATH);
    let previous = load_index(&index_path);
    let (candidates, scan_truncated) = collect_source_candidates(workspace_root, max_files)?;
    let mut stats = RefreshStats {
        scan_truncated,
        refreshed_at: crate::now_ts_u64(),
        ..RefreshStats::default()
    };
    let mut files = BTreeMap::new();

    for candidate in candidates {
        stats.scanned_files += 1;
        if let Some(existing) = previous.files.get(&candidate.relative_path) {
            if existing.size_bytes == candidate.size_bytes
                && existing.modified_ns == candidate.modified_ns
            {
                files.insert(candidate.relative_path, existing.clone());
                stats.reused_files += 1;
                continue;
            }
        }
        let indexed = index_source_file(&candidate)?;
        if indexed.parse_status == "parsed" {
            stats.parsed_files += 1;
        } else {
            stats.skipped_files += 1;
        }
        files.insert(candidate.relative_path, indexed);
    }
    stats.removed_files = previous
        .files
        .keys()
        .filter(|path| !files.contains_key(*path))
        .count();
    let index = RepositoryIndex {
        schema_version: INDEX_SCHEMA_VERSION,
        generated_at: stats.refreshed_at,
        scan_complete: !stats.scan_truncated,
        files,
    };
    persist_index(&index_path, &index)?;
    Ok((index, stats))
}

fn load_index(path: &Path) -> RepositoryIndex {
    let Ok(bytes) = fs::read(path) else {
        return RepositoryIndex::default();
    };
    let Ok(index) = serde_json::from_slice::<RepositoryIndex>(&bytes) else {
        return RepositoryIndex::default();
    };
    if index.schema_version != INDEX_SCHEMA_VERSION {
        return RepositoryIndex::default();
    }
    index
}

fn persist_index(path: &Path, index: &RepositoryIndex) -> Result<(), CodeIndexError> {
    let parent = path.parent().ok_or_else(|| {
        CodeIndexError::new("index_path_invalid", "code_index.index_path_invalid")
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| CodeIndexError::new("index_write_failed", error.to_string()))?;
    let bytes = serde_json::to_vec(index)
        .map_err(|error| CodeIndexError::new("serialize_failed", error.to_string()))?;
    let temporary = parent.join(format!(
        ".repository-v1-{}.tmp",
        uuid::Uuid::new_v4().as_simple()
    ));
    fs::write(&temporary, bytes)
        .map_err(|error| CodeIndexError::new("index_write_failed", error.to_string()))?;
    fs::rename(&temporary, path)
        .map_err(|error| CodeIndexError::new("index_write_failed", error.to_string()))
}

fn collect_source_candidates(
    workspace_root: &Path,
    max_files: usize,
) -> Result<(Vec<SourceCandidate>, bool), CodeIndexError> {
    let mut candidates = Vec::new();
    let mut stack = vec![workspace_root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| CodeIndexError::new("repository_read_failed", error.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                CodeIndexError::new("repository_read_failed", error.to_string())
            })?;
            let file_type = entry.file_type().map_err(|error| {
                CodeIndexError::new("repository_read_failed", error.to_string())
            })?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                if !excluded_directory(entry.file_name().to_string_lossy().as_ref()) {
                    stack.push(path);
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Some(language) = source_language(&path) else {
                continue;
            };
            let metadata = entry.metadata().map_err(|error| {
                CodeIndexError::new("repository_read_failed", error.to_string())
            })?;
            let relative_path = relative_workspace_path(workspace_root, &path)?;
            candidates.push(SourceCandidate {
                path,
                relative_path,
                language,
                size_bytes: metadata.len(),
                modified_ns: modified_ns(&metadata),
            });
            if candidates.len() > max_files {
                candidates.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
                candidates.truncate(max_files);
                return Ok((candidates, true));
            }
        }
    }
    candidates.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok((candidates, false))
}

fn excluded_directory(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".rustclaw"
            | "target"
            | "node_modules"
            | "dist"
            | "build"
            | "vendor"
            | "__pycache__"
    )
}

fn source_language(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" | "jsx" => Some("javascript"),
        "ts" | "tsx" => Some("typescript"),
        "go" => Some("go"),
        "java" => Some("java"),
        "c" | "h" => Some("c"),
        "cc" | "cpp" | "cxx" | "hpp" => Some("cpp"),
        _ => None,
    }
}

fn index_source_file(candidate: &SourceCandidate) -> Result<IndexedFile, CodeIndexError> {
    let bytes = fs::read(&candidate.path)
        .map_err(|error| CodeIndexError::new("source_read_failed", error.to_string()))?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let mut indexed = IndexedFile {
        language: candidate.language.to_string(),
        size_bytes: candidate.size_bytes,
        modified_ns: candidate.modified_ns,
        sha256,
        parse_status: "file_only".to_string(),
        symbols: Vec::new(),
        references: Vec::new(),
    };
    if candidate.language != "rust" || candidate.size_bytes > MAX_SOURCE_BYTES {
        return Ok(indexed);
    }
    let Ok(source) = std::str::from_utf8(&bytes) else {
        indexed.parse_status = "invalid_utf8".to_string();
        return Ok(indexed);
    };
    let Ok(syntax) = syn::parse_file(source) else {
        indexed.parse_status = "parse_failed".to_string();
        return Ok(indexed);
    };
    let mut collector = RustSymbolCollector::default();
    collector.visit_file(&syntax);
    collector.finish();
    indexed.parse_status = "parsed".to_string();
    indexed.symbols = collector.symbols;
    indexed.references = collector.references;
    Ok(indexed)
}

#[derive(Default)]
struct RustSymbolCollector {
    scope: Vec<String>,
    symbols: Vec<SymbolDefinition>,
    references: Vec<SymbolReference>,
}

impl RustSymbolCollector {
    fn record_definition(
        &mut self,
        name: &str,
        kind: &str,
        span: Span,
        visibility: &syn::Visibility,
        is_test: bool,
    ) {
        let start = span.start().line.max(1);
        let end = span.end().line.max(start);
        let qualified_name = if self.scope.is_empty() {
            name.to_string()
        } else {
            format!("{}::{name}", self.scope.join("::"))
        };
        self.symbols.push(SymbolDefinition {
            name: name.to_string(),
            qualified_name,
            kind: kind.to_string(),
            line: start,
            end_line: end,
            visibility: visibility_token(visibility).to_string(),
            is_test,
        });
    }

    fn record_reference(&mut self, name: &str, span: Span, kind: &str) {
        if name.is_empty() {
            return;
        }
        self.references.push(SymbolReference {
            name: name.to_string(),
            line: span.start().line.max(1),
            kind: kind.to_string(),
        });
    }

    fn finish(&mut self) {
        self.symbols.sort_by(|left, right| {
            left.line
                .cmp(&right.line)
                .then_with(|| left.qualified_name.cmp(&right.qualified_name))
        });
        self.symbols.dedup();
        let definitions = self
            .symbols
            .iter()
            .map(|symbol| (symbol.name.clone(), symbol.line))
            .collect::<BTreeSet<_>>();
        self.references
            .retain(|reference| !definitions.contains(&(reference.name.clone(), reference.line)));
        self.references.sort();
        self.references.dedup();
    }

    fn record_token_stream_references(&mut self, stream: &TokenStream) {
        for token in stream.clone() {
            match token {
                TokenTree::Ident(ident) => {
                    self.record_reference(&ident.to_string(), ident.span(), "macro_token")
                }
                TokenTree::Group(group) => self.record_token_stream_references(&group.stream()),
                TokenTree::Punct(_) | TokenTree::Literal(_) => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for RustSymbolCollector {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        let name = item.sig.ident.to_string();
        self.record_definition(
            &name,
            "function",
            item.span(),
            &item.vis,
            has_test_attribute(&item.attrs),
        );
        self.scope.push(name);
        visit::visit_item_fn(self, item);
        self.scope.pop();
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let name = item.ident.to_string();
        self.record_definition(&name, "module", item.span(), &item.vis, false);
        self.scope.push(name);
        visit::visit_item_mod(self, item);
        self.scope.pop();
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        self.record_definition(
            &item.ident.to_string(),
            "struct",
            item.span(),
            &item.vis,
            false,
        );
        visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.record_definition(
            &item.ident.to_string(),
            "enum",
            item.span(),
            &item.vis,
            false,
        );
        visit::visit_item_enum(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        let name = item.ident.to_string();
        self.record_definition(&name, "trait", item.span(), &item.vis, false);
        self.scope.push(name);
        visit::visit_item_trait(self, item);
        self.scope.pop();
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let container = type_name(&item.self_ty).unwrap_or_else(|| "impl".to_string());
        self.scope.push(container);
        visit::visit_item_impl(self, item);
        self.scope.pop();
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        let name = item.sig.ident.to_string();
        self.record_definition(
            &name,
            "method",
            item.span(),
            &item.vis,
            has_test_attribute(&item.attrs),
        );
        self.scope.push(name);
        visit::visit_impl_item_fn(self, item);
        self.scope.pop();
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        let inherited = syn::Visibility::Inherited;
        self.record_definition(
            &item.sig.ident.to_string(),
            "trait_method",
            item.span(),
            &inherited,
            false,
        );
        visit::visit_trait_item_fn(self, item);
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        self.record_definition(
            &item.ident.to_string(),
            "const",
            item.span(),
            &item.vis,
            false,
        );
        visit::visit_item_const(self, item);
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        self.record_definition(
            &item.ident.to_string(),
            "static",
            item.span(),
            &item.vis,
            false,
        );
        visit::visit_item_static(self, item);
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.record_definition(
            &item.ident.to_string(),
            "type",
            item.span(),
            &item.vis,
            false,
        );
        visit::visit_item_type(self, item);
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if let Some(ident) = item.ident.as_ref() {
            let inherited = syn::Visibility::Inherited;
            self.record_definition(&ident.to_string(), "macro", item.span(), &inherited, false);
        }
        visit::visit_item_macro(self, item);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        for segment in &path.segments {
            self.record_reference(&segment.ident.to_string(), segment.ident.span(), "path");
        }
        visit::visit_path(self, path);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.record_reference(&call.method.to_string(), call.method.span(), "method_call");
        visit::visit_expr_method_call(self, call);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.record_token_stream_references(&mac.tokens);
        visit::visit_macro(self, mac);
    }

    fn visit_use_tree(&mut self, tree: &'ast syn::UseTree) {
        match tree {
            syn::UseTree::Path(path) => {
                self.record_reference(&path.ident.to_string(), path.ident.span(), "use")
            }
            syn::UseTree::Name(name) => {
                self.record_reference(&name.ident.to_string(), name.ident.span(), "use")
            }
            syn::UseTree::Rename(rename) => {
                self.record_reference(&rename.ident.to_string(), rename.ident.span(), "use")
            }
            syn::UseTree::Glob(_) | syn::UseTree::Group(_) => {}
        }
        visit::visit_use_tree(self, tree);
    }
}

fn has_test_attribute(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute
            .path()
            .segments
            .last()
            .map(|segment| segment.ident == "test")
            .unwrap_or(false)
    })
}

fn type_name(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn visibility_token(visibility: &syn::Visibility) -> &'static str {
    match visibility {
        syn::Visibility::Public(_) => "public",
        syn::Visibility::Restricted(_) => "restricted",
        syn::Visibility::Inherited => "private",
    }
}

fn search_symbols(
    index: &RepositoryIndex,
    args: &JsonMap<String, Value>,
    refresh: &RefreshStats,
) -> Result<Value, CodeIndexError> {
    let query = required_machine_string(args, "query")?;
    let mode = optional_machine_string(args, "mode").unwrap_or("exact");
    validate_search_mode(mode)?;
    let matches = all_definitions(index)
        .filter(|(_, symbol)| symbol_matches(&symbol.name, query, mode))
        .map(|(path, symbol)| definition_value(path, symbol))
        .collect::<Vec<_>>();
    let (matches, page) = paginate_values(matches, args)?;
    Ok(query_result(
        "search_symbols",
        index,
        refresh,
        json!({"query": query, "mode": mode, "definitions": matches, "page": page}),
    ))
}

fn find_definitions(
    index: &RepositoryIndex,
    args: &JsonMap<String, Value>,
    refresh: &RefreshStats,
) -> Result<Value, CodeIndexError> {
    let symbol = required_machine_string(args, "symbol")?;
    let definitions = all_definitions(index)
        .filter(|(_, definition)| definition.name == symbol || definition.qualified_name == symbol)
        .map(|(path, definition)| definition_value(path, definition))
        .collect::<Vec<_>>();
    let (definitions, page) = paginate_values(definitions, args)?;
    Ok(query_result(
        "find_definitions",
        index,
        refresh,
        json!({"symbol": symbol, "definitions": definitions, "page": page}),
    ))
}

fn find_references(
    index: &RepositoryIndex,
    args: &JsonMap<String, Value>,
    refresh: &RefreshStats,
) -> Result<Value, CodeIndexError> {
    let symbol = required_machine_string(args, "symbol")?;
    let references = all_references(index)
        .filter(|(_, reference)| reference.name == symbol)
        .map(|(path, reference)| reference_value(path, reference))
        .collect::<Vec<_>>();
    let (references, page) = paginate_values(references, args)?;
    Ok(query_result(
        "find_references",
        index,
        refresh,
        json!({"symbol": symbol, "references": references, "page": page}),
    ))
}

fn list_tests(
    index: &RepositoryIndex,
    args: &JsonMap<String, Value>,
    refresh: &RefreshStats,
) -> Result<Value, CodeIndexError> {
    let path_filter = optional_machine_string(args, "path")
        .map(normalize_relative_path)
        .transpose()?;
    let symbol_filter = optional_machine_string(args, "symbol");
    let referenced_paths = symbol_filter.map(|symbol| {
        all_references(index)
            .filter(|(_, reference)| reference.name == symbol)
            .map(|(path, _)| path.to_string())
            .collect::<BTreeSet<_>>()
    });
    let tests = all_definitions(index)
        .filter(|(path, definition)| {
            definition.is_test
                && path_filter
                    .as_ref()
                    .map(|filter| path.starts_with(filter))
                    .unwrap_or(true)
                && referenced_paths
                    .as_ref()
                    .map(|paths| paths.contains(*path))
                    .unwrap_or(true)
        })
        .map(|(path, definition)| definition_value(path, definition))
        .collect::<Vec<_>>();
    let (tests, page) = paginate_values(tests, args)?;
    Ok(query_result(
        "list_tests",
        index,
        refresh,
        json!({"path": path_filter, "symbol": symbol_filter, "tests": tests, "page": page}),
    ))
}

fn changed_impact(
    workspace_root: &Path,
    index: &RepositoryIndex,
    args: &JsonMap<String, Value>,
    refresh: &RefreshStats,
) -> Result<Value, CodeIndexError> {
    let changed_paths =
        machine_paths(args.get("paths"))?.unwrap_or_else(|| git_changed_paths(workspace_root));
    let changed = changed_paths.iter().cloned().collect::<BTreeSet<_>>();
    let changed_symbols = all_definitions(index)
        .filter(|(path, _)| changed.contains(*path))
        .map(|(_, definition)| definition.name.clone())
        .collect::<BTreeSet<_>>();
    let all_dependent_files = all_references(index)
        .filter(|(path, reference)| {
            !changed.contains(*path) && changed_symbols.contains(&reference.name)
        })
        .map(|(path, _)| path.to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let impacted_paths = changed
        .iter()
        .cloned()
        .chain(all_dependent_files.iter().cloned())
        .collect::<BTreeSet<_>>();
    let impacted_tests = all_definitions(index)
        .filter(|(path, definition)| definition.is_test && impacted_paths.contains(*path))
        .map(|(path, definition)| definition_value(path, definition))
        .collect::<Vec<_>>();
    let (dependent_files, dependent_files_page) = paginate_strings(all_dependent_files, args)?;
    let (impacted_tests, impacted_tests_page) = paginate_values(impacted_tests, args)?;
    Ok(query_result(
        "changed_impact",
        index,
        refresh,
        json!({
            "changed_paths": changed_paths,
            "changed_symbols": changed_symbols,
            "dependent_files": dependent_files,
            "impacted_tests": impacted_tests,
            "page": dependent_files_page,
            "pages": {
                "dependent_files": dependent_files_page,
                "impacted_tests": impacted_tests_page,
            },
        }),
    ))
}

fn retrieve_context(
    workspace_root: &Path,
    index: &RepositoryIndex,
    args: &JsonMap<String, Value>,
    refresh: &RefreshStats,
) -> Result<Value, CodeIndexError> {
    let symbols = machine_strings(args.get("symbols"))?.unwrap_or_default();
    let paths = machine_paths(args.get("paths"))?.unwrap_or_default();
    if symbols.is_empty() && paths.is_empty() {
        return Err(CodeIndexError::new(
            "missing_required",
            "code_index.retrieve_context_requires_symbols_or_paths",
        ));
    }
    let mode = optional_machine_string(args, "mode").unwrap_or("exact");
    validate_search_mode(mode)?;
    let context_lines = bounded_usize(
        args.get("context_lines"),
        DEFAULT_CONTEXT_LINES,
        0,
        MAX_CONTEXT_LINES,
    )?;
    let path_set = paths.iter().cloned().collect::<BTreeSet<_>>();
    let selected = all_definitions(index)
        .filter(|(path, definition)| {
            (!symbols.is_empty()
                && symbols.iter().any(|symbol| {
                    symbol_matches(&definition.name, symbol, mode)
                        || symbol_matches(&definition.qualified_name, symbol, mode)
                }))
                || (!path_set.is_empty() && path_set.contains(*path))
        })
        .map(|(path, definition)| (path.to_string(), definition.clone()))
        .collect::<Vec<_>>();
    let (selected, page) = paginate_pairs(selected, args)?;
    let mut snippets = Vec::new();
    for (path, definition) in selected {
        snippets.push(context_snippet(
            workspace_root,
            &path,
            &definition,
            context_lines,
        )?);
    }
    let selected_paths = snippets
        .iter()
        .filter_map(|snippet| snippet.get("path").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let related_tests = all_definitions(index)
        .filter(|(path, definition)| definition.is_test && selected_paths.contains(*path))
        .map(|(path, definition)| definition_value(path, definition))
        .collect::<Vec<_>>();
    let (related_tests, related_tests_page) = paginate_values(related_tests, args)?;
    Ok(query_result(
        "retrieve_context",
        index,
        refresh,
        json!({
            "symbols": symbols,
            "paths": paths,
            "mode": mode,
            "snippets": snippets,
            "related_tests": related_tests,
            "page": page,
            "related_tests_page": related_tests_page,
        }),
    ))
}

fn context_snippet(
    workspace_root: &Path,
    relative_path: &str,
    definition: &SymbolDefinition,
    context_lines: usize,
) -> Result<Value, CodeIndexError> {
    let start_line = definition.line.saturating_sub(context_lines).max(1);
    let end_line = definition.end_line.saturating_add(context_lines);
    let path = workspace_root.join(relative_path);
    let source = fs::read_to_string(&path)
        .map_err(|error| CodeIndexError::new("source_read_failed", error.to_string()))?;
    let content_sha256 = format!("sha256:{:x}", Sha256::digest(source.as_bytes()));
    let excerpt = source
        .lines()
        .skip(start_line.saturating_sub(1))
        .take(end_line.saturating_sub(start_line).saturating_add(1))
        .collect::<Vec<_>>()
        .join("\n");
    let excerpt = if excerpt.len() > MAX_SNIPPET_BYTES {
        excerpt[..safe_char_boundary(&excerpt, MAX_SNIPPET_BYTES)].to_string()
    } else {
        excerpt
    };
    Ok(json!({
        "path": relative_path,
        "symbol": definition.name,
        "qualified_name": definition.qualified_name,
        "kind": definition.kind,
        "definition_line": definition.line,
        "start_line": start_line,
        "end_line": end_line,
        "excerpt": excerpt,
        "content_sha256": content_sha256,
        "source": "refreshed_repository_index",
        "freshness": "refreshed_this_call",
        "range_handle": {
            "path": relative_path,
            "start_line": start_line,
            "end_line": end_line,
            "read_capability": "filesystem.read_text_range",
        },
    }))
}

fn query_result(
    action: &str,
    index: &RepositoryIndex,
    refresh: &RefreshStats,
    data: Value,
) -> Value {
    let page = data.get("page").cloned();
    let truncated = page
        .as_ref()
        .and_then(|page| page.get("has_more"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    json!({
        "schema_version": INDEX_SCHEMA_VERSION,
        "kind": "repository_code_index",
        "action": action,
        "status_code": "ok",
        "summary": index_summary(index, refresh),
        "page": page,
        "truncated": truncated,
        "provenance": {
            "source": "rustclaw_repository_index",
            "backend": "syn_ast_with_file_fallback",
            "index_ref": INDEX_RELATIVE_PATH,
            "generated_at": index.generated_at,
            "refreshed_at": refresh.refreshed_at,
            "scan_complete": index.scan_complete,
        },
        "data": data,
    })
}

fn index_summary(index: &RepositoryIndex, refresh: &RefreshStats) -> Value {
    let symbol_count = index
        .files
        .values()
        .map(|file| file.symbols.len())
        .sum::<usize>();
    let reference_count = index
        .files
        .values()
        .map(|file| file.references.len())
        .sum::<usize>();
    let test_count = index
        .files
        .values()
        .flat_map(|file| file.symbols.iter())
        .filter(|symbol| symbol.is_test)
        .count();
    let mut parse_status_counts = BTreeMap::<String, usize>::new();
    for file in index.files.values() {
        *parse_status_counts
            .entry(file.parse_status.clone())
            .or_default() += 1;
    }
    let fallback_file_count = index
        .files
        .values()
        .filter(|file| file.parse_status != "parsed")
        .count();
    json!({
        "file_count": index.files.len(),
        "symbol_count": symbol_count,
        "reference_count": reference_count,
        "test_count": test_count,
        "scanned_files": refresh.scanned_files,
        "parsed_files": refresh.parsed_files,
        "reused_files": refresh.reused_files,
        "removed_files": refresh.removed_files,
        "skipped_files": refresh.skipped_files,
        "scan_truncated": refresh.scan_truncated,
        "scan_complete": index.scan_complete,
        "generated_at": index.generated_at,
        "refreshed_at": refresh.refreshed_at,
        "index_source": "rustclaw_repository_index",
        "index_backend": "syn_ast_with_file_fallback",
        "fallback_file_count": fallback_file_count,
        "parse_status_counts": parse_status_counts,
        "index_ref": INDEX_RELATIVE_PATH,
    })
}

fn all_definitions(index: &RepositoryIndex) -> impl Iterator<Item = (&str, &SymbolDefinition)> {
    index.files.iter().flat_map(|(path, file)| {
        file.symbols
            .iter()
            .map(move |symbol| (path.as_str(), symbol))
    })
}

fn all_references(index: &RepositoryIndex) -> impl Iterator<Item = (&str, &SymbolReference)> {
    index.files.iter().flat_map(|(path, file)| {
        file.references
            .iter()
            .map(move |reference| (path.as_str(), reference))
    })
}

fn definition_value(path: &str, definition: &SymbolDefinition) -> Value {
    json!({
        "path": path,
        "name": definition.name,
        "qualified_name": definition.qualified_name,
        "kind": definition.kind,
        "line": definition.line,
        "end_line": definition.end_line,
        "visibility": definition.visibility,
        "is_test": definition.is_test,
        "source": "syn_ast",
        "freshness": "refreshed_this_call",
        "range_handle": {
            "path": path,
            "start_line": definition.line,
            "end_line": definition.end_line,
            "read_capability": "filesystem.read_text_range",
        },
    })
}

fn reference_value(path: &str, reference: &SymbolReference) -> Value {
    json!({
        "path": path,
        "name": reference.name,
        "line": reference.line,
        "kind": reference.kind,
        "source": "syn_ast",
        "freshness": "refreshed_this_call",
        "range_handle": {
            "path": path,
            "start_line": reference.line,
            "end_line": reference.line,
            "read_capability": "filesystem.read_text_range",
        },
    })
}

fn symbol_matches(value: &str, query: &str, mode: &str) -> bool {
    match mode {
        "exact" => value == query,
        "prefix" => value.starts_with(query),
        "contains" => value.contains(query),
        _ => false,
    }
}

fn validate_search_mode(mode: &str) -> Result<(), CodeIndexError> {
    if matches!(mode, "exact" | "prefix" | "contains") {
        Ok(())
    } else {
        Err(CodeIndexError::new(
            "invalid_search_mode",
            format!("code_index.invalid_search_mode mode={mode}"),
        ))
    }
}

fn result_limit(args: &JsonMap<String, Value>) -> Result<usize, CodeIndexError> {
    bounded_usize(
        args.get("max_results"),
        DEFAULT_MAX_RESULTS,
        1,
        HARD_MAX_RESULTS,
    )
}

fn paginate_values(
    values: Vec<Value>,
    args: &JsonMap<String, Value>,
) -> Result<(Vec<Value>, Value), CodeIndexError> {
    paginate_items(values, args)
}

fn paginate_strings(
    values: Vec<String>,
    args: &JsonMap<String, Value>,
) -> Result<(Vec<String>, Value), CodeIndexError> {
    paginate_items(values, args)
}

fn paginate_pairs(
    values: Vec<(String, SymbolDefinition)>,
    args: &JsonMap<String, Value>,
) -> Result<(Vec<(String, SymbolDefinition)>, Value), CodeIndexError> {
    paginate_items(values, args)
}

fn paginate_items<T>(
    values: Vec<T>,
    args: &JsonMap<String, Value>,
) -> Result<(Vec<T>, Value), CodeIndexError> {
    let limit = result_limit(args)?;
    let requested_cursor = args
        .get("cursor")
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| CodeIndexError::new("invalid_args", "code_index.cursor_invalid"))
        })
        .transpose()?
        .unwrap_or(0);
    let total_count = values.len();
    let start = requested_cursor.min(total_count);
    let end = start.saturating_add(limit).min(total_count);
    let returned = end.saturating_sub(start);
    let page_values = values
        .into_iter()
        .skip(start)
        .take(returned)
        .collect::<Vec<_>>();
    Ok((
        page_values,
        json!({
            "cursor": start,
            "requested_cursor": requested_cursor,
            "start_index": start,
            "end_index": end,
            "limit": limit,
            "returned_count": returned,
            "total_count": total_count,
            "has_more": end < total_count,
            "next_cursor": (end < total_count).then_some(end),
            "previous_cursor": (start > 0).then_some(start.saturating_sub(limit)),
        }),
    ))
}

fn bounded_usize(
    value: Option<&Value>,
    default: usize,
    minimum: usize,
    maximum: usize,
) -> Result<usize, CodeIndexError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let number = value
        .as_u64()
        .ok_or_else(|| CodeIndexError::new("invalid_args", "code_index.integer_required"))?;
    Ok((number as usize).clamp(minimum, maximum))
}

fn required_machine_string<'a>(
    args: &'a JsonMap<String, Value>,
    key: &str,
) -> Result<&'a str, CodeIndexError> {
    optional_machine_string(args, key).ok_or_else(|| {
        CodeIndexError::new(
            "missing_required",
            format!("code_index.missing_required key={key}"),
        )
    })
}

fn optional_machine_string<'a>(args: &'a JsonMap<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn machine_strings(value: Option<&Value>) -> Result<Option<Vec<String>>, CodeIndexError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let values = match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(values) => values
            .iter()
            .map(|value| {
                value.as_str().map(str::to_string).ok_or_else(|| {
                    CodeIndexError::new("invalid_args", "code_index.string_array_required")
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(CodeIndexError::new(
                "invalid_args",
                "code_index.string_or_array_required",
            ))
        }
    };
    let values = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    Ok(Some(values))
}

fn machine_paths(value: Option<&Value>) -> Result<Option<Vec<String>>, CodeIndexError> {
    let Some(values) = machine_strings(value)? else {
        return Ok(None);
    };
    values
        .into_iter()
        .map(|value| normalize_relative_path(&value))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn normalize_relative_path(value: &str) -> Result<String, CodeIndexError> {
    let path = Path::new(value.trim());
    if path.is_absolute() {
        return Err(CodeIndexError::new(
            "path_outside_workspace",
            "code_index.absolute_path_rejected",
        ));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(CodeIndexError::new(
                    "path_outside_workspace",
                    "code_index.path_traversal_rejected",
                ))
            }
        }
    }
    Ok(normalized.to_string_lossy().replace('\\', "/"))
}

fn relative_workspace_path(root: &Path, path: &Path) -> Result<String, CodeIndexError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        CodeIndexError::new(
            "path_outside_workspace",
            "code_index.path_outside_workspace",
        )
    })?;
    normalize_relative_path(&relative.to_string_lossy())
}

fn modified_ns(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

fn git_changed_paths(workspace_root: &Path) -> Vec<String> {
    let mut paths = BTreeSet::new();
    for args in [
        vec!["diff", "--name-only", "--relative", "HEAD"],
        vec!["ls-files", "--others", "--exclude-standard"],
    ] {
        let Ok(output) = Command::new("git")
            .args(args)
            .current_dir(workspace_root)
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Ok(path) = normalize_relative_path(line) {
                if !path.is_empty() {
                    paths.insert(path);
                }
            }
        }
    }
    paths.into_iter().collect()
}

fn safe_char_boundary(value: &str, limit: usize) -> usize {
    let mut end = limit.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
#[path = "builtin_code_index_tests.rs"]
mod tests;
