use super::*;

const SETTINGS: &str = r#"
config_path = "embedded-path-must-not-replace-the-selected-file.toml"
long_term_enabled = false
hybrid_recall_enabled = false
reindex_on_startup = false
recall_limit = 17
prompt_recall_limit = 5
background_job_concurrency = 3
"#;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(raw: Option<&str>) -> Self {
        let root = std::env::temp_dir().join(format!(
            "agent-memory-shape-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&root).expect("create unique fixture");
        if let Some(raw) = raw {
            std::fs::write(root.join("memory.toml"), raw).expect("write fixture");
        }
        Self { root }
    }

    fn source(&self) -> MemoryConfig {
        MemoryConfig {
            config_path: "memory.toml".into(),
            long_term_enabled: true,
            hybrid_recall_enabled: true,
            reindex_on_startup: true,
            recall_limit: 41,
            prompt_recall_limit: 31,
            background_job_concurrency: 5,
            ..MemoryConfig::default()
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn assert_selected_values(actual: &MemoryConfig, path: &str) {
    assert!(!actual.long_term_enabled);
    assert!(!actual.hybrid_recall_enabled);
    assert!(!actual.reindex_on_startup);
    assert_eq!(actual.recall_limit, 17);
    assert_eq!(actual.prompt_recall_limit, 5);
    assert_eq!(actual.background_job_concurrency, 3);
    assert_eq!(actual.config_path, path);
}

fn assert_main_values(actual: &MemoryConfig, expected: &MemoryConfig) {
    assert_eq!(actual.long_term_enabled, expected.long_term_enabled);
    assert_eq!(actual.hybrid_recall_enabled, expected.hybrid_recall_enabled);
    assert_eq!(actual.reindex_on_startup, expected.reindex_on_startup);
    assert_eq!(actual.recall_limit, expected.recall_limit);
    assert_eq!(actual.prompt_recall_limit, expected.prompt_recall_limit);
    assert_eq!(
        actual.background_job_concurrency,
        expected.background_job_concurrency
    );
    assert_eq!(actual.config_path, expected.config_path);
}

#[test]
fn flat_explicit_false_and_nondefault_values_are_preserved() {
    let fixture = Fixture::new(Some(SETTINGS));
    let loaded = load_memory_runtime_config(&fixture.root, &fixture.source());
    assert_selected_values(&loaded, "memory.toml");
}

#[test]
fn wrapped_explicit_false_and_nondefault_values_are_not_replaced_by_defaults() {
    let fixture = Fixture::new(Some(&format!("[memory]\n{SETTINGS}")));
    let loaded = load_memory_runtime_config(&fixture.root, &fixture.source());
    assert_selected_values(&loaded, "memory.toml");
}

#[test]
fn flat_and_wrapped_forms_agree_and_keep_the_selected_absolute_path() {
    for raw in [SETTINGS.to_string(), format!("[memory]\n{SETTINGS}")] {
        let fixture = Fixture::new(Some(&raw));
        let mut source = fixture.source();
        source.config_path = fixture
            .root
            .join("memory.toml")
            .to_string_lossy()
            .into_owned();
        let loaded = load_memory_runtime_config(&fixture.root, &source);
        assert_selected_values(&loaded, &source.config_path);
    }
}

#[test]
fn explicit_memory_namespace_is_not_shadowed_by_flat_fields() {
    let raw = format!("hybrid_recall_enabled = true\nrecall_limit = 999\n[memory]\n{SETTINGS}");
    let fixture = Fixture::new(Some(&raw));
    let loaded = load_memory_runtime_config(&fixture.root, &fixture.source());
    assert_selected_values(&loaded, "memory.toml");
}

#[test]
fn malformed_wrapper_uses_explicit_main_values_not_flat_defaults() {
    for raw in [
        "[memory]\nhybrid_recall_enabled = 'not-a-boolean'",
        "[memory]\nrecall_limit = 'not-an-integer'",
        "memory = 17",
    ] {
        let fixture = Fixture::new(Some(raw));
        let mut source = fixture.source();
        source.long_term_enabled = false;
        source.hybrid_recall_enabled = false;
        let loaded = load_memory_runtime_config(&fixture.root, &source);
        assert_main_values(&loaded, &source);
    }
}

#[test]
fn missing_or_malformed_file_keeps_main_configuration() {
    for raw in [None, Some("[invalid")] {
        let fixture = Fixture::new(raw);
        let source = fixture.source();
        let loaded = load_memory_runtime_config(&fixture.root, &source);
        assert_main_values(&loaded, &source);
    }
}

#[test]
fn empty_path_keeps_main_configuration_without_reading_a_file() {
    let fixture = Fixture::new(None);
    let mut source = fixture.source();
    source.config_path = "   ".into();
    let loaded = load_memory_runtime_config(&fixture.root.join("absent"), &source);
    assert_main_values(&loaded, &source);
    assert_eq!(std::fs::read_dir(&fixture.root).unwrap().count(), 0);
}
