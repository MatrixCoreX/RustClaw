//! 技能注册表：从 TOML 驱动技能的发现、启用、别名、超时与 prompt 路径。
//! Phase 1：仅做“发现/启用/别名/超时”从 registry 读取，执行层与 planner 仍用现有逻辑。

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

/// 技能类型：builtin（clawd 内执行）/ runner（skill-runner 子进程）/ 预留 external
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SkillKind {
    Builtin,
    #[default]
    Runner,
    #[serde(other)]
    External,
}

/// 技能输出类型，用于后续 UI/通道展示与路由
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputKind {
    #[default]
    Text,
    File,
    Image,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillRiskLevel {
    #[default]
    Unknown,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrimaryFallbackRole {
    #[default]
    None,
    Primary,
    Fallback,
}

/// §P4.1 主体：技能能力声明（closed-set + 命名 secrets.*）。
///
/// **设计目标**
/// - operator 在 registry 加载阶段就能审视技能要的资源（CI 拦截漂移）。
/// - §P4.4 短期 secrets token：将凭 `secrets.<name>` 决定是否注入对应密钥。
/// - §P4.2 skill shape 决策：对外呼出的能力（net/exec）会推导出更严的隔离形态。
///
/// **词汇表**（任何不在表内的字符串都会让 registry 加载报错，避免 typo）：
/// - `llm`：调用 LLM 网关（受 clawd LLM gateway 管控：fallback / 限流 / 审计）。
/// - `net`：除 LLM 网关外的对外网络（HTTP/HTTPS/raw socket）。
/// - `fs.read`：读取技能 bundle 目录之外的文件。
/// - `fs.write`：在技能 bundle 目录之外创建/修改文件。
/// - `exec`：fork/exec 子进程（含 shell）。
/// - `exec.sudo`：以提权方式 fork（必须额外 opt-in，独立于 `exec`）。
/// - `secrets.<name>`：需要某个具名密钥。`<name>` 仅允许 `[a-z0-9_]`，长度
///   1..=64，避免拼写漂移。
///
/// **关于 `secrets.<name>` 的命名规范（重要 — 与 §P4.4 配合）**：
///
/// 本仓里 `image_generation` / `image_edit` / `image_vision` / `[llm]`（即文本
/// 对话与规划）是**四个互相独立**的 LLM provider 配置域 —— 同一个 vendor 可
/// 以在不同用途下填不同 base_url 与 api_key。所以 secret 命名必须按
/// `<用途>_<vendor>_api_key` 展开，**不能**用 `secrets.openai_api_key` 这种
/// 跨用途的"vendor-唯一"命名，否则会把 image_generate 的 key 误注入到
/// chat/规划链路（反之亦然）。
///
/// 推荐命名：
/// - `secrets.image_generation_minimax_api_key`
/// - `secrets.image_edit_qwen_api_key`
/// - `secrets.image_vision_openai_api_key`
/// - `secrets.text_openai_api_key`（对应 `[llm.openai].api_key`）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Capability {
    Llm,
    Net,
    FsRead,
    FsWrite,
    Exec,
    ExecSudo,
    /// 内部存的是密钥的 canonical name（小写、`[a-z0-9_]`）。
    Secrets(String),
}

impl Capability {
    /// 解析 TOML 中的字符串到 [`Capability`]。
    ///
    /// 收尾 §P4.1 的契约：未知 token 必须报错而不是默默忽略。
    pub fn parse(token: &str) -> Result<Self, String> {
        let raw = token.trim();
        if raw.is_empty() {
            return Err("capability token must not be empty".to_string());
        }
        // 全部按小写比对，让 registry 写法宽松、内存表示稳定。
        let lower = raw.to_ascii_lowercase();
        match lower.as_str() {
            "llm" => Ok(Self::Llm),
            "net" => Ok(Self::Net),
            "fs.read" => Ok(Self::FsRead),
            "fs.write" => Ok(Self::FsWrite),
            "exec" => Ok(Self::Exec),
            "exec.sudo" => Ok(Self::ExecSudo),
            other => {
                if let Some(name) = other.strip_prefix("secrets.") {
                    if name.is_empty() || name.len() > 64 {
                        return Err(format!(
                            "secrets capability name length must be 1..=64: `{token}`"
                        ));
                    }
                    if !name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    {
                        return Err(format!(
                            "secrets capability name must match [a-z0-9_]: `{token}`"
                        ));
                    }
                    // §P4.1 ↔ image/text 配置独立性：拦截"裸 vendor 名"反模式。
                    // image_generation / image_edit / image_vision / [llm] 是
                    // 4 套互相独立的 LLM provider 配置（同一 vendor 在不同用途
                    // 下可能填不同 key）。`secrets.openai_api_key` 这种命名
                    // 隐含"vendor-唯一 key"，会让 §P4.4 的 token 注入跨域串货。
                    // 必须按 `<用途>_<vendor>_api_key` 命名（如
                    // `secrets.image_generation_minimax_api_key`）。
                    const KNOWN_VENDORS: &[&str] = &[
                        "openai",
                        "google",
                        "gemini",
                        "anthropic",
                        "claude",
                        "grok",
                        "xai",
                        "deepseek",
                        "qwen",
                        "minimax",
                        "mimo",
                        "xiaomi",
                    ];
                    for vendor in KNOWN_VENDORS {
                        if name == &format!("{vendor}_api_key") || name == *vendor {
                            return Err(format!(
                                "secrets capability `{token}` uses bare vendor naming, which conflates the four independent LLM provider configs (image_generation/image_edit/image_vision/text). Rename to `secrets.<usage>_<vendor>_api_key`, e.g. `secrets.image_generation_{vendor}_api_key` or `secrets.text_{vendor}_api_key`."
                            ));
                        }
                    }
                    Ok(Self::Secrets(name.to_string()))
                } else {
                    Err(format!(
                        "unknown capability `{token}` (allowed: llm, net, fs.read, fs.write, exec, exec.sudo, secrets.<name>)"
                    ))
                }
            }
        }
    }

    /// 反向 → token，便于日志/错误信息打印（与 [`Self::parse`] 自洽）。
    pub fn as_token(&self) -> String {
        match self {
            Self::Llm => "llm".to_string(),
            Self::Net => "net".to_string(),
            Self::FsRead => "fs.read".to_string(),
            Self::FsWrite => "fs.write".to_string(),
            Self::Exec => "exec".to_string(),
            Self::ExecSudo => "exec.sudo".to_string(),
            Self::Secrets(name) => format!("secrets.{name}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SkillManifest {
    pub name: String,
    pub kind: SkillKind,
    pub output_kind: OutputKind,
    pub description: Option<String>,
    pub semantic_tags: Vec<String>,
    pub preferred_over_run_cmd: bool,
    pub validation_actions: Vec<String>,
    pub prompt_file: Option<String>,
    pub input_schema: Option<JsonValue>,
    pub output_schema: Option<JsonValue>,
    pub risk_level: Option<SkillRiskLevel>,
    pub auto_invocable: Option<bool>,
    pub requires_confirmation: Option<bool>,
    pub side_effect: Option<bool>,
    pub timeout_seconds: Option<u64>,
    pub retryable: Option<bool>,
    pub group: Option<String>,
    pub primary_fallback_role: Option<PrimaryFallbackRole>,
    /// §P4.1 主体：这条技能对外声明需要使用的能力集（去重、按 [`Capability::as_token`]
    /// 排序）。空表示"纯计算 + 标准库"，不需要任何特权资源。
    pub capabilities: Vec<Capability>,
}

/// 注册表中的单条技能定义
#[derive(Debug, Clone, Deserialize)]
pub struct SkillRegistryEntry {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub kind: SkillKind,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// 该技能专用超时（秒）；未设或 0 表示用全局默认
    #[serde(default)]
    pub timeout_seconds: u64,
    /// prompt 文件路径，相对 workspace 或绝对
    #[serde(default)]
    pub prompt_file: String,
    #[serde(default)]
    pub output_kind: OutputKind,
    #[serde(default)]
    pub description: Option<String>,
    /// Planner-facing semantic tags, for example `sqlite_table_listing`,
    /// `archive_unpack`, or `service_status`. These are descriptive routing
    /// hints, not permissions.
    #[serde(default)]
    pub semantic_tags: Vec<String>,
    /// Prefer this structured skill over ad-hoc shell commands when its
    /// semantic tags match the task.
    #[serde(default)]
    pub preferred_over_run_cmd: bool,
    /// Read-only actions that can validate or inspect the skill's domain.
    #[serde(default)]
    pub validation_actions: Vec<String>,
    #[serde(default)]
    pub input_schema: Option<TomlValue>,
    #[serde(default)]
    pub output_schema: Option<TomlValue>,
    #[serde(default)]
    pub risk_level: Option<SkillRiskLevel>,
    #[serde(default)]
    pub auto_invocable: Option<bool>,
    #[serde(default)]
    pub requires_confirmation: Option<bool>,
    #[serde(default)]
    pub side_effect: Option<bool>,
    #[serde(default)]
    pub retryable: Option<bool>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub primary_fallback_role: Option<PrimaryFallbackRole>,

    // ---------- Phase 5: 执行配置 ----------
    /// Runner 技能：在 runner 侧的执行名；未设则用 name。
    /// skill-runner 会将该值按约定解析为二进制名：
    /// 例如 foo_bar -> foo-bar-skill，若已以 -skill 结尾则直接使用。
    #[serde(default)]
    pub runner_name: Option<String>,
    /// External 技能：执行方式，如 "http_json"
    #[serde(default)]
    pub external_kind: Option<String>,
    /// External 技能：调用地址（如 HTTP URL）
    #[serde(default)]
    pub external_endpoint: Option<String>,
    /// External 技能：本地 bundle 目录（相对 workspace 或绝对路径）
    #[serde(default)]
    pub external_bundle_dir: Option<String>,
    /// External 技能：入口文件（如 SKILL.md、scripts/stock_cli.py）
    #[serde(default)]
    pub external_entry_file: Option<String>,
    /// External 技能：运行时（如 python3、node）
    #[serde(default)]
    pub external_runtime: Option<String>,
    /// External 技能：依赖的二进制命令
    #[serde(default)]
    pub external_require_bins: Vec<String>,
    /// External 技能：依赖的 Python 模块
    #[serde(default)]
    pub external_require_py_modules: Vec<String>,
    /// External 技能：来源链接或本地来源描述
    #[serde(default)]
    pub external_source_url: Option<String>,
    /// External 技能：专用超时（秒）；未设则用 timeout_seconds
    #[serde(default)]
    pub external_timeout_seconds: Option<u64>,
    /// External 技能：鉴权引用（预留，本轮不实现完整 secret 管理）
    #[serde(default)]
    pub external_auth_ref: Option<String>,

    // ---------- Phase 4.1 主体：能力声明 ----------
    /// 该技能对外声明的能力集（TOML 写法示例：
    /// `capabilities = ["llm", "net", "secrets.image_generation_minimax_api_key"]`。
    /// 注意 `secrets.<name>` 必须按 `<用途>_<vendor>_api_key` 命名，
    /// image / text / vision 等用途各自独立，不要写成 `secrets.openai_api_key` 这种跨用途的"vendor-唯一"命名）。
    ///
    /// 文件层用 `Vec<String>` 接，[`SkillsRegistry::load_from_path`] 会把它
    /// 转成 [`Capability`]（未知 token 会让 registry **加载失败**）。
    /// 转换后的强类型放在 [`SkillRegistryEntry::resolved_capabilities`]，
    /// `capabilities_raw` 字段仅作为审计/调试痕迹保留。
    #[serde(default, rename = "capabilities")]
    pub capabilities_raw: Vec<String>,

    /// 内部使用：`load_from_path` 把 `capabilities_raw` 解析后塞这里。
    /// 不被 serde 反序列化（永远从 raw 算出来，避免双源真相）。
    #[serde(skip)]
    pub resolved_capabilities: Vec<Capability>,
}

fn default_true() -> bool {
    true
}

fn trim_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn normalize_metadata_tokens(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let token = value.trim().to_ascii_lowercase();
        if token.is_empty() || out.iter().any(|existing| existing == &token) {
            continue;
        }
        out.push(token);
    }
    out
}

fn toml_value_to_json(value: &TomlValue) -> Option<JsonValue> {
    serde_json::to_value(value).ok()
}

/// 整张技能注册表（通常从 TOML [[skills]] 加载）
#[derive(Debug, Clone, Default)]
pub struct SkillsRegistry {
    /// 按 canonical name 索引
    by_name: HashMap<String, SkillRegistryEntry>,
    /// 别名 -> canonical name
    alias_to_name: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct SkillsRegistryFile {
    #[serde(default)]
    skills: Vec<SkillRegistryEntry>,
}

impl SkillsRegistry {
    /// 从 TOML 文件加载；路径为绝对或相对当前工作目录。文件不存在或解析失败返回 Ok(空 registry)。
    pub fn load_from_path(path: &Path) -> Result<Self, String> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(e) => return Err(format!("read registry failed: {}: {}", path.display(), e)),
        };
        let file: SkillsRegistryFile = toml::from_str(&content)
            .map_err(|e| format!("parse registry failed: {}: {}", path.display(), e))?;
        let mut by_name = HashMap::new();
        let mut alias_to_name = HashMap::new();
        for mut entry in file.skills {
            let raw = entry.name.trim().to_string();
            if raw.is_empty() {
                continue;
            }
            let canonical = to_canonical_key(&raw);
            if by_name.contains_key(&canonical) {
                return Err(format!(
                    "duplicate skill name `{canonical}` in {}",
                    path.display()
                ));
            }
            entry.name = canonical.clone();
            let aliases: Vec<String> = entry
                .aliases
                .iter()
                .map(|a| to_canonical_key(a))
                .filter(|a| !a.is_empty() && *a != canonical)
                .collect();
            entry.semantic_tags = normalize_metadata_tokens(&entry.semantic_tags);
            entry.validation_actions = normalize_metadata_tokens(&entry.validation_actions);
            // §P4.1 主体：把 capabilities_raw 翻译成强类型，未知 token 直接报错。
            // 排序 + dedup 让"声明顺序"不影响等价性，并避免重复声明在策略层
            // 引出二义性。
            let mut resolved: Vec<Capability> = Vec::with_capacity(entry.capabilities_raw.len());
            for token in &entry.capabilities_raw {
                let cap = Capability::parse(token).map_err(|e| {
                    format!(
                        "parse capabilities for skill `{canonical}` in {}: {e}",
                        path.display()
                    )
                })?;
                resolved.push(cap);
            }
            resolved.sort_by(|a, b| a.as_token().cmp(&b.as_token()));
            resolved.dedup();
            entry.resolved_capabilities = resolved;

            by_name.insert(canonical.clone(), entry);
            if let Some(existing) = alias_to_name.get(&canonical) {
                if existing != &canonical {
                    return Err(format!(
                        "duplicate skill alias/name `{canonical}` in {}: `{existing}` and `{canonical}`",
                        path.display()
                    ));
                }
            } else {
                alias_to_name.insert(canonical.clone(), canonical.clone());
            }
            for a in &aliases {
                if let Some(existing) = alias_to_name.get(a) {
                    if existing != &canonical {
                        return Err(format!(
                            "duplicate skill alias `{a}` in {}: `{existing}` and `{canonical}`",
                            path.display()
                        ));
                    }
                } else {
                    alias_to_name.insert(a.clone(), canonical.clone());
                }
            }
        }
        let registry = Self {
            by_name,
            alias_to_name,
        };

        // §P4.2：声明的 capabilities 必须和 manifest 的 shape 一致，否则
        // 加载失败 — 例如 exec.sudo 不允许自动执行（必须 confirm + high
        // risk），fs.write/exec 不允许显式 side_effect=false。
        let shape_violations = registry.validate_shape_consistency();
        if !shape_violations.is_empty() {
            return Err(format!(
                "skill registry shape consistency check failed in {}:\n  - {}",
                path.display(),
                shape_violations.join("\n  - ")
            ));
        }

        Ok(registry)
    }

    /// 解析别名或名称得到 canonical name（小写）；不存在则返回 None
    pub fn resolve_canonical(&self, name: &str) -> Option<&str> {
        let key = to_canonical_key(name);
        if key.is_empty() {
            return None;
        }
        self.alias_to_name.get(&key).map(String::as_str)
    }

    /// 按 canonical name 取条目
    pub fn get(&self, canonical_name: &str) -> Option<&SkillRegistryEntry> {
        self.by_name.get(&to_canonical_key(canonical_name))
    }

    /// 所有在注册表中且 enabled 的 canonical 名称
    pub fn enabled_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .by_name
            .iter()
            .filter(|(_, e)| e.enabled)
            .map(|(n, _)| n.clone())
            .collect();
        names.sort();
        names
    }

    /// 该技能在 registry 中配置的 timeout（秒）；0 表示用全局默认
    pub fn timeout_seconds(&self, canonical_name: &str) -> u64 {
        self.get(canonical_name)
            .and_then(|e| {
                if e.timeout_seconds > 0 {
                    Some(e.timeout_seconds)
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    /// 所有已注册的 canonical 名称（含未启用）
    pub fn all_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.by_name.keys().cloned().collect();
        names.sort();
        names
    }

    /// 是否在注册表中（别名或 canonical 能解析即视为已知）
    pub fn is_known(&self, name: &str) -> bool {
        self.resolve_canonical(name).is_some()
    }

    /// 该技能在 registry 中是否为 builtin 类型
    pub fn is_builtin(&self, canonical_name: &str) -> bool {
        self.get(canonical_name)
            .map(|e| e.kind == SkillKind::Builtin)
            .unwrap_or(false)
    }

    /// 该技能在 registry 中配置的 prompt 文件路径；空字符串视为 None
    pub fn prompt_file(&self, canonical_name: &str) -> Option<&str> {
        self.get(canonical_name).and_then(|e| {
            let s = e.prompt_file.trim();
            if s.is_empty() {
                None
            } else {
                Some(e.prompt_file.as_str())
            }
        })
    }

    /// Phase 5: Runner 执行名；未配置则用 canonical_name。返回 String 避免混合借用来源的 lifetime 问题。
    pub fn runner_name(&self, canonical_name: &str) -> String {
        self.get(canonical_name)
            .and_then(|e| e.runner_name.as_deref())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| canonical_name.to_string())
    }

    /// Phase 5: External 执行配置（仅当 kind=External 且配置完整时返回）
    pub fn external_config(&self, canonical_name: &str) -> Option<ExternalSkillConfig<'_>> {
        let e = self.get(canonical_name)?;
        if e.kind != SkillKind::External {
            return None;
        }
        let kind = e
            .external_kind
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        Some(ExternalSkillConfig {
            kind,
            endpoint: e
                .external_endpoint
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            bundle_dir: e
                .external_bundle_dir
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            entry_file: e
                .external_entry_file
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            runtime: e
                .external_runtime
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            require_bins: &e.external_require_bins,
            require_py_modules: &e.external_require_py_modules,
            source_url: e
                .external_source_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            timeout_seconds: e
                .external_timeout_seconds
                .filter(|&t| t > 0)
                .or_else(|| (e.timeout_seconds > 0).then_some(e.timeout_seconds)),
            auth_ref: e.external_auth_ref.as_deref(),
        })
    }

    pub fn manifest(&self, canonical_name: &str) -> Option<SkillManifest> {
        let entry = self.get(canonical_name)?;
        let timeout_seconds = entry
            .external_timeout_seconds
            .filter(|&t| t > 0)
            .or_else(|| (entry.timeout_seconds > 0).then_some(entry.timeout_seconds));
        Some(SkillManifest {
            name: entry.name.clone(),
            kind: entry.kind,
            output_kind: entry.output_kind,
            description: trim_optional_string(entry.description.as_deref()),
            semantic_tags: entry.semantic_tags.clone(),
            preferred_over_run_cmd: entry.preferred_over_run_cmd,
            validation_actions: entry.validation_actions.clone(),
            prompt_file: trim_optional_string(Some(entry.prompt_file.as_str())),
            input_schema: entry.input_schema.as_ref().and_then(toml_value_to_json),
            output_schema: entry.output_schema.as_ref().and_then(toml_value_to_json),
            risk_level: entry.risk_level,
            auto_invocable: entry.auto_invocable,
            requires_confirmation: entry.requires_confirmation,
            side_effect: entry.side_effect,
            timeout_seconds,
            retryable: entry.retryable,
            group: trim_optional_string(entry.group.as_deref()),
            primary_fallback_role: entry.primary_fallback_role,
            capabilities: entry.resolved_capabilities.clone(),
        })
    }

    /// §P4.1 主体：取该技能的强类型能力声明（已去重 + 排序）。
    /// 未注册的技能返回空切片（与"未声明任何能力"语义同 — 都不应放行任何
    /// 受控资源）。
    pub fn capabilities(&self, canonical_name: &str) -> &[Capability] {
        match self.get(canonical_name) {
            Some(entry) => entry.resolved_capabilities.as_slice(),
            None => &[],
        }
    }

    /// §P4.1 主体：审计/策略层的命中查询。`secrets.<name>` 走精确匹配，
    /// 其他能力走变体相等。例：调用 `has_capability("image_generate", &Capability::Llm)`。
    pub fn has_capability(&self, canonical_name: &str, cap: &Capability) -> bool {
        self.capabilities(canonical_name).iter().any(|c| c == cap)
    }

    /// §P4.2：capability ↔ skill shape 一致性审计。
    ///
    /// 规则（PR 阶段就会触发，不会等到 runtime 才暴）：
    /// - **R1** `exec.sudo` ⇒ 必须 `requires_confirmation = true`，禁止自动提权。
    /// - **R2** `exec.sudo` ⇒ 必须 `risk_level = "high"`，让所有 risk 路由都把它当最高级。
    /// - **R3** 含 `fs.write` / `exec` / `exec.sudo` ⇒ 禁止显式 `side_effect = false`
    ///   （未声明时容忍，迁移友好；一旦显式关掉，必然是误配）。
    ///
    /// 返回违规列表（按字符串排序，便于 diff 稳定）。空列表表示通过。
    /// 这个函数对外公开是为了让 CI 单独跑一遍以保证 registry 文件本身合法；
    /// `load_from_path` 内部已经在加载流程结束时调用它，违规会让 registry
    /// **加载失败**（与 `Capability::parse` 的失败行为一致）。
    pub fn validate_shape_consistency(&self) -> Vec<String> {
        let mut violations: Vec<String> = Vec::new();

        for (name, entry) in &self.by_name {
            let caps = &entry.resolved_capabilities;
            let has = |c: &Capability| caps.contains(c);

            if has(&Capability::ExecSudo) {
                if entry.requires_confirmation != Some(true) {
                    violations.push(format!(
                        "skill `{name}` declares `exec.sudo` but `requires_confirmation` is not `true` (R1)"
                    ));
                }
                if entry.risk_level != Some(SkillRiskLevel::High) {
                    violations.push(format!(
                        "skill `{name}` declares `exec.sudo` but `risk_level` is not `high` (R2)"
                    ));
                }
            }

            let has_write_or_exec =
                has(&Capability::FsWrite) || has(&Capability::Exec) || has(&Capability::ExecSudo);
            if has_write_or_exec && entry.side_effect == Some(false) {
                violations.push(format!(
                    "skill `{name}` declares fs.write/exec/exec.sudo but `side_effect = false` is set explicitly (R3)"
                ));
            }
        }

        violations.sort();
        violations
    }
}

/// Phase 5: External 技能执行配置（只读引用）
#[derive(Debug, Clone)]
pub struct ExternalSkillConfig<'a> {
    pub kind: &'a str,
    pub endpoint: Option<&'a str>,
    pub bundle_dir: Option<&'a str>,
    pub entry_file: Option<&'a str>,
    pub runtime: Option<&'a str>,
    pub require_bins: &'a [String],
    pub require_py_modules: &'a [String],
    pub source_url: Option<&'a str>,
    pub timeout_seconds: Option<u64>,
    pub auth_ref: Option<&'a str>,
}

/// 小写化（技能名仅 ASCII，用 to_lowercase 即可）
fn to_canonical_key(s: &str) -> String {
    s.trim().to_lowercase()
}

/// §P4.1 收尾：clawd 进程内"必须存在且 kind=builtin"的技能 canonical 集合。
///
/// 这一组在 `crates/clawd/src/skills.rs::is_builtin_skill_name` 仍保留作为
/// registry 缺失时的安全网，但运行期权威是这里。任何变动都需要同时更新
/// 那张安全网；CI 上有 `crates/clawd/tests/config_templates.rs` 里的
/// `registry_covers_all_required_builtins` 守底，registry 漏一个就红。
pub const REQUIRED_BUILTIN_SKILLS: &[&str] = &[
    "run_cmd",
    "read_file",
    "write_file",
    "list_dir",
    "make_dir",
    "remove_file",
    "schedule",
];

/// §P4.1 收尾：registry 完整性校验报告，便于启动期 / CI 一次性输出全部漂移点。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RegistryIntegrityReport {
    /// 在 [`REQUIRED_BUILTIN_SKILLS`] 里但 registry 完全找不到的 canonical name。
    pub missing: Vec<String>,
    /// 找到了，但 `kind` 不是 `Builtin`（例如被误改成 `Runner`）的 canonical name。
    pub wrong_kind: Vec<String>,
}

