use super::*;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvRestore {
    name: &'static str,
    previous: Option<String>,
}

impl EnvRestore {
    fn capture(name: &'static str) -> Self {
        Self {
            name,
            previous: std::env::var(name).ok(),
        }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}

fn temp_workspace_root() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("agent-runtime-model-catalog-{unique}"));
    std::fs::create_dir_all(root.join("configs")).expect("create configs");
    root
}

fn write_fixture(root: &Path) {
    std::fs::write(
        root.join("configs/config.toml"),
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M3"

[llm.minimax]
base_url = "https://api.minimaxi.com/v1"
api_key = "secret-minimax"
model = "MiniMax-M3"
models = ["MiniMax-M3", "MiniMax-M2.7"]
input_modalities = ["text", "image", "video"]
output_modalities = ["text"]
context_window_tokens = 1000000
timeout_seconds = 180

[llm.qwen]
base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1"
api_key = "secret-qwen"
model = "qwen-max-latest"
models = ["qwen-max-latest"]
input_modalities = ["text"]
output_modalities = ["text"]
timeout_seconds = 60

[llm.fixture_missing]
base_url = "https://fixture.invalid/v1"
api_key = ""
model = "fixture-model"
models = ["fixture-model"]
input_modalities = ["text"]
output_modalities = ["text"]
timeout_seconds = 30

[llm.mimo]
base_url = "https://token-plan-cn.xiaomimimo.com/v1"
api_key = ""
model = "mimo-v2.5-pro"
models = ["mimo-v2.5-pro"]
input_modalities = ["text"]
output_modalities = ["text"]
timeout_seconds = 180
"#,
    )
    .expect("write config");
    std::fs::write(
        root.join("configs/image.toml"),
        r#"
[image_vision]
default_vendor = "minimax"
default_model = "MiniMax-M3"
minimax_models = ["MiniMax-M3"]
qwen_models = ["qwen-vl-max"]

[image_generation]
default_vendor = "minimax"
default_model = "image-01"
minimax_models = ["image-01"]
qwen_models = ["wanx2.1-t2i-turbo"]

[image_edit]
default_vendor = "minimax"
default_model = "image-01"
minimax_models = ["image-01"]

[image_generation.providers.minimax]
base_url = "https://api.minimaxi.com/v1"
model = "image-01"
"#,
    )
    .expect("write image");
    std::fs::write(
        root.join("configs/audio.toml"),
        r#"
[audio_transcribe]
default_vendor = "custom"
default_model = "local-whisper"
custom_models = ["local-whisper"]
qwen_models = ["qwen3-asr-flash"]
minimax_models = []

[audio_transcribe.providers.custom]
base_url = "http://127.0.0.1:8178/v1"
model = "local-whisper"

[audio_synthesize]
default_vendor = "minimax"
default_model = "speech-2.8-turbo"
minimax_models = ["speech-2.8-turbo"]
qwen_models = ["qwen3-tts-flash"]
"#,
    )
    .expect("write audio");
    std::fs::write(
        root.join("configs/video.toml"),
        r#"
[video_generation]
default_vendor = "minimax"
default_model = "MiniMax-Hailuo-2.3"
minimax_models = ["MiniMax-Hailuo-2.3"]
"#,
    )
    .expect("write video");
    std::fs::write(
        root.join("configs/music.toml"),
        r#"
[music_generation]
default_vendor = "minimax"
default_model = "music-2.6"
minimax_models = ["music-2.6"]
"#,
    )
    .expect("write music");
}

#[test]
fn catalog_separates_selected_model_inputs_from_media_skill_support() {
    let root = temp_workspace_root();
    write_fixture(&root);

    let catalog = build_model_catalog_from_workspace(&root).expect("catalog");
    let minimax = catalog
        .entries
        .iter()
        .find(|entry| entry.provider == "minimax" && entry.model == "MiniMax-M3")
        .expect("minimax entry");

    assert_eq!(catalog.selected_provider, "minimax");
    assert_eq!(catalog.selected_model, "MiniMax-M3");
    assert!(minimax.active_text_provider);
    assert_eq!(
        minimax.input_modalities,
        vec!["text".to_string(), "image".to_string(), "video".to_string()]
    );
    assert_eq!(minimax.output_modalities, vec!["text".to_string()]);
    assert!(minimax.supports_image_input);
    assert!(minimax.supports_video_input);
    assert!(!minimax.supports_audio_input);
    assert!(minimax.supports_image_understanding);
    assert!(!minimax.supports_audio_transcription);
    assert!(!minimax.supports_image_generation);
    assert!(!minimax.supports_image_edit);
    assert!(!minimax.supports_audio_generation);
    assert!(!minimax.supports_video_generation);
    assert!(!minimax.supports_music_generation);
    assert!(!minimax.async_required);
    assert_eq!(minimax.credential_state, "configured_inline");
    assert_eq!(minimax.context_window_tokens, Some(1_000_000));
    assert_eq!(
        minimax.base_url_kind,
        "minimax_official_openai_compat".to_string()
    );
}

