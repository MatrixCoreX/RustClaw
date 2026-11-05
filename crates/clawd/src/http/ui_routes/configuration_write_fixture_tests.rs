use super::*;

pub(super) const REPLACEMENT: &str = "fixture-shared-settings-replacement-91357";
pub(super) const SELECTED_SECRET: &str = "fixture-selected-settings-secret-82468";

pub(super) fn custom_fixture() -> Fixture {
    let fixture = Fixture::new();
    let raw = fs::read_to_string(fixture.root.join("configs/config.toml")).unwrap();
    fixture.write(
        "config.toml",
        &raw.replace("selected_vendor='fixture'", "selected_vendor='custom'")
            .replace("[llm.fixture]", "[llm.custom]"),
    );
    fixture
}

pub(super) fn selected_fixture() -> (Fixture, PathBuf) {
    let mut fixture = custom_fixture();
    let directory = fixture.root.join("selected");
    fs::create_dir(&directory).unwrap();
    for name in ["image.toml", "audio.toml", "video.toml", "music.toml"] {
        fs::copy(
            fixture.root.join("configs").join(name),
            directory.join(name),
        )
        .unwrap();
    }
    let main = directory.join("runtime.toml");
    let raw = fs::read_to_string(fixture.root.join("configs/config.toml")).unwrap();
    fs::write(
        &main,
        raw.replace(
            "selected_model='fixture-model'",
            "selected_model='selected-model'",
        )
        .replace(SECRET, SELECTED_SECRET),
    )
    .unwrap();
    let image = directory.join("image.toml");
    fs::write(
        &image,
        fs::read_to_string(&image)
            .unwrap()
            .replace(
                "default_model='fixture-model'",
                "default_model='selected-image'",
            )
            .replace(SECRET, SELECTED_SECRET),
    )
    .unwrap();
    fixture.state.reload_ctx.config_path_for_reload = "selected/runtime.toml".to_string();
    (fixture, main)
}

pub(super) fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        ADMIN.parse().unwrap(),
    );
    headers
}

pub(super) fn legacy_request(key: Option<&str>) -> UpdateLlmConfigRequest {
    UpdateLlmConfigRequest {
        selected_vendor: "custom".to_string(),
        selected_model: "legacy-selected".to_string(),
        vendor_base_url: Some("https://fixture.invalid/v1".to_string()),
        vendor_api_key: key.map(str::to_string),
        vendor_api_format: Some("openai_compat".to_string()),
    }
}

pub(super) fn edit(fixture: &Fixture, name: &'static str) -> ModelSettingsEdit {
    let original = fs::read(fixture.root.join("configs").join(name)).ok();
    let mut updated = original.clone().unwrap_or_default();
    updated.extend_from_slice(b"\n# owned configuration publication\n");
    ModelSettingsEdit {
        name,
        original,
        updated,
    }
}

pub(super) struct ReleaseGate(pub Option<std::sync::mpsc::Sender<()>>);
impl ReleaseGate {
    pub fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        self.release();
    }
}

pub(super) struct NoopWake;
impl std::task::Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}
