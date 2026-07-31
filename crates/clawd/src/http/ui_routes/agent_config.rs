const AGENT_CONFIG_SCHEMA_VERSION: u32 = 1;
const AGENT_CUSTOM_PERSONA_MAX_CHARS: usize = 1200;

static AGENT_CONFIG_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
struct AgentPersonaPresetItem {
    id: &'static str,
    name_key: &'static str,
    description_key: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct AgentPersonaConstraints {
    custom_persona_max_chars: usize,
    allowed_control_characters: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
struct AgentConfigViewItem {
    id: String,
    name: String,
    description: String,
    saved_profile: String,
    effective_profile: String,
    custom_persona: String,
    preferred_vendor: Option<String>,
    preferred_model: Option<String>,
    allowed_skills: Vec<String>,
    runtime_applied: bool,
}

#[derive(Debug, Clone, Serialize)]
struct AgentConfigResponse {
    schema_version: u32,
    config_path: String,
    editable: bool,
    applies_to: &'static str,
    notice_key: &'static str,
    agents: Vec<AgentConfigViewItem>,
    preset_catalog: Vec<AgentPersonaPresetItem>,
    constraints: AgentPersonaConstraints,
}

#[derive(Debug, Deserialize)]
struct UpdateAgentConfigRequest {
    agent_id: String,
    #[serde(default)]
    persona_profile: Option<String>,
    #[serde(default)]
    custom_persona: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CanonicalAgentsFile {
    schema_version: u32,
    agents: Vec<claw_core::config::AgentConfig>,
}

struct AgentConfigService<'a> {
    state: &'a AppState,
}

impl<'a> AgentConfigService<'a> {
    fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    fn base_config_path(&self) -> PathBuf {
        active_runtime_config_path(self.state)
    }

    fn agents_config_path(&self) -> anyhow::Result<PathBuf> {
        let config_path = self.base_config_path();
        let parent = config_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("runtime config path has no parent"))?;
        Ok(parent.join("agents.toml"))
    }

    fn load(&self) -> anyhow::Result<claw_core::config::AppConfig> {
        Ok(claw_core::config::AppConfig::load(
            &self.base_config_path().to_string_lossy(),
        )?)
    }

    fn response(&self, editable: bool) -> anyhow::Result<AgentConfigResponse> {
        let config = self.load()?;
        let runtime = self.state.agent_runtime_snapshot();
        let agents = config
            .normalized_agents()
            .into_iter()
            .map(|agent| {
                let applied = runtime.get(&agent.id);
                AgentConfigViewItem {
                    id: agent.id.clone(),
                    name: agent.name,
                    description: agent.description,
                    saved_profile: agent.persona_profile.clone(),
                    effective_profile: applied
                        .map(|item| item.persona_profile.clone())
                        .unwrap_or_else(|| "executor".to_string()),
                    custom_persona: agent.persona_fragment.clone(),
                    preferred_vendor: agent.preferred_vendor,
                    preferred_model: agent.preferred_model,
                    allowed_skills: agent.allowed_skills,
                    runtime_applied: applied.is_some_and(|item| {
                        item.configured_persona_profile == agent.persona_profile
                            && (agent.persona_profile != "custom"
                                || item.persona_fragment == agent.persona_fragment)
                    }),
                }
            })
            .collect();
        Ok(AgentConfigResponse {
            schema_version: AGENT_CONFIG_SCHEMA_VERSION,
            config_path: "configs/agents.toml".to_string(),
            editable,
            applies_to: "new_tasks",
            notice_key: "agent.persona.scope_notice",
            agents,
            preset_catalog: agent_persona_preset_catalog(),
            constraints: AgentPersonaConstraints {
                custom_persona_max_chars: AGENT_CUSTOM_PERSONA_MAX_CHARS,
                allowed_control_characters: vec!["tab", "newline"],
            },
        })
    }

    fn update(&self, req: &UpdateAgentConfigRequest) -> anyhow::Result<()> {
        let _write_guard = AGENT_CONFIG_WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| anyhow::anyhow!("agent_config_write_lock_poisoned"))?;
        let config = self.load()?;
        let mut agents = config.normalized_agents();
        let requested_agent_id = req.agent_id.trim();
        if requested_agent_id.is_empty() {
            anyhow::bail!("agent_id_required");
        }
        let agent = agents
            .iter_mut()
            .find(|agent| agent.id == requested_agent_id)
            .ok_or_else(|| anyhow::anyhow!("agent_not_found"))?;

        if let Some(profile) = req.persona_profile.as_deref() {
            let (profile, known) = claw_core::config::normalize_agent_persona_profile(profile);
            if !known {
                anyhow::bail!("unknown_persona_profile");
            }
            agent.persona_profile = profile.to_string();
        }
        if let Some(fragment) = req.custom_persona.as_deref() {
            validate_agent_custom_persona(fragment)?;
            agent.persona_fragment = fragment.trim().to_string();
        }
        validate_agent_custom_persona(&agent.persona_fragment)?;
        agent.persona_prompt.clear();

        let canonical = CanonicalAgentsFile {
            schema_version: AGENT_CONFIG_SCHEMA_VERSION,
            agents,
        };
        let raw = toml::to_string_pretty(&canonical)?;
        let path = self.agents_config_path()?;
        let original = fs::read(&path).ok();
        atomic_write_agent_config(&path, raw.as_bytes())?;

        let reloaded = match self.load() {
            Ok(config) => config,
            Err(error) => {
                if let Some(original) = original.as_deref() {
                    let _ = atomic_write_agent_config(&path, original);
                } else {
                    let _ = fs::remove_file(&path);
                }
                return Err(anyhow::anyhow!("agent_config_reread_failed: {error}"));
            }
        };
        let snapshot = crate::runtime::provider_runtime::build_agent_runtime_snapshot(&reloaded);
        if let Err(error) = self.state.replace_agent_runtime_snapshot(snapshot) {
            if let Some(original) = original.as_deref() {
                let _ = atomic_write_agent_config(&path, original);
            } else {
                let _ = fs::remove_file(&path);
            }
            return Err(anyhow::anyhow!("agent_runtime_swap_failed: {error}"));
        }

        if let Some(runtime) = self
            .state
            .agent_runtime_snapshot()
            .get(requested_agent_id)
        {
            tracing::info!(
                agent_id = %runtime.id,
                profile = %runtime.persona_profile,
                persona_chars = runtime.persona_fragment.chars().count(),
                persona_digest = %runtime.persona_digest,
                result = "applied",
                "agent_config_update"
            );
        }
        Ok(())
    }
}