impl RegistryIntegrityReport {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.wrong_kind.is_empty()
    }

    /// 把报告打成给人看的一行错误描述。空报告返回 None。
    pub fn into_human_message(self) -> Option<String> {
        if self.is_clean() {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        if !self.missing.is_empty() {
            parts.push(format!("missing builtins: {}", self.missing.join(", ")));
        }
        if !self.wrong_kind.is_empty() {
            parts.push(format!(
                "builtins with wrong kind (expected kind=builtin): {}",
                self.wrong_kind.join(", ")
            ));
        }
        Some(parts.join("; "))
    }
}

impl SkillsRegistry {
    /// 检查 registry 是否覆盖了 [`REQUIRED_BUILTIN_SKILLS`]，并且每条都标
    /// `kind = "builtin"`。返回结构化报告，便于一次性输出所有漂移点。
    ///
    /// 这是 §P4.1 alias 收敛子项的"启动期 + CI 双保险"基础：
    /// - clawd 启动时调一次，发现漂移直接 bail；
    /// - `tests/config_templates.rs` 在 CI 跑同一套校验，避免 dev 漏跑。
    pub fn integrity_report(&self) -> RegistryIntegrityReport {
        let mut missing: Vec<String> = Vec::new();
        let mut wrong_kind: Vec<String> = Vec::new();
        for name in REQUIRED_BUILTIN_SKILLS {
            match self.get(name) {
                None => missing.push((*name).to_string()),
                Some(entry) if entry.kind != SkillKind::Builtin => {
                    wrong_kind.push((*name).to_string());
                }
                Some(_) => {}
            }
        }
        RegistryIntegrityReport {
            missing,
            wrong_kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registry stores `prompt_file` as a logical path
    /// (e.g. prompts/skills/run_cmd.md).
    /// Runtime in clawd assembles skill prompts from
    /// prompts/layers/generated/skills/<name>.md plus optional
    /// prompts/layers/vendor_patches/<vendor>/skills/<name>.md.
    #[test]
    fn test_registry_resolve_and_timeout() {
        let toml = r#"
[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"
aliases = ["shell", "exec"]
timeout_seconds = 60
prompt_file = "prompts/skills/run_cmd.md"
output_kind = "text"

[[skills]]
name = "image_vision"
enabled = true
kind = "runner"
aliases = ["vision"]
timeout_seconds = 90
prompt_file = "prompts/skills/image_vision.md"
output_kind = "image"
"#;
        let path = std::env::temp_dir().join("test_skills_registry.toml");
        std::fs::write(&path, toml).unwrap();
        let reg = SkillsRegistry::load_from_path(&path).unwrap();
        assert_eq!(reg.resolve_canonical("run_cmd"), Some("run_cmd"));
        assert_eq!(reg.resolve_canonical("shell"), Some("run_cmd"));
        assert_eq!(reg.resolve_canonical("exec"), Some("run_cmd"));
        assert_eq!(reg.resolve_canonical("image_vision"), Some("image_vision"));
        assert_eq!(reg.resolve_canonical("vision"), Some("image_vision"));
        assert_eq!(reg.timeout_seconds("run_cmd"), 60);
        assert_eq!(reg.timeout_seconds("image_vision"), 90);
        assert!(reg.enabled_names().contains(&"run_cmd".to_string()));
        assert!(reg.enabled_names().contains(&"image_vision".to_string()));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn integrity_report_flags_missing_and_wrong_kind() {
        let toml = r#"
[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"

[[skills]]
name = "read_file"
enabled = true
kind = "runner"   # 故意写错 kind，应该被 wrong_kind 抓到
"#;
        let path = std::env::temp_dir().join("test_registry_integrity_report.toml");
        std::fs::write(&path, toml).unwrap();
        let reg = SkillsRegistry::load_from_path(&path).unwrap();
        let report = reg.integrity_report();

        assert!(!report.is_clean());
        assert!(
            report.missing.contains(&"write_file".to_string()),
            "missing should include uncovered builtins, got {:?}",
            report.missing
        );
        assert_eq!(report.wrong_kind, vec!["read_file".to_string()]);

        let human = report.into_human_message().unwrap();
        assert!(human.contains("missing builtins"));
        assert!(human.contains("wrong kind"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn registry_load_rejects_duplicate_aliases_across_skills() {
        let toml = r#"
[[skills]]
name = "system_basic"
enabled = true
kind = "runner"
aliases = ["system"]

[[skills]]
name = "service_control"
enabled = true
kind = "runner"
aliases = ["system"]
"#;
        let path = std::env::temp_dir().join("test_registry_duplicate_alias.toml");
        std::fs::write(&path, toml).unwrap();
        let err = SkillsRegistry::load_from_path(&path)
            .err()
            .expect("duplicate aliases must fail registry load");
        assert!(err.contains("duplicate skill alias `system`"), "{err}");
        assert!(err.contains("system_basic"), "{err}");
        assert!(err.contains("service_control"), "{err}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn registry_load_rejects_alias_colliding_with_later_skill_name() {
        let toml = r#"
[[skills]]
name = "system_basic"
enabled = true
kind = "runner"
aliases = ["service_control"]

[[skills]]
name = "service_control"
enabled = true
kind = "runner"
"#;
        let path = std::env::temp_dir().join("test_registry_alias_name_collision.toml");
        std::fs::write(&path, toml).unwrap();
        let err = SkillsRegistry::load_from_path(&path)
            .err()
            .expect("alias/name collisions must fail registry load");
        assert!(
            err.contains("duplicate skill alias/name `service_control`"),
            "{err}"
        );
        assert!(err.contains("system_basic"), "{err}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn capability_parses_closed_set_and_secrets_namespace() {
        // 注意：`secrets.<name>` 用 `<用途>_<vendor>_api_key` 命名，避免把
        // image / chat / vision 三套独立 LLM 配置的 key 串到一起。
        for token in [
            "llm",
            "net",
            "fs.read",
            "fs.write",
            "exec",
            "exec.sudo",
            "secrets.image_generation_minimax_api_key",
            "secrets.text_openai_api_key",
        ] {
            let cap = Capability::parse(token).unwrap_or_else(|e| {
                panic!("expected `{token}` to parse, got error: {e}");
            });
            assert_eq!(cap.as_token(), token);
        }
    }

    #[test]
    fn capability_parse_is_case_insensitive_but_normalizes_to_lowercase() {
        assert_eq!(Capability::parse("LLM").unwrap(), Capability::Llm);
        assert_eq!(Capability::parse("FS.Write").unwrap(), Capability::FsWrite);
        assert_eq!(
            Capability::parse("Secrets.Image_Generation_MiniMax_Api_Key").unwrap(),
            Capability::Secrets("image_generation_minimax_api_key".to_string())
        );
    }

    #[test]
    fn capability_rejects_unknown_tokens_and_bad_secret_names() {
        // 完全未知词
        assert!(Capability::parse("disk").is_err());
        // 空 token
        assert!(Capability::parse("").is_err());
        // secrets 但名字非法（含点、空、过长）
        assert!(Capability::parse("secrets.").is_err());
        assert!(Capability::parse("secrets.bad-name").is_err());
        assert!(Capability::parse("secrets.has space").is_err());
        let too_long = format!("secrets.{}", "a".repeat(65));
        assert!(Capability::parse(&too_long).is_err());
    }

    #[test]
    fn capability_rejects_bare_vendor_secret_naming() {
        // 反模式：会让 image / text / vision / chat 共用 key —— 必须拒。
        for token in [
            "secrets.openai_api_key",
            "secrets.gemini_api_key",
            "secrets.anthropic_api_key",
            "secrets.claude_api_key",
            "secrets.qwen_api_key",
            "secrets.minimax_api_key",
            // 极端反模式：连 _api_key 都不带，纯 vendor 名
            "secrets.openai",
            "secrets.minimax",
        ] {
            let err = Capability::parse(token).err().unwrap_or_else(|| {
                panic!("expected `{token}` to be rejected as bare-vendor naming")
            });
            assert!(
                err.contains("bare vendor naming"),
                "unexpected error for `{token}`: {err}"
            );
            assert!(
                err.contains("<usage>_<vendor>_api_key"),
                "error should hint the canonical naming pattern: {err}"
            );
        }

        // 正模式：带用途前缀必须放行。
        for token in [
            "secrets.image_generation_minimax_api_key",
            "secrets.image_edit_qwen_api_key",
            "secrets.image_vision_openai_api_key",
            "secrets.text_openai_api_key",
            "secrets.chat_minimax_api_key",
        ] {
            assert!(
                Capability::parse(token).is_ok(),
                "expected `{token}` to be accepted"
            );
        }
    }

    #[test]
    fn registry_load_resolves_capabilities_and_dedups() {
        // 故意写两次 "llm" 验证 dedup；secret 用 image_generation 命名空间，
        // 与 chat/规划用的 [llm] 配置完全分离，杜绝把 image 的 key 注入到 text 链路。
        let toml = r#"
[[skills]]
name = "image_generate"
enabled = true
kind = "runner"
side_effect = true
capabilities = ["llm", "net", "fs.write", "llm", "secrets.image_generation_minimax_api_key"]
"#;
        let path = std::env::temp_dir().join("test_registry_capabilities_ok.toml");
        std::fs::write(&path, toml).unwrap();
        let reg = SkillsRegistry::load_from_path(&path).unwrap();
        let caps = reg.capabilities("image_generate");
        let tokens: Vec<String> = caps.iter().map(Capability::as_token).collect();
        // 已 dedup（"llm" 出现两次）+ 已按 as_token 排序
        assert_eq!(
            tokens,
            vec![
                "fs.write".to_string(),
                "llm".to_string(),
                "net".to_string(),
                "secrets.image_generation_minimax_api_key".to_string(),
            ]
        );
        assert!(reg.has_capability("image_generate", &Capability::Llm));
        assert!(reg.has_capability(
            "image_generate",
            &Capability::Secrets("image_generation_minimax_api_key".to_string())
        ));
        // 关键：image_generation 的 key 不应被 chat/text 链路误命中
        assert!(!reg.has_capability(
            "image_generate",
            &Capability::Secrets("text_minimax_api_key".to_string())
        ));
        assert!(!reg.has_capability("image_generate", &Capability::ExecSudo));
        // manifest 视图也带上
        let manifest = reg.manifest("image_generate").unwrap();
        assert_eq!(manifest.capabilities.len(), 4);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn registry_manifest_exposes_planner_metadata() {
        let toml = r#"
[[skills]]
name = "db_basic"
enabled = true
kind = "runner"
description = "Use structured SQLite actions."
semantic_tags = ["sqlite_query", "SQLite_Query", "sqlite_table_listing", ""]
preferred_over_run_cmd = true
validation_actions = ["sqlite_query", "SQLITE_QUERY"]
"#;
        let path = std::env::temp_dir().join("test_registry_planner_metadata.toml");
        std::fs::write(&path, toml).unwrap();
        let reg = SkillsRegistry::load_from_path(&path).unwrap();
        let manifest = reg.manifest("db_basic").unwrap();
        assert_eq!(
            manifest.description.as_deref(),
            Some("Use structured SQLite actions.")
        );
        assert_eq!(
            manifest.semantic_tags,
            vec![
                "sqlite_query".to_string(),
                "sqlite_table_listing".to_string()
            ]
        );
        assert!(manifest.preferred_over_run_cmd);
        assert_eq!(
            manifest.validation_actions,
            vec!["sqlite_query".to_string()]
        );
        let entry = reg.get("db_basic").unwrap();
        assert_eq!(entry.semantic_tags, manifest.semantic_tags);
        assert_eq!(entry.validation_actions, manifest.validation_actions);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shape_consistency_rejects_exec_sudo_without_confirmation() {
        let toml = r#"
[[skills]]
name = "danger_skill"
enabled = true
kind = "runner"
risk_level = "high"
side_effect = true
capabilities = ["exec.sudo"]
# 故意没设 requires_confirmation
"#;
        let path = std::env::temp_dir().join("test_shape_exec_sudo_no_confirm.toml");
        std::fs::write(&path, toml).unwrap();
        let err = SkillsRegistry::load_from_path(&path)
            .err()
            .expect("expected load to fail when exec.sudo lacks requires_confirmation");
        assert!(err.contains("`danger_skill`"), "{err}");
        assert!(err.contains("requires_confirmation"), "{err}");
        assert!(err.contains("R1"), "{err}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shape_consistency_rejects_exec_sudo_without_high_risk() {
        let toml = r#"
[[skills]]
name = "danger_skill"
enabled = true
kind = "runner"
requires_confirmation = true
side_effect = true
risk_level = "medium"
capabilities = ["exec.sudo"]
"#;
        let path = std::env::temp_dir().join("test_shape_exec_sudo_medium_risk.toml");
        std::fs::write(&path, toml).unwrap();
        let err = SkillsRegistry::load_from_path(&path)
            .err()
            .expect("expected load to fail when exec.sudo is not high risk");
        assert!(err.contains("risk_level"), "{err}");
        assert!(err.contains("R2"), "{err}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shape_consistency_rejects_explicit_side_effect_false_with_write_cap() {
        let toml = r#"
[[skills]]
name = "lying_skill"
enabled = true
kind = "runner"
side_effect = false
capabilities = ["fs.write"]
"#;
        let path = std::env::temp_dir().join("test_shape_fs_write_no_side_effect.toml");
        std::fs::write(&path, toml).unwrap();
        let err = SkillsRegistry::load_from_path(&path)
            .err()
            .expect("expected load to fail when fs.write declared with side_effect=false");
        assert!(err.contains("`lying_skill`"), "{err}");
        assert!(err.contains("side_effect"), "{err}");
        assert!(err.contains("R3"), "{err}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shape_consistency_passes_with_proper_declarations() {
        // 完全合规：exec.sudo + confirm + high + side_effect=true
        let toml = r#"
[[skills]]
name = "safe_sudo"
enabled = true
kind = "runner"
requires_confirmation = true
risk_level = "high"
side_effect = true
capabilities = ["exec.sudo"]

[[skills]]
name = "writer"
enabled = true
kind = "runner"
side_effect = true
capabilities = ["fs.write"]

[[skills]]
name = "writer_unspecified_side_effect"
enabled = true
kind = "runner"
# 没设 side_effect — 容忍 None，但禁止显式 false
capabilities = ["fs.write"]
"#;
        let path = std::env::temp_dir().join("test_shape_consistency_clean.toml");
        std::fs::write(&path, toml).unwrap();
        let reg =
            SkillsRegistry::load_from_path(&path).expect("registry with proper shape should load");
        assert!(reg.validate_shape_consistency().is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn registry_load_rejects_unknown_capability_token() {
        let toml = r#"
[[skills]]
name = "foo"
enabled = true
kind = "runner"
capabilities = ["llm", "wifi"]
"#;
        let path = std::env::temp_dir().join("test_registry_capabilities_bad.toml");
        std::fs::write(&path, toml).unwrap();
        let err = SkillsRegistry::load_from_path(&path)
            .err()
            .expect("expected load to fail on unknown capability token");
        assert!(
            err.contains("`foo`"),
            "error should mention skill name: {err}"
        );
        assert!(
            err.contains("wifi"),
            "error should mention bad token: {err}"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn integrity_report_clean_when_all_required_builtins_present() {
        let mut toml = String::new();
        for name in REQUIRED_BUILTIN_SKILLS {
            toml.push_str(&format!(
                "[[skills]]\nname = \"{name}\"\nenabled = true\nkind = \"builtin\"\n\n"
            ));
        }
        let path = std::env::temp_dir().join("test_registry_integrity_clean.toml");
        std::fs::write(&path, toml).unwrap();
        let reg = SkillsRegistry::load_from_path(&path).unwrap();
        let report = reg.integrity_report();
        assert!(report.is_clean(), "expected clean report, got {report:?}");
        assert!(report.into_human_message().is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_registry_manifest_view() {
        let toml = r#"
[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"
prompt_file = "prompts/skills/run_cmd.md"
description = "Run a shell command"
risk_level = "high"
auto_invocable = false
requires_confirmation = true
side_effect = true
retryable = true
group = "shell"
primary_fallback_role = "primary"
input_schema = { type = "object", required = ["command"] }
output_schema = { type = "object", properties = { text = { type = "string" } } }
"#;
        let path = std::env::temp_dir().join("test_skills_manifest_registry.toml");
        std::fs::write(&path, toml).unwrap();
        let reg = SkillsRegistry::load_from_path(&path).unwrap();
        let manifest = reg.manifest("run_cmd").unwrap();
        assert_eq!(manifest.name, "run_cmd");
        assert_eq!(manifest.description.as_deref(), Some("Run a shell command"));
        assert_eq!(
            manifest.prompt_file.as_deref(),
            Some("prompts/skills/run_cmd.md")
        );
        assert_eq!(manifest.risk_level, Some(SkillRiskLevel::High));
        assert_eq!(manifest.auto_invocable, Some(false));
        assert_eq!(manifest.requires_confirmation, Some(true));
        assert_eq!(manifest.side_effect, Some(true));
        assert_eq!(manifest.retryable, Some(true));
        assert_eq!(manifest.group.as_deref(), Some("shell"));
        assert_eq!(
            manifest.primary_fallback_role,
            Some(PrimaryFallbackRole::Primary)
        );
        assert_eq!(
            manifest
                .input_schema
                .as_ref()
                .and_then(|v| v.get("type"))
                .and_then(|v| v.as_str()),
            Some("object")
        );
        assert_eq!(
            manifest
                .output_schema
                .as_ref()
                .and_then(|v| v.get("type"))
                .and_then(|v| v.as_str()),
            Some("object")
        );
        let _ = std::fs::remove_file(path);
    }
}