#[test]
fn catalog_emits_distinct_entries_for_independent_multimodal_models() {
    let root = temp_workspace_root();
    write_fixture(&root);

    let catalog = build_model_catalog_from_workspace(&root).expect("catalog");
    let find = |provider: &str, model: &str| {
        catalog
            .entries
            .iter()
            .find(|entry| entry.provider == provider && entry.model == model)
            .unwrap_or_else(|| panic!("missing {provider}:{model}"))
    };
    let image = find("minimax", "image-01");
    let speech = find("minimax", "speech-2.8-turbo");
    let video = find("minimax", "MiniMax-Hailuo-2.3");
    let music = find("minimax", "music-2.6");
    let whisper = find("custom", "local-whisper");

    assert!(image.supports_image_generation);
    assert!(image.supports_image_edit);
    assert!(!image.supports_text);
    assert!(speech.supports_audio_generation);
    assert!(video.supports_video_generation);
    assert!(music.supports_music_generation);
    assert!(whisper.supports_audio_transcription);
    assert_eq!(whisper.credential_state, "not_required_local");
    assert_eq!(whisper.base_url_kind, "local_loopback");
}

#[test]
fn catalog_reports_missing_credential_state_without_secret_values() {
    let root = temp_workspace_root();
    write_fixture(&root);

    let catalog = build_model_catalog_from_workspace(&root).expect("catalog");
    let missing = catalog
        .entries
        .iter()
        .find(|entry| entry.provider == "fixture_missing")
        .expect("fixture missing entry");

    assert_eq!(missing.credential_state, "missing");
    assert!(missing.supports_text);
}

#[test]
fn catalog_reports_env_credential_state_without_secret_values() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let _restore_mimo = EnvRestore::capture("MIMO_API_KEY");
    let _restore_xiaomi = EnvRestore::capture("XIAOMI_API_KEY");
    unsafe {
        std::env::set_var("MIMO_API_KEY", "secret-env-mimo");
        std::env::remove_var("XIAOMI_API_KEY");
    }

    let root = temp_workspace_root();
    write_fixture(&root);
    let catalog = build_model_catalog_from_workspace(&root).expect("catalog");
    let mimo = catalog
        .entries
        .iter()
        .find(|entry| entry.provider == "mimo")
        .expect("mimo entry");
    let serialized = serde_json::to_string(&catalog).expect("json");

    assert_eq!(mimo.credential_state, "configured_env");
    assert!(!serialized.contains("secret-env-mimo"));
}

#[test]
fn catalog_reports_env_file_credential_state_without_secret_values() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let _restore_catalog_env = EnvRestore::capture("CHINESE_PROVIDER_ENV_FILE");
    let _restore_mimo = EnvRestore::capture("MIMO_API_KEY");
    let _restore_xiaomi = EnvRestore::capture("XIAOMI_API_KEY");
    unsafe {
        std::env::remove_var("MIMO_API_KEY");
        std::env::remove_var("XIAOMI_API_KEY");
    }

    let root = temp_workspace_root();
    write_fixture(&root);
    let env_file = root.join("runtime_env_filled.sh");
    std::fs::write(
        &env_file,
        r#"
# comment
export MIMO_API_KEY='secret-env-file-mimo'
"#,
    )
    .expect("write env file");
    unsafe {
        std::env::set_var("CHINESE_PROVIDER_ENV_FILE", env_file.as_os_str());
    }

    let catalog = build_model_catalog_from_workspace(&root).expect("catalog");
    let mimo = catalog
        .entries
        .iter()
        .find(|entry| entry.provider == "mimo")
        .expect("mimo entry");
    let serialized = serde_json::to_string(&catalog).expect("json");

    assert_eq!(mimo.credential_state, "configured_env");
    assert!(!serialized.contains("secret-env-file-mimo"));
    assert!(!serialized.contains("runtime_env_filled.sh"));
}

#[test]
fn catalog_ignores_missing_env_file_path() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let _restore_catalog_env = EnvRestore::capture("CHINESE_PROVIDER_ENV_FILE");
    unsafe {
        std::env::set_var(
            "CHINESE_PROVIDER_ENV_FILE",
            "/tmp/agent-runtime-definitely-missing-env-file.sh",
        );
    }

    let root = temp_workspace_root();
    write_fixture(&root);
    let catalog = build_model_catalog_from_workspace(&root).expect("catalog");

    assert!(catalog
        .entries
        .iter()
        .any(|entry| entry.provider == "fixture_missing" && entry.credential_state == "missing"));
}

#[test]
fn catalog_output_does_not_serialize_secret_values() {
    let root = temp_workspace_root();
    write_fixture(&root);

    let catalog = build_model_catalog_from_workspace(&root).expect("catalog");
    let serialized = serde_json::to_string(&catalog).expect("json");

    assert!(!serialized.contains("secret-minimax"));
    assert!(!serialized.contains("secret-qwen"));
    assert!(serialized.contains("configured_inline"));
    assert!(serialized.contains("qwen_dashscope_openai_compat"));
}
