//! 技能注册表：从 TOML 驱动技能的发现、启用、别名、超时与 prompt 路径。
//! Phase 1：仅做“发现/启用/别名/超时”从 registry 读取，执行层与 planner 仍用现有逻辑。

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

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
}

fn default_true() -> bool {
    true
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
            entry.name = canonical.clone();
            let aliases: Vec<String> = entry
                .aliases
                .iter()
                .map(|a| to_canonical_key(a))
                .filter(|a| !a.is_empty() && *a != canonical)
                .collect();
            by_name.insert(canonical.clone(), entry);
            alias_to_name.insert(canonical.clone(), canonical.clone());
            for a in &aliases {
                if !alias_to_name.contains_key(a) {
                    alias_to_name.insert(a.clone(), canonical.clone());
                }
            }
        }
        Ok(Self {
            by_name,
            alias_to_name,
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Registry stores prompt_file as a logical path (e.g. prompts/skills/run_cmd.md).
    /// Runtime in clawd resolves to prompts/vendors/<vendor>/skills/ or prompts/vendors/default/skills/ only.
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
}