fn validate_agent_custom_persona(fragment: &str) -> anyhow::Result<()> {
    if fragment.chars().count() > AGENT_CUSTOM_PERSONA_MAX_CHARS {
        anyhow::bail!("custom_persona_too_long");
    }
    if fragment
        .chars()
        .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        anyhow::bail!("custom_persona_control_character_forbidden");
    }
    Ok(())
}

fn atomic_write_agent_config(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "agents config has no parent")
    })?;
    fs::create_dir_all(parent)?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".agents.toml.{}.{}.tmp",
        std::process::id(),
        suffix
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&temporary, metadata.permissions())?;
        }
        fs::rename(&temporary, path)?;
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn agent_persona_preset_catalog() -> Vec<AgentPersonaPresetItem> {
    vec![
        AgentPersonaPresetItem {
            id: "inherit",
            name_key: "agent.persona.inherit.name",
            description_key: "agent.persona.inherit.description",
        },
        AgentPersonaPresetItem {
            id: "executor",
            name_key: "agent.persona.executor.name",
            description_key: "agent.persona.executor.description",
        },
        AgentPersonaPresetItem {
            id: "companion",
            name_key: "agent.persona.companion.name",
            description_key: "agent.persona.companion.description",
        },
        AgentPersonaPresetItem {
            id: "expert",
            name_key: "agent.persona.expert.name",
            description_key: "agent.persona.expert.description",
        },
        AgentPersonaPresetItem {
            id: "teacher",
            name_key: "agent.persona.teacher.name",
            description_key: "agent.persona.teacher.description",
        },
        AgentPersonaPresetItem {
            id: "advisor",
            name_key: "agent.persona.advisor.name",
            description_key: "agent.persona.advisor.description",
        },
        AgentPersonaPresetItem {
            id: "reviewer",
            name_key: "agent.persona.reviewer.name",
            description_key: "agent.persona.reviewer.description",
        },
        AgentPersonaPresetItem {
            id: "custom",
            name_key: "agent.persona.custom.name",
            description_key: "agent.persona.custom.description",
        },
    ]
}

async fn get_agents_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<AgentConfigResponse>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(response))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: response.ok,
                    data: None,
                    error: response.error,
                }),
            );
        }
    };
    match AgentConfigService::new(&state).response(identity.role.eq_ignore_ascii_case("admin")) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read agent config failed: {error}")),
            }),
        ),
    }
}

async fn update_agents_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateAgentConfigRequest>,
) -> (StatusCode, Json<ApiResponse<AgentConfigResponse>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(response))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: response.ok,
                    data: None,
                    error: response.error,
                }),
            );
        }
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("admin_required".to_string()),
            }),
        );
    }
    let service = AgentConfigService::new(&state);
    if let Err(error) = service.update(&req) {
        let message = error.to_string();
        let status = if message.contains("not_found") {
            StatusCode::NOT_FOUND
        } else if message.contains("required")
            || message.contains("unknown_persona")
            || message.contains("too_long")
            || message.contains("control_character")
        {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        return (
            status,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(message),
            }),
        );
    }
    match service.response(true) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("agent config saved but response failed: {error}")),
            }),
        ),
    }
}
