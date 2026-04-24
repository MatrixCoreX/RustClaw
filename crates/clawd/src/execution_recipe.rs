use std::path::Path;

use serde_json::Value;

use crate::{AppState, RouteResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ExecutionRecipeKind {
    #[default]
    None,
    OpsClosedLoop,
}

impl ExecutionRecipeKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::OpsClosedLoop => "ops_closed_loop",
        }
    }
}

pub(crate) fn parse_execution_recipe_kind_text(value: &str) -> ExecutionRecipeKind {
    match value.trim().to_ascii_lowercase().as_str() {
        "ops_closed_loop" | "ops" | "closed_loop" => ExecutionRecipeKind::OpsClosedLoop,
        _ => ExecutionRecipeKind::None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ExecutionRecipeProfile {
    #[default]
    None,
    OpsService,
    ConfigChange,
    CodeChange,
    SkillAuthoring,
}

impl ExecutionRecipeProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::OpsService => "ops_service",
            Self::ConfigChange => "config_change",
            Self::CodeChange => "code_change",
            Self::SkillAuthoring => "skill_authoring",
        }
    }
}

pub(crate) fn parse_execution_recipe_profile_text(value: &str) -> ExecutionRecipeProfile {
    match value.trim().to_ascii_lowercase().as_str() {
        "ops_service" | "ops" | "service_ops" => ExecutionRecipeProfile::OpsService,
        "config_change" | "config" => ExecutionRecipeProfile::ConfigChange,
        "code_change" | "code" | "project_change" | "problem_resolution" => {
            ExecutionRecipeProfile::CodeChange
        }
        "skill_authoring" | "skill" | "extension_build" => ExecutionRecipeProfile::SkillAuthoring,
        _ => ExecutionRecipeProfile::None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ExecutionRecipeTargetScope {
    #[default]
    Unknown,
    System,
    CurrentRepo,
    ExternalWorkspace,
    Greenfield,
}

impl ExecutionRecipeTargetScope {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::System => "system",
            Self::CurrentRepo => "current_repo",
            Self::ExternalWorkspace => "external_workspace",
            Self::Greenfield => "greenfield",
        }
    }
}

pub(crate) fn parse_execution_recipe_target_scope_text(value: &str) -> ExecutionRecipeTargetScope {
    match value.trim().to_ascii_lowercase().as_str() {
        "system" | "host" => ExecutionRecipeTargetScope::System,
        "current_repo" | "repo" | "current_workspace" | "workspace" => {
            ExecutionRecipeTargetScope::CurrentRepo
        }
        "external_workspace" | "external" | "other_workspace" => {
            ExecutionRecipeTargetScope::ExternalWorkspace
        }
        "greenfield" | "new_project" | "new_script" => ExecutionRecipeTargetScope::Greenfield,
        _ => ExecutionRecipeTargetScope::Unknown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ExecutionRecipePhase {
    #[default]
    Inspect,
    Apply,
    Validate,
    Repair,
    Done,
}

impl ExecutionRecipePhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::Apply => "apply",
            Self::Validate => "validate",
            Self::Repair => "repair",
            Self::Done => "done",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ExecutionRecipeSpec {
    pub(crate) kind: ExecutionRecipeKind,
    pub(crate) profile: ExecutionRecipeProfile,
    pub(crate) target_scope: ExecutionRecipeTargetScope,
    pub(crate) inspect_first: bool,
    pub(crate) validation_required: bool,
    pub(crate) max_repairs: usize,
}

pub(crate) fn explicit_execution_recipe_spec(
    kind: ExecutionRecipeKind,
    profile: ExecutionRecipeProfile,
    target_scope: ExecutionRecipeTargetScope,
) -> Option<ExecutionRecipeSpec> {
    if matches!(kind, ExecutionRecipeKind::None) || matches!(profile, ExecutionRecipeProfile::None)
    {
        return None;
    }
    Some(ExecutionRecipeSpec {
        kind,
        profile,
        target_scope,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ExecutionRecipeRuntimeState {
    pub(crate) kind: ExecutionRecipeKind,
    pub(crate) profile: ExecutionRecipeProfile,
    pub(crate) target_scope: ExecutionRecipeTargetScope,
    pub(crate) phase: ExecutionRecipePhase,
    pub(crate) inspect_first: bool,
    pub(crate) validation_required: bool,
    pub(crate) max_repairs: usize,
    pub(crate) repair_count: usize,
    pub(crate) saw_inspect: bool,
    pub(crate) saw_mutation: bool,
    pub(crate) saw_validation: bool,
    pub(crate) saw_external_target: bool,
    pub(crate) saw_greenfield_creation: bool,
}

impl ExecutionRecipeRuntimeState {
    fn profile_guidance(self) -> &'static str {
        match self.profile {
            ExecutionRecipeProfile::None => {
                "Treat this as a general closed-loop execution task."
            }
            ExecutionRecipeProfile::OpsService => {
                "Focus on service/system state: inspect status, logs, ports, and runtime config before mutating; validate with machine-verifiable service or network signals."
            }
            ExecutionRecipeProfile::ConfigChange => {
                "Focus on configuration safety: inspect the current file and effective values first, prefer minimal targeted edits, and validate parse/reload/effective-state after changes."
            }
            ExecutionRecipeProfile::CodeChange => {
                "Focus on solving the requested code or script problem: inspect relevant files and current failures first, keep edits scoped, and validate with compile/test/lint/runtime evidence."
            }
            ExecutionRecipeProfile::SkillAuthoring => {
                "Focus on building or updating a reusable skill/extension: inspect existing interface, registration, prompts, and docs first, then validate structure, integration points, and targeted tests."
            }
        }
    }

    fn target_scope_guidance(self) -> &'static str {
        match self.target_scope {
            ExecutionRecipeTargetScope::Unknown => {
                "Scope is not explicit. Infer the smallest safe working area from the request before mutating."
            }
            ExecutionRecipeTargetScope::System => {
                "Target scope is the host system or service environment. Prefer system/service evidence over repository-local assumptions."
            }
            ExecutionRecipeTargetScope::CurrentRepo => {
                "Target scope is the current repository/workspace. Prefer local project files, tests, and configs in this repo."
            }
            ExecutionRecipeTargetScope::ExternalWorkspace => {
                "Target scope is outside the current repo. Confirm paths and avoid assuming the current workspace contains the target files."
            }
            ExecutionRecipeTargetScope::Greenfield => {
                "Target scope is greenfield creation. Create the minimal new files or scaffold needed, then validate the new artifact works."
            }
        }
    }

    pub(crate) fn from_spec(spec: ExecutionRecipeSpec) -> Self {
        Self {
            kind: spec.kind,
            profile: spec.profile,
            target_scope: spec.target_scope,
            phase: if matches!(spec.kind, ExecutionRecipeKind::None) {
                ExecutionRecipePhase::Done
            } else {
                ExecutionRecipePhase::Inspect
            },
            inspect_first: spec.inspect_first,
            validation_required: spec.validation_required,
            max_repairs: spec.max_repairs,
            repair_count: 0,
            saw_inspect: false,
            saw_mutation: false,
            saw_validation: false,
            saw_external_target: false,
            saw_greenfield_creation: false,
        }
    }

    pub(crate) fn is_active(self) -> bool {
        !matches!(self.kind, ExecutionRecipeKind::None)
    }

    pub(crate) fn needs_validation(self) -> bool {
        self.is_active() && self.validation_required && self.saw_mutation && !self.saw_validation
    }

    pub(crate) fn remaining_repairs(self) -> usize {
        self.max_repairs.saturating_sub(self.repair_count)
    }

    pub(crate) fn phase_summary_line(self) -> String {
        format!(
            "kind={} profile={} target_scope={} phase={} inspect_first={} validation_required={} repair_count={} max_repairs={} saw_inspect={} saw_mutation={} saw_validation={} saw_external_target={} saw_greenfield_creation={}",
            self.kind.as_str(),
            self.profile.as_str(),
            self.target_scope.as_str(),
            self.phase.as_str(),
            self.inspect_first,
            self.validation_required,
            self.repair_count,
            self.max_repairs,
            self.saw_inspect,
            self.saw_mutation,
            self.saw_validation,
            self.saw_external_target,
            self.saw_greenfield_creation
        )
    }

    pub(crate) fn goal_overlay(self) -> Option<String> {
        if !self.is_active() {
            return None;
        }
        Some(format!(
            "[EXECUTION_RECIPE]\nkind={}\nprofile={}\ntarget_scope={}\ncurrent_phase={}\ninspect_first={}\nvalidation_required={}\nrepair_count={}\nmax_repairs={}\nremaining_repairs={}\nobserved_state=saw_inspect:{} saw_mutation:{} saw_validation:{} saw_external_target:{} saw_greenfield_creation:{}\nProfileGuidance:\n- {}\nScopeGuidance:\n- {}\nRules:\n- Collect current state/config evidence before mutating.\n- After any mutating step, include machine-verifiable validation steps.\n- If validation fails after a mutation, use the next round for repair instead of looping blindly.\n",
            self.kind.as_str(),
            self.profile.as_str(),
            self.target_scope.as_str(),
            self.phase.as_str(),
            self.inspect_first,
            self.validation_required,
            self.repair_count,
            self.max_repairs,
            self.remaining_repairs(),
            self.saw_inspect,
            self.saw_mutation,
            self.saw_validation,
            self.saw_external_target,
            self.saw_greenfield_creation,
            self.profile_guidance(),
            self.target_scope_guidance()
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ActionEffect {
    pub(crate) observes: bool,
    pub(crate) mutates: bool,
    pub(crate) validates: bool,
}

impl ActionEffect {
    pub(crate) const fn observe() -> Self {
        Self {
            observes: true,
            mutates: false,
            validates: false,
        }
    }

    pub(crate) const fn mutate() -> Self {
        Self {
            observes: false,
            mutates: true,
            validates: false,
        }
    }

    pub(crate) const fn validate() -> Self {
        Self {
            observes: true,
            mutates: false,
            validates: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ValidationObservation {
    Passed,
    Failed(String),
    Inconclusive,
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn looks_ops_target(text: &str) -> bool {
    contains_any(
        text,
        &[
            "service",
            "systemd",
            "daemon",
            "proxy",
            "server",
            "nginx",
            "sing-box",
            "docker",
            "compose",
            "ssh",
            "network",
            "firewall",
            "config",
            "configuration",
            "服务",
            "代理",
            "服务器",
            "网络",
            "防火墙",
            "配置",
            "重启",
            "systemctl",
        ],
    )
}

fn looks_system_scope(text: &str) -> bool {
    contains_any(
        text,
        &[
            "service",
            "systemd",
            "daemon",
            "proxy",
            "server",
            "nginx",
            "sing-box",
            "docker",
            "compose",
            "ssh",
            "network",
            "firewall",
            "systemctl",
            "服务",
            "代理",
            "服务器",
            "网络",
            "防火墙",
            "重启",
        ],
    )
}

fn looks_config_target(text: &str) -> bool {
    contains_any(
        text,
        &[
            "config.toml",
            "settings",
            "setting",
            "toml",
            "yaml",
            "yml",
            "json",
            "ini",
            "env",
            "feature flag",
            "flag",
            "配置文件",
            "配置项",
            "参数",
            "开关",
            "设置",
            "风险配置",
            "allow_sudo",
            "full_access",
            "api_key",
            "registry_path",
        ],
    )
}

// 自 `detect_execution_recipe` 收紧后，`code_target` 不再单独参与触发判定，
// 但保留此函数以便后续按 profile 维度细化判定（例如未来给"修改代码 + 必须验证"
// 增加更细的门槛）。
#[allow(dead_code)]
fn looks_code_change_target(text: &str) -> bool {
    contains_any(
        text,
        &[
            "bug",
            "feature",
            "compile",
            "build",
            "test",
            "lint",
            "refactor",
            "code",
            "module",
            "function",
            "script",
            "repo",
            "repository",
            "project",
            "workspace",
            "接口",
            "编译",
            "构建",
            "测试",
            "重构",
            "代码",
            "模块",
            "函数",
            "脚本",
            "仓库",
            "项目",
            "工作区",
            "需求",
            "问题",
        ],
    )
}

fn looks_skill_authoring_target(text: &str) -> bool {
    contains_any(
        text,
        &[
            "skill",
            "skills/",
            "interfacemd",
            "interface.md",
            "skill-runner",
            "extension",
            "plugin",
            "tooling capability",
            "技能",
            "新技能",
            "开发skill",
            "写个skill",
            "扩展能力",
            "插件",
        ],
    )
}

fn looks_mutation_request(text: &str) -> bool {
    contains_any(
        text,
        &[
            "add",
            "implement",
            "create",
            "write",
            "develop",
            "build",
            "install",
            "configure",
            "config",
            "setup",
            "set up",
            "fix",
            "repair",
            "deploy",
            "restart",
            "reload",
            "start ",
            "stop ",
            "enable",
            "disable",
            "modify",
            "change",
            "patch",
            "edit",
            "update",
            "switch",
            "replace",
            "migrate",
            "tune",
            "安装",
            "配置",
            "修复",
            "重启",
            "重载",
            "启动",
            "停止",
            "启用",
            "禁用",
            "修改",
            "变更",
            "更新",
            "切换",
            "替换",
            "调整",
            "部署",
            "新增",
            "添加",
            "实现",
            "编写",
            "开发",
            "创建",
        ],
    )
}

fn looks_validation_request(text: &str) -> bool {
    contains_any(
        text,
        &[
            "validate",
            "verification",
            "verify",
            "test",
            "check",
            "confirm",
            "ensure",
            "health",
            "compile",
            "build",
            "lint",
            "work",
            "working",
            "available",
            "reachable",
            "until it works",
            "make sure",
            "验证",
            "测试",
            "检查",
            "确认",
            "确保",
            "编译通过",
            "测试通过",
            "通过测试",
            "可用",
            "连通",
            "跑通",
            "生效",
            "正常",
        ],
    )
}

fn looks_greenfield_scope(text: &str) -> bool {
    contains_any(
        text,
        &[
            "from scratch",
            "greenfield",
            "new project",
            "new repo",
            "new repository",
            "new script",
            "standalone script",
            "独立脚本",
            "新建脚本",
            "新项目",
            "新仓库",
            "从零开始",
            "新建项目",
        ],
    )
}

fn looks_external_workspace_scope(text: &str) -> bool {
    contains_any(
        text,
        &[
            "outside this repo",
            "outside current repo",
            "outside this project",
            "another repo",
            "other repo",
            "other repository",
            "another project",
            "other project",
            "external workspace",
            "另一个仓库",
            "其他仓库",
            "另一个项目",
            "其他项目",
            "仓库外",
            "项目外",
            "别的项目",
            "其他目录",
            "另一个目录",
        ],
    )
}

fn looks_current_repo_scope(text: &str) -> bool {
    contains_any(
        text,
        &[
            "this repo",
            "current repo",
            "this repository",
            "current repository",
            "this project",
            "current project",
            "this workspace",
            "current workspace",
            "当前仓库",
            "这个仓库",
            "当前项目",
            "这个项目",
            "当前工作区",
            "这个工作区",
        ],
    )
}

fn detect_execution_recipe_profile(joined: &str) -> ExecutionRecipeProfile {
    if looks_skill_authoring_target(joined) {
        ExecutionRecipeProfile::SkillAuthoring
    } else if looks_config_target(joined) {
        ExecutionRecipeProfile::ConfigChange
    } else if looks_ops_target(joined) {
        ExecutionRecipeProfile::OpsService
    } else {
        ExecutionRecipeProfile::CodeChange
    }
}

fn detect_execution_recipe_target_scope(
    joined: &str,
    profile: ExecutionRecipeProfile,
) -> ExecutionRecipeTargetScope {
    if looks_greenfield_scope(joined) {
        ExecutionRecipeTargetScope::Greenfield
    } else if looks_external_workspace_scope(joined) {
        ExecutionRecipeTargetScope::ExternalWorkspace
    } else if looks_current_repo_scope(joined) {
        ExecutionRecipeTargetScope::CurrentRepo
    } else if matches!(profile, ExecutionRecipeProfile::SkillAuthoring) {
        ExecutionRecipeTargetScope::CurrentRepo
    } else if matches!(profile, ExecutionRecipeProfile::OpsService) || looks_system_scope(joined) {
        ExecutionRecipeTargetScope::System
    } else if matches!(
        profile,
        ExecutionRecipeProfile::ConfigChange | ExecutionRecipeProfile::CodeChange
    ) {
        ExecutionRecipeTargetScope::CurrentRepo
    } else {
        ExecutionRecipeTargetScope::Unknown
    }
}

fn inherited_clarify_rewrite_request_text(route: &RouteResult) -> Option<String> {
    let prefix = "Continue the previous request that was waiting for clarification:";
    let suffix = "\nUser now provides the missing target/content:";
    let rest = route.resolved_intent.trim().strip_prefix(prefix)?.trim();
    let prior_request = rest
        .split_once(suffix)
        .map(|(left, _)| left)
        .unwrap_or(rest);
    let trimmed = prior_request.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) fn detect_execution_recipe(
    route_result: Option<&RouteResult>,
    goal: &str,
    user_text: &str,
) -> ExecutionRecipeSpec {
    let Some(route) = route_result else {
        return ExecutionRecipeSpec::default();
    };
    if route.needs_clarify || !route.is_execute_gate() {
        return ExecutionRecipeSpec::default();
    }
    if crate::route_reason_is_any_route_contract(&route.route_reason) {
        return ExecutionRecipeSpec::default();
    }
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ExistenceWithPath
            | crate::OutputSemanticKind::ScalarCount
            | crate::OutputSemanticKind::ServiceStatus
    ) {
        return ExecutionRecipeSpec::default();
    }

    let detection_text = {
        if let Some(inherited_request) = inherited_clarify_rewrite_request_text(route) {
            inherited_request
        } else {
            let resolved_intent = route.resolved_intent.trim();
            let user_text = user_text.trim();
            if !resolved_intent.is_empty() && !user_text.is_empty() {
                if resolved_intent == user_text {
                    resolved_intent.to_string()
                } else {
                    format!("{resolved_intent}\n{user_text}")
                }
            } else if !resolved_intent.is_empty() {
                resolved_intent.to_string()
            } else if !user_text.is_empty() {
                user_text.to_string()
            } else {
                goal.trim().to_string()
            }
        }
    };
    // `goal` 在 runtime 里往往是带 memory / recent_execution_context / auto-locator
    // 的大 prompt，不是纯语义意图。recipe 检测如果把整段 goal 拼进来，极易被
    // 历史上下文或结构块里的噪声词误升级成 ops_closed_loop。
    let joined = detection_text.to_ascii_lowercase();
    let has_until_clause = contains_any(&joined, &["until it works", "直到", "跑通", "生效"]);
    let mutation_request = looks_mutation_request(&joined);
    let validation_request = looks_validation_request(&joined);
    let ops_target = looks_ops_target(&joined);
    // 触发条件收紧：要求 mutation 与 (validation | ops_target | until) 同时出现。
    // 单纯的 mutation + code_target（如 "write a small temporary script"）只是一次性
    // 临时改动，不应被识别成需要"修改→验证→修复"闭环的 OpsClosedLoop，否则会
    // 误把走 self_extension 的临时脚本请求 bypass 掉。code_target 仍参与下面的
    // profile 选型，但不再作为触发信号。
    if mutation_request && (validation_request || ops_target || has_until_clause) {
        let profile = detect_execution_recipe_profile(&joined);
        let target_scope = detect_execution_recipe_target_scope(&joined, profile);
        return ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            profile,
            target_scope,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        };
    }
    ExecutionRecipeSpec::default()
}

fn normalized_first_command_word(command: &str) -> Option<String> {
    command
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| {
                    ch.is_whitespace()
                        || matches!(
                            ch,
                            '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
                        )
                })
                .to_ascii_lowercase()
        })
        .find(|token| {
            !token.is_empty()
                && !(token.contains('=')
                    && !token.starts_with("./")
                    && !token.contains('/')
                    && !token.starts_with('-'))
        })
}

fn run_cmd_looks_config_change_validation(command_lower: &str) -> bool {
    command_lower.contains(" check")
        || command_lower.starts_with("check ")
        || command_lower.contains(" validate")
        || command_lower.contains(" verify")
        || command_lower.contains(" reload")
        || command_lower.contains(" systemctl status")
        || command_lower.contains(" systemctl is-active")
        || command_lower.contains(" nginx -t")
        || command_lower.contains(" sing-box check")
        || command_lower.contains(" curl ")
        || command_lower.contains(" wget ")
}

fn run_cmd_looks_code_change_validation(command_lower: &str) -> bool {
    contains_any(
        command_lower,
        &[
            "cargo check",
            "cargo test",
            "cargo clippy",
            "cargo build",
            "cargo run",
            "pytest",
            "python -m pytest",
            "python3 -m pytest",
            "python -m unittest",
            "python3 -m unittest",
            "uv run pytest",
            "uv run python",
            "npm test",
            "npm run test",
            "npm run build",
            "npm run lint",
            "pnpm test",
            "pnpm run test",
            "pnpm run build",
            "pnpm run lint",
            "yarn test",
            "yarn build",
            "yarn lint",
            "bun test",
            "bun run test",
            "bun run build",
            "bun run lint",
            "go test",
            "go build",
            "go run",
            "make test",
            "make check",
            "make build",
            "just test",
            "just check",
            "mvn test",
            "gradle test",
        ],
    )
}

fn run_cmd_looks_skill_authoring_validation(command_lower: &str) -> bool {
    run_cmd_looks_code_change_validation(command_lower)
        || command_lower.contains("skill-runner")
        || command_lower.contains("sync_skill_docs.py")
}

pub(crate) fn validation_satisfies_recipe_profile(
    recipe: ExecutionRecipeRuntimeState,
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> bool {
    let normalized_skill = state.resolve_canonical_skill_name(skill_name);
    match recipe.profile {
        ExecutionRecipeProfile::None | ExecutionRecipeProfile::OpsService => {
            classify_skill_action_effect(state, &normalized_skill, args).validates
        }
        ExecutionRecipeProfile::ConfigChange => match normalized_skill.as_str() {
            "config_guard" => {
                let action = args
                    .get("action")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                args.get("path").is_some()
                    && (action.is_empty() || contains_any(&action, &["validate", "check", "read"]))
            }
            "service_control" | "health_check" | "http_basic" => true,
            "run_cmd" => args
                .get("command")
                .and_then(|value| value.as_str())
                .map(|command| {
                    run_cmd_looks_config_change_validation(&command.to_ascii_lowercase())
                })
                .unwrap_or(false),
            _ => false,
        },
        ExecutionRecipeProfile::CodeChange => match normalized_skill.as_str() {
            "service_control" | "health_check" | "http_basic" => true,
            "run_cmd" => args
                .get("command")
                .and_then(|value| value.as_str())
                .map(|command| run_cmd_looks_code_change_validation(&command.to_ascii_lowercase()))
                .unwrap_or(false),
            _ => false,
        },
        ExecutionRecipeProfile::SkillAuthoring => match normalized_skill.as_str() {
            "run_cmd" => args
                .get("command")
                .and_then(|value| value.as_str())
                .map(|command| {
                    run_cmd_looks_skill_authoring_validation(&command.to_ascii_lowercase())
                })
                .unwrap_or(false),
            _ => false,
        },
    }
}

pub(crate) fn validation_detail_for_recipe(recipe: ExecutionRecipeRuntimeState) -> &'static str {
    match recipe.profile {
        ExecutionRecipeProfile::ConfigChange => {
            "config_change requires post-change validation such as parse/check/reload or effective-state verification"
        }
        ExecutionRecipeProfile::CodeChange => {
            "code_change requires compile/test/build or runtime verification after mutation"
        }
        ExecutionRecipeProfile::SkillAuthoring => {
            "skill_authoring requires integration validation after mutation (for example cargo check/test or extension registration verification)"
        }
        _ => "ops_closed_loop requires a machine-verifiable validation step after mutation",
    }
}

pub(crate) fn target_scope_detail_for_recipe(recipe: ExecutionRecipeRuntimeState) -> &'static str {
    match recipe.target_scope {
        ExecutionRecipeTargetScope::CurrentRepo => {
            "current_repo scope must stay inside the current workspace and should not drift to external absolute paths"
        }
        ExecutionRecipeTargetScope::ExternalWorkspace => {
            "external_workspace scope requires an explicit external path or working directory outside the current workspace"
        }
        ExecutionRecipeTargetScope::Greenfield => {
            "greenfield scope requires creating a new file, directory, or scaffold before verification"
        }
        _ => "execution recipe target scope is misaligned with the planned actions",
    }
}

fn trim_path_like_token(token: &str) -> &str {
    token.trim_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | ':'
            )
    })
}

fn path_candidate_scope(
    candidate: &str,
    workspace_root: &Path,
) -> Option<ExecutionRecipeTargetScope> {
    let candidate = trim_path_like_token(candidate);
    if candidate.is_empty() || candidate.contains("://") {
        return None;
    }
    let path = Path::new(candidate);
    if path.is_absolute() {
        return Some(if path.starts_with(workspace_root) {
            ExecutionRecipeTargetScope::CurrentRepo
        } else {
            ExecutionRecipeTargetScope::ExternalWorkspace
        });
    }
    if candidate.starts_with("./")
        || candidate.starts_with("../")
        || candidate.contains('/')
        || candidate.starts_with("~/")
    {
        return Some(ExecutionRecipeTargetScope::CurrentRepo);
    }
    None
}

fn arg_path_candidates(args: &Value) -> Vec<String> {
    let mut candidates = Vec::new();
    for key in [
        "path",
        "cwd",
        "dir",
        "directory",
        "root",
        "workspace",
        "workspace_root",
        "output_path",
        "target_path",
    ] {
        if let Some(value) = args.get(key).and_then(|value| value.as_str()) {
            let trimmed = trim_path_like_token(value);
            if !trimmed.is_empty() {
                candidates.push(trimmed.to_string());
            }
        }
    }
    candidates
}

fn run_cmd_path_candidates(args: &Value) -> Vec<String> {
    let mut candidates = arg_path_candidates(args);
    let Some(command) = args.get("command").and_then(|value| value.as_str()) else {
        return candidates;
    };
    let mut expect_cd_target = false;
    for raw_token in command.split_whitespace() {
        let token = trim_path_like_token(raw_token);
        if token.is_empty() {
            continue;
        }
        if expect_cd_target {
            candidates.push(token.to_string());
            expect_cd_target = false;
            continue;
        }
        if matches!(token, "cd" | "pushd") {
            expect_cd_target = true;
            continue;
        }
        if token.starts_with('/')
            || token.starts_with("./")
            || token.starts_with("../")
            || token.starts_with("~/")
        {
            candidates.push(token.to_string());
        }
    }
    candidates
}

fn action_path_candidates(state: &AppState, skill_name: &str, args: &Value) -> Vec<String> {
    let normalized_skill = state.resolve_canonical_skill_name(skill_name);
    match normalized_skill.as_str() {
        "run_cmd" => run_cmd_path_candidates(args),
        _ => arg_path_candidates(args),
    }
}

fn run_cmd_looks_greenfield_creation(command_lower: &str) -> bool {
    contains_any(
        command_lower,
        &[
            "cargo new",
            "cargo init",
            "npm create",
            "pnpm create",
            "yarn create",
            "bun create",
            "go mod init",
            "python -m venv",
            "python3 -m venv",
            "uv init",
            "mkdir ",
            "mkdir -p",
            "touch ",
        ],
    )
}

pub(crate) fn action_targets_external_workspace(
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> bool {
    action_path_candidates(state, skill_name, args)
        .into_iter()
        .any(|candidate| {
            matches!(
                path_candidate_scope(&candidate, &state.skill_rt.workspace_root),
                Some(ExecutionRecipeTargetScope::ExternalWorkspace)
            )
        })
}

pub(crate) fn action_conflicts_with_recipe_target_scope(
    recipe: ExecutionRecipeRuntimeState,
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> bool {
    match recipe.target_scope {
        ExecutionRecipeTargetScope::CurrentRepo => {
            action_targets_external_workspace(state, skill_name, args)
        }
        ExecutionRecipeTargetScope::ExternalWorkspace => {
            let candidates = action_path_candidates(state, skill_name, args);
            !candidates.is_empty()
                && candidates.into_iter().any(|candidate| {
                    matches!(
                        path_candidate_scope(&candidate, &state.skill_rt.workspace_root),
                        Some(ExecutionRecipeTargetScope::CurrentRepo)
                    )
                })
        }
        _ => false,
    }
}

pub(crate) fn action_satisfies_greenfield_creation(
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> bool {
    match state.resolve_canonical_skill_name(skill_name).as_str() {
        "write_file" | "make_dir" => true,
        "run_cmd" => args
            .get("command")
            .and_then(|value| value.as_str())
            .map(|command| {
                let lower = command.to_ascii_lowercase();
                run_cmd_looks_greenfield_creation(&lower)
                    || run_cmd_has_explicit_write_marker(command)
            })
            .unwrap_or(false),
        _ => false,
    }
}

pub(crate) fn apply_target_scope_progress(
    recipe: &mut ExecutionRecipeRuntimeState,
    state: &AppState,
    skill_name: &str,
    args: &Value,
    action_succeeded: bool,
) {
    if !recipe.is_active() {
        return;
    }
    if matches!(
        recipe.target_scope,
        ExecutionRecipeTargetScope::ExternalWorkspace
    ) && action_targets_external_workspace(state, skill_name, args)
    {
        recipe.saw_external_target = true;
    }
    if action_succeeded
        && matches!(recipe.target_scope, ExecutionRecipeTargetScope::Greenfield)
        && action_satisfies_greenfield_creation(state, skill_name, args)
    {
        recipe.saw_greenfield_creation = true;
    }
}

fn run_cmd_has_explicit_write_marker(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first_word = normalized_first_command_word(command);
    command.contains('>')
        || lower.contains(" tee ")
        || lower.starts_with("tee ")
        || lower.contains(" sed -i")
        || lower.starts_with("sed -i")
        || lower.contains(" perl -pi")
        || lower.starts_with("perl -pi")
        || lower.contains("systemctl start")
        || lower.contains("systemctl stop")
        || lower.contains("systemctl restart")
        || lower.contains("systemctl reload")
        || lower.contains("systemctl enable")
        || lower.contains("systemctl disable")
        || lower.contains(" service ")
            && contains_any(
                &lower,
                &[
                    " start", " stop", " restart", " reload", " enable", " disable",
                ],
            )
        || matches!(
            first_word.as_deref(),
            Some(
                "cp" | "mv"
                    | "rm"
                    | "mkdir"
                    | "touch"
                    | "truncate"
                    | "install"
                    | "dd"
                    | "chmod"
                    | "chown"
                    | "ln"
                    | "launchctl"
            )
        )
}

fn shell_contains_command_invocation(command_lower: &str, word: &str) -> bool {
    command_lower.starts_with(&format!("{word} "))
        || command_lower.contains(&format!("\n{word} "))
        || ["&&", ";", "|", "||", "("]
            .into_iter()
            .any(|prefix| command_lower.contains(&format!("{prefix} {word} ")))
}

fn run_cmd_looks_validation(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first_word = normalized_first_command_word(command);
    contains_any(
        &lower,
        &[
            " check",
            "check ",
            " test",
            "test ",
            " verify",
            "verify ",
            " validate",
            "validate ",
            "cargo check",
            "cargo test",
            "cargo clippy",
            "cargo build",
            "cargo run",
            "pytest",
            "python -m pytest",
            "python3 -m pytest",
            "python -m unittest",
            "python3 -m unittest",
            "uv run pytest",
            "uv run python",
            "npm run test",
            "npm run build",
            "npm run lint",
            "pnpm run test",
            "pnpm run build",
            "pnpm run lint",
            "yarn test",
            "yarn build",
            "yarn lint",
            "bun test",
            "bun run test",
            "bun run build",
            "bun run lint",
            "go test",
            "go build",
            "go run",
            "make test",
            "make check",
            "make build",
            "just test",
            "just check",
            "mvn test",
            "gradle test",
            "systemctl status",
            "systemctl is-active",
            " service status",
            "nginx -t",
            "sing-box check",
            "docker ps",
            "docker inspect",
            "docker compose ps",
            "kubectl get",
            "kubectl describe",
            "journalctl",
            "health",
            "validation_passed",
            "validation_failed",
        ],
    ) || matches!(
        first_word.as_deref(),
        Some("curl" | "wget" | "nc" | "ss" | "lsof")
    ) || ["curl", "wget", "nc", "ss", "lsof"]
        .into_iter()
        .any(|word| shell_contains_command_invocation(&lower, word))
}

fn combined_action_effect(mutates: bool, validates: bool) -> ActionEffect {
    if !mutates && !validates {
        return ActionEffect::observe();
    }
    ActionEffect {
        observes: validates,
        mutates,
        validates,
    }
}

fn run_cmd_action_effect(command: &str) -> ActionEffect {
    let mutates = run_cmd_has_explicit_write_marker(command);
    let validates = run_cmd_looks_validation(command);
    if command.trim().is_empty() {
        ActionEffect::default()
    } else {
        combined_action_effect(mutates, validates)
    }
}

pub(crate) fn split_run_cmd_mutation_and_validation(command: &str) -> Option<(String, String)> {
    let effect = run_cmd_action_effect(command);
    if !effect.mutates || !effect.validates {
        return None;
    }
    let bytes = command.as_bytes();
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte != b'&' {
            continue;
        }
        let prev = idx.checked_sub(1).and_then(|pos| bytes.get(pos)).copied();
        let next = bytes.get(idx + 1).copied();
        if prev == Some(b'&')
            || next == Some(b'&')
            || prev == Some(b'>')
            || next == Some(b'>')
            || next.is_some_and(|value| value.is_ascii_digit())
        {
            continue;
        }
        let mutate_part = command[..=idx].trim();
        let validate_part = command[idx + 1..].trim();
        if mutate_part.is_empty() || validate_part.is_empty() {
            continue;
        }
        if run_cmd_has_explicit_write_marker(mutate_part) && run_cmd_looks_validation(validate_part)
        {
            return Some((mutate_part.to_string(), validate_part.to_string()));
        }
    }
    None
}

pub(crate) fn classify_skill_action_effect(
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> ActionEffect {
    match state.resolve_canonical_skill_name(skill_name).as_str() {
        "read_file" | "list_dir" | "fs_search" | "git_basic" | "db_basic" | "process_basic"
        | "log_analyze" => ActionEffect::observe(),
        "write_file" | "remove_file" | "make_dir" | "package_manager" | "install_module" => {
            ActionEffect::mutate()
        }
        "health_check" | "http_basic" => ActionEffect::validate(),
        "system_basic" => {
            let action = args
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if contains_any(&action, &["check", "health"]) {
                ActionEffect::validate()
            } else if !action.is_empty() {
                ActionEffect::observe()
            } else {
                ActionEffect::default()
            }
        }
        "config_guard" => {
            let action = args
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if args.get("key").is_some() || args.get("value").is_some() {
                ActionEffect::mutate()
            } else if contains_any(&action, &["patch", "write", "set", "update", "modify"]) {
                ActionEffect::mutate()
            } else if contains_any(&action, &["validate", "check"]) {
                ActionEffect::validate()
            } else {
                ActionEffect::observe()
            }
        }
        "service_control" => {
            let action = args
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if contains_any(
                &action,
                &["start", "stop", "restart", "reload", "enable", "disable"],
            ) {
                ActionEffect::mutate()
            } else if contains_any(&action, &["status", "verify"]) {
                ActionEffect::validate()
            } else if !action.is_empty() {
                ActionEffect::observe()
            } else {
                ActionEffect::default()
            }
        }
        "run_cmd" => {
            let command = args
                .get("command")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            run_cmd_action_effect(command)
        }
        _ => ActionEffect::default(),
    }
}

fn service_state_is_healthy(state: &str) -> bool {
    let lower = state.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "active" | "running" | "active (running)" | "started" | "healthy" | "ok"
    ) || (lower.contains("active") && lower.contains("running"))
}

fn service_state_looks_failed(state: &str) -> bool {
    let lower = state.trim().to_ascii_lowercase();
    contains_any(
        &lower,
        &[
            "inactive",
            "stopped",
            "failed",
            "dead",
            "not running",
            "unhealthy",
            "unknown",
            "error",
        ],
    )
}

fn assess_service_control_validation(output: &str) -> ValidationObservation {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return ValidationObservation::Inconclusive;
    };
    if value.get("status").and_then(|v| v.as_str()) == Some("error") {
        let detail = value
            .get("failure_reason")
            .and_then(|v| v.as_str())
            .filter(|text| !text.trim().is_empty())
            .or_else(|| value.get("summary").and_then(|v| v.as_str()))
            .unwrap_or("service_control reported an error");
        return ValidationObservation::Failed(detail.to_string());
    }
    if value
        .get("verified")
        .and_then(|v| v.as_bool())
        .is_some_and(|verified| !verified)
    {
        let detail = value
            .get("failure_reason")
            .and_then(|v| v.as_str())
            .filter(|text| !text.trim().is_empty())
            .or_else(|| value.get("summary").and_then(|v| v.as_str()))
            .or_else(|| value.get("post_state").and_then(|v| v.as_str()))
            .or_else(|| value.get("pre_state").and_then(|v| v.as_str()))
            .unwrap_or("service verification did not pass");
        return ValidationObservation::Failed(detail.to_string());
    }
    let state = value
        .get("post_state")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("pre_state").and_then(|v| v.as_str()))
        .unwrap_or_default();
    if !state.is_empty() {
        if service_state_is_healthy(state) {
            return ValidationObservation::Passed;
        }
        if service_state_looks_failed(state) {
            return ValidationObservation::Failed(state.to_string());
        }
    }
    if value
        .get("verified")
        .and_then(|v| v.as_bool())
        .is_some_and(|verified| verified)
    {
        return ValidationObservation::Passed;
    }
    ValidationObservation::Inconclusive
}

fn assess_health_check_validation(output: &str) -> ValidationObservation {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return ValidationObservation::Inconclusive;
    };
    let clawd_count = value.get("clawd_process_count").and_then(|v| v.as_u64());
    let telegramd_count = value
        .get("telegramd_process_count")
        .and_then(|v| v.as_u64());
    let clawd_port_open = value
        .get("clawd_health_port_open")
        .and_then(|v| v.as_bool());
    if clawd_count == Some(0) || clawd_port_open == Some(false) {
        return ValidationObservation::Failed("clawd health check is not passing yet".to_string());
    }
    if telegramd_count == Some(0) {
        return ValidationObservation::Failed("telegramd is not running".to_string());
    }
    if clawd_count.is_some() && clawd_port_open.is_some() {
        return ValidationObservation::Passed;
    }
    ValidationObservation::Inconclusive
}

fn has_strong_run_cmd_success_marker(command: &str, output_lower: &str) -> bool {
    let first_word = normalized_first_command_word(command);
    output_lower.lines().any(|line| {
        let trimmed = line.trim();
        matches!(
            trimmed,
            "active" | "running" | "ok" | "healthy" | "success" | "ready" | "passed"
        ) || trimmed == "status=200"
            || trimmed == "status=204"
            || trimmed == "status=301"
            || trimmed == "status=302"
            || trimmed == "validation_passed"
            || trimmed.contains("syntax is ok")
            || trimmed.contains("test is successful")
            || trimmed.contains("configuration ok")
            || trimmed.contains("configuration file") && trimmed.contains("test is successful")
    }) || (output_lower.trim().is_empty()
        && matches!(
            first_word.as_deref(),
            Some("curl" | "wget" | "nc" | "ss" | "lsof")
        ))
}

fn has_strong_run_cmd_failure_marker(output_lower: &str) -> bool {
    contains_any(
        output_lower,
        &[
            "inactive",
            "stopped",
            "failed",
            "not running",
            "unhealthy",
            "validation_failed",
            "connection refused",
            "connection reset",
            "timed out",
            "timeout",
            "unreachable",
            "permission denied",
            "no such host",
            "could not",
            "syntax error",
            "panic",
            "error:",
            "not ok",
        ],
    )
}

fn first_nonempty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn second_nonempty_line(text: &str) -> Option<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .nth(1)
}

fn output_is_exit_zero_sentinel(output: &str) -> bool {
    output
        .trim()
        .to_ascii_lowercase()
        .starts_with("exit=0 command=")
}

fn assess_systemctl_is_active_validation(output: &str) -> ValidationObservation {
    match first_nonempty_line(output)
        .map(|line| line.to_ascii_lowercase())
        .as_deref()
    {
        Some("active") => ValidationObservation::Passed,
        Some("inactive" | "failed" | "deactivating" | "activating") => {
            ValidationObservation::Failed(output.trim().to_string())
        }
        _ => ValidationObservation::Inconclusive,
    }
}

fn assess_service_status_validation(output: &str) -> ValidationObservation {
    let lower = output.trim().to_ascii_lowercase();
    if lower.contains("active: active (running)")
        || lower.contains(" is running")
        || lower.contains("start/running")
    {
        return ValidationObservation::Passed;
    }
    if lower.contains("active: inactive")
        || lower.contains("active: failed")
        || lower.contains(" is not running")
        || lower.contains("stop/waiting")
        || lower.contains("inactive (dead)")
    {
        return ValidationObservation::Failed(output.trim().to_string());
    }
    ValidationObservation::Inconclusive
}

fn assess_nginx_test_validation(output: &str) -> ValidationObservation {
    let lower = output.trim().to_ascii_lowercase();
    if lower.contains("syntax is ok") && lower.contains("test is successful") {
        return ValidationObservation::Passed;
    }
    if has_strong_run_cmd_failure_marker(&lower) {
        return ValidationObservation::Failed(output.trim().to_string());
    }
    ValidationObservation::Inconclusive
}

fn assess_sing_box_check_validation(output: &str) -> ValidationObservation {
    let lower = output.trim().to_ascii_lowercase();
    if output_is_exit_zero_sentinel(output)
        || lower.contains("configuration ok")
        || lower.contains("config ok")
        || lower.contains("check passed")
        || lower.contains("syntax is ok")
    {
        return ValidationObservation::Passed;
    }
    if has_strong_run_cmd_failure_marker(&lower)
        || lower.contains("decode config")
        || lower.contains("parse config")
    {
        return ValidationObservation::Failed(output.trim().to_string());
    }
    ValidationObservation::Inconclusive
}

fn assess_http_probe_validation(command: &str, output: &str) -> ValidationObservation {
    let lower = output.trim().to_ascii_lowercase();
    if lower.contains("validation_passed") {
        return ValidationObservation::Passed;
    }
    if lower.contains("validation_failed") {
        return ValidationObservation::Failed(output.trim().to_string());
    }
    if let Some(status_line) =
        first_nonempty_line(output).and_then(|line| line.strip_prefix("status="))
    {
        if let Ok(code) = status_line.trim().parse::<u16>() {
            return match code {
                200..=399 => ValidationObservation::Passed,
                _ => ValidationObservation::Failed(format!("http returned status={code}")),
            };
        }
    }
    if output_is_exit_zero_sentinel(output)
        && normalized_first_command_word(command)
            .as_deref()
            .is_some_and(|cmd| matches!(cmd, "curl" | "wget" | "nc"))
    {
        return ValidationObservation::Passed;
    }
    if has_strong_run_cmd_failure_marker(&lower) {
        return ValidationObservation::Failed(output.trim().to_string());
    }
    if command.to_ascii_lowercase().contains("grep") && !output.trim().is_empty() {
        return ValidationObservation::Passed;
    }
    ValidationObservation::Inconclusive
}

fn assess_socket_listing_validation(output: &str) -> ValidationObservation {
    let first = first_nonempty_line(output);
    let second = second_nonempty_line(output);
    match (first, second) {
        (Some(_header), Some(_row)) => ValidationObservation::Passed,
        (Some(_header), None) => ValidationObservation::Failed(
            "validation command returned no matching rows".to_string(),
        ),
        _ => ValidationObservation::Inconclusive,
    }
}

fn assess_run_cmd_validation(command: &str, output: &str) -> ValidationObservation {
    if !run_cmd_looks_validation(command) {
        return ValidationObservation::Inconclusive;
    }
    let command_lower = command.trim().to_ascii_lowercase();
    if command_lower.contains("systemctl is-active") {
        return assess_systemctl_is_active_validation(output);
    }
    if command_lower.contains("systemctl status")
        || command_lower.contains(" service status")
        || command_lower.contains("service --status-all")
    {
        return assess_service_status_validation(output);
    }
    if command_lower.contains("nginx -t") {
        return assess_nginx_test_validation(output);
    }
    if command_lower.contains("sing-box check") {
        return assess_sing_box_check_validation(output);
    }
    if normalized_first_command_word(command)
        .as_deref()
        .is_some_and(|cmd| matches!(cmd, "curl" | "wget" | "nc"))
    {
        return assess_http_probe_validation(command, output);
    }
    if normalized_first_command_word(command)
        .as_deref()
        .is_some_and(|cmd| matches!(cmd, "ss" | "lsof"))
    {
        return assess_socket_listing_validation(output);
    }
    let output_lower = output.trim().to_ascii_lowercase();
    let has_success = has_strong_run_cmd_success_marker(command, &output_lower);
    let has_failure = has_strong_run_cmd_failure_marker(&output_lower);
    match (has_success, has_failure) {
        (true, false) => ValidationObservation::Passed,
        (false, true) => ValidationObservation::Failed(output.trim().to_string()),
        _ => ValidationObservation::Inconclusive,
    }
}

fn assess_system_basic_validation(args: &Value, output: &str) -> ValidationObservation {
    let action = args
        .get("action")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if action == "diagnose_runtime" {
        return assess_health_check_validation(output);
    }
    ValidationObservation::Inconclusive
}

fn assess_http_basic_validation(args: &Value, output: &str) -> ValidationObservation {
    let status_code = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .and_then(|line| line.strip_prefix("status="))
        .and_then(|digits| digits.trim().parse::<u16>().ok());
    match status_code {
        Some(200..=299) => {
            let expected = args
                .get("expect_contains")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if let Some(expected) = expected {
                let body = output.lines().skip(1).collect::<Vec<_>>().join("\n");
                if body.contains(expected) {
                    ValidationObservation::Passed
                } else {
                    ValidationObservation::Failed(format!(
                        "http response missing expected text={expected}"
                    ))
                }
            } else {
                ValidationObservation::Passed
            }
        }
        Some(code) => ValidationObservation::Failed(format!("http returned status={code}")),
        None => ValidationObservation::Inconclusive,
    }
}

pub(crate) fn assess_validation_output(
    state: &AppState,
    skill_name: &str,
    args: &Value,
    output: &str,
) -> ValidationObservation {
    match state.resolve_canonical_skill_name(skill_name).as_str() {
        "service_control" => assess_service_control_validation(output),
        "health_check" => assess_health_check_validation(output),
        "http_basic" => assess_http_basic_validation(args, output),
        "system_basic" => assess_system_basic_validation(args, output),
        "run_cmd" => args
            .get("command")
            .and_then(|value| value.as_str())
            .map(|command| assess_run_cmd_validation(command, output))
            .unwrap_or(ValidationObservation::Inconclusive),
        _ => ValidationObservation::Inconclusive,
    }
}

pub(crate) fn stop_signal_for_validation_failure(
    state: &ExecutionRecipeRuntimeState,
) -> &'static str {
    if state.is_active() && state.repair_count > state.max_repairs {
        "recipe_repair_budget_exhausted"
    } else {
        "recoverable_failure_continue_round"
    }
}

pub(crate) fn effective_action_effect_for_recipe(
    state: ExecutionRecipeRuntimeState,
    effect: ActionEffect,
) -> ActionEffect {
    if state.is_active() && effect.validates && !effect.mutates && !state.saw_mutation {
        return ActionEffect::observe();
    }
    effect
}

pub(crate) fn apply_action_effect_success(
    state: &mut ExecutionRecipeRuntimeState,
    effect: ActionEffect,
) {
    if !state.is_active() {
        return;
    }
    if effect.observes {
        state.saw_inspect = true;
    }
    if effect.mutates {
        state.saw_mutation = true;
        state.saw_validation = false;
    }
    if effect.validates && state.saw_mutation {
        state.saw_validation = true;
        state.phase = ExecutionRecipePhase::Done;
        return;
    }
    if effect.mutates {
        state.phase = ExecutionRecipePhase::Validate;
        return;
    }
    if matches!(state.phase, ExecutionRecipePhase::Inspect) && state.saw_inspect {
        state.phase = ExecutionRecipePhase::Apply;
    }
}

pub(crate) fn apply_action_effect_failure(
    state: &mut ExecutionRecipeRuntimeState,
    effect: ActionEffect,
) {
    if !state.is_active() {
        return;
    }
    if effect.observes {
        state.saw_inspect = true;
        if matches!(state.phase, ExecutionRecipePhase::Inspect) && !state.saw_mutation {
            state.phase = ExecutionRecipePhase::Apply;
        }
    }
    if effect.validates && state.saw_mutation && !state.saw_validation {
        state.repair_count += 1;
        state.phase = ExecutionRecipePhase::Repair;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_action_effect_failure, apply_action_effect_success, assess_validation_output,
        classify_skill_action_effect, detect_execution_recipe, effective_action_effect_for_recipe,
        stop_signal_for_validation_failure, ActionEffect, ExecutionRecipeKind,
        ExecutionRecipePhase, ExecutionRecipeProfile, ExecutionRecipeRuntimeState,
        ExecutionRecipeSpec, ExecutionRecipeTargetScope, ValidationObservation,
    };
    use crate::{
        AgentRuntimeConfig, AppState, RouteResult, RoutedMode, ScheduleKind, SkillViewsSnapshot,
        ToolsPolicy, DEFAULT_AGENT_ID,
    };
    use claw_core::config::{AgentConfig, ToolsConfig};
    use serde_json::json;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, RwLock};

    fn test_state() -> AppState {
        let agents_by_id = HashMap::from([(
            DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            core: crate::CoreServices {
                agents_by_id: Arc::new(agents_by_id),
                skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                    registry: None,
                    skills_list: Arc::new(HashSet::new()),
                }))),
                ..crate::CoreServices::test_default()
            },
            skill_rt: crate::SkillRuntime {
                locator_scan_max_files: 200,
                tools_policy: Arc::new(
                    ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
                ),
                ..crate::SkillRuntime::test_default()
            },
            policy: crate::PolicyConfig::test_default(),
            worker: crate::WorkerConfig::test_default(),
            metrics: crate::TaskMetricsRegistry::default(),
            channels: crate::ChannelConfig::default(),
            reload_ctx: crate::ReloadContext::default(),
            ask_states: crate::AskStateRegistry::default(),
        }
    }

    fn route_result(mode: RoutedMode, resolved_intent: &str) -> RouteResult {
        RouteResult {
            routed_mode: mode,
            ask_mode: crate::AskMode::from_routed_mode(mode),
            resolved_intent: resolved_intent.to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        }
    }

    #[test]
    fn detect_ops_closed_loop_for_configure_and_validate_request() {
        let route = route_result(
            RoutedMode::Act,
            "configure sing-box and verify the proxy works",
        );
        let spec = detect_execution_recipe(
            Some(&route),
            "configure sing-box and verify the proxy works",
            "configure sing-box and verify the proxy works",
        );
        assert_eq!(spec.kind, ExecutionRecipeKind::OpsClosedLoop);
        assert_eq!(spec.profile, ExecutionRecipeProfile::OpsService);
        assert_eq!(spec.target_scope, ExecutionRecipeTargetScope::System);
        assert!(spec.inspect_first);
        assert!(spec.validation_required);
    }

    #[test]
    fn read_only_request_does_not_trigger_recipe() {
        let route = route_result(RoutedMode::Act, "check current working directory");
        let spec = detect_execution_recipe(Some(&route), "check current working directory", "pwd");
        assert_eq!(spec.kind, ExecutionRecipeKind::None);
    }

    #[test]
    fn route_contract_scalar_extract_does_not_trigger_recipe() {
        let mut route = route_result(
            RoutedMode::Act,
            "读取 /home/guagua/rustclaw/configs/config.toml 里的 tools.allow_sudo，只输出值",
        );
        route.route_reason =
            "route_contract:generic_explicit_path_scalar_extract".to_string();
        let spec =
            detect_execution_recipe(Some(&route), &route.resolved_intent, &route.resolved_intent);
        assert_eq!(spec.kind, ExecutionRecipeKind::None);
    }

    #[test]
    fn detect_config_change_profile_for_repo_config_request() {
        let route = route_result(
            RoutedMode::Act,
            "update configs/config.toml and verify the new setting takes effect in this repo",
        );
        let spec = detect_execution_recipe(
            Some(&route),
            "update configs/config.toml and verify the new setting takes effect in this repo",
            "把 configs/config.toml 里的设置改掉并验证在这个项目里生效",
        );
        assert_eq!(spec.kind, ExecutionRecipeKind::OpsClosedLoop);
        assert_eq!(spec.profile, ExecutionRecipeProfile::ConfigChange);
        assert_eq!(spec.target_scope, ExecutionRecipeTargetScope::CurrentRepo);
    }

    #[test]
    fn detect_code_change_profile_for_current_repo_bugfix() {
        let route = route_result(
            RoutedMode::Act,
            "fix the compile error in this repo and run tests until it works",
        );
        let spec = detect_execution_recipe(
            Some(&route),
            "fix the compile error in this repo and run tests until it works",
            "修复这个仓库里的编译错误并跑测试直到通过",
        );
        assert_eq!(spec.kind, ExecutionRecipeKind::OpsClosedLoop);
        assert_eq!(spec.profile, ExecutionRecipeProfile::CodeChange);
        assert_eq!(spec.target_scope, ExecutionRecipeTargetScope::CurrentRepo);
    }

    #[test]
    fn detect_skill_authoring_profile_for_explicit_skill_request() {
        let route = route_result(
            RoutedMode::Act,
            "develop a new skill in this repo and verify it works",
        );
        let spec = detect_execution_recipe(
            Some(&route),
            "develop a new skill in this repo and verify it works",
            "在这个仓库里开发一个新 skill，并验证它能工作",
        );
        assert_eq!(spec.kind, ExecutionRecipeKind::OpsClosedLoop);
        assert_eq!(spec.profile, ExecutionRecipeProfile::SkillAuthoring);
        assert_eq!(spec.target_scope, ExecutionRecipeTargetScope::CurrentRepo);
    }

    #[test]
    fn detect_greenfield_scope_for_new_script_request() {
        let route = route_result(
            RoutedMode::Act,
            "create a new standalone script from scratch and test it until it works",
        );
        let spec = detect_execution_recipe(
            Some(&route),
            "create a new standalone script from scratch and test it until it works",
            "从零开始写一个独立脚本并测试直到通过",
        );
        assert_eq!(spec.kind, ExecutionRecipeKind::OpsClosedLoop);
        assert_eq!(spec.profile, ExecutionRecipeProfile::CodeChange);
        assert_eq!(spec.target_scope, ExecutionRecipeTargetScope::Greenfield);
    }

    #[test]
    fn clarify_rewrite_read_request_ignores_noisy_goal_when_detecting_recipe() {
        let route = route_result(
            RoutedMode::Act,
            "Continue the previous request that was waiting for clarification: 看看那个模型日志最后 5 行\nUser now provides the missing target/content: scripts/nl_tests/fixtures/device_local/logs/model_io.log",
        );
        let noisy_goal = "### RECENT_EXECUTION_CONTEXT\nconfigs/config.toml verify restart until it works\n\n[AUTO_LOCATOR]\nResolved concrete path from default locator directory: /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/logs/model_io.log";
        let spec = detect_execution_recipe(
            Some(&route),
            noisy_goal,
            "scripts/nl_tests/fixtures/device_local/logs/model_io.log",
        );
        assert_eq!(spec.kind, ExecutionRecipeKind::None);
    }

    #[test]
    fn existence_semantic_kind_short_circuits_recipe_detection() {
        let mut route = route_result(
            RoutedMode::Act,
            "Continue the previous request that was waiting for clarification: 看看那个重启脚本在不在\nUser now provides the missing target/content: restart_clawd_latest.sh",
        );
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        let spec = detect_execution_recipe(
            Some(&route),
            "restart service and verify until it works",
            "restart_clawd_latest.sh",
        );
        assert_eq!(spec.kind, ExecutionRecipeKind::None);
    }

    #[test]
    fn scalar_count_semantic_kind_short_circuits_recipe_detection() {
        let mut route = route_result(RoutedMode::Act, "数一下那个目录里有多少个文件");
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let spec = detect_execution_recipe(Some(&route), "", "数一下那个目录里有多少个文件");
        assert_eq!(spec.kind, ExecutionRecipeKind::None);
    }

    #[test]
    fn goal_overlay_includes_code_change_guidance_for_current_repo() {
        let overlay = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            profile: ExecutionRecipeProfile::CodeChange,
            target_scope: ExecutionRecipeTargetScope::CurrentRepo,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
        })
        .goal_overlay()
        .expect("overlay");
        assert!(overlay.contains("profile=code_change"));
        assert!(overlay.contains("target_scope=current_repo"));
        assert!(overlay.contains("compile/test/lint/runtime evidence"));
        assert!(overlay.contains("current repository/workspace"));
    }

    #[test]
    fn goal_overlay_includes_skill_authoring_and_greenfield_guidance() {
        let overlay = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            profile: ExecutionRecipeProfile::SkillAuthoring,
            target_scope: ExecutionRecipeTargetScope::Greenfield,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
        })
        .goal_overlay()
        .expect("overlay");
        assert!(overlay.contains("profile=skill_authoring"));
        assert!(overlay.contains("target_scope=greenfield"));
        assert!(overlay.contains("reusable skill/extension"));
        assert!(overlay.contains("minimal new files or scaffold"));
    }

    #[test]
    fn classify_run_cmd_restart_as_mutation() {
        let state = test_state();
        let effect = classify_skill_action_effect(
            &state,
            "run_cmd",
            &json!({"command":"systemctl restart sing-box"}),
        );
        assert!(effect.mutates);
        assert!(!effect.validates);
    }

    #[test]
    fn classify_run_cmd_combined_mutation_and_validation() {
        let state = test_state();
        let effect = classify_skill_action_effect(
            &state,
            "run_cmd",
            &json!({"command":"cd /tmp/demo && python3 -m http.server 65429 --bind 127.0.0.1 > /tmp/http.log 2>&1 & sleep 2 && curl -s http://127.0.0.1:65429/ | grep -q ops-demo-ok && echo VALIDATION_PASSED || echo VALIDATION_FAILED"}),
        );
        assert!(effect.mutates);
        assert!(effect.validates);
    }

    #[test]
    fn split_combined_run_cmd_into_mutate_and_validate_parts() {
        let command = "cd /tmp/demo && nohup python3 -m http.server 65429 --bind 127.0.0.1 > /tmp/http.log 2>&1 & sleep 2 && curl -s http://127.0.0.1:65429/ | grep -q ops-demo-ok && echo VALIDATION_PASSED || echo VALIDATION_FAILED";
        let (mutate_part, validate_part) =
            super::split_run_cmd_mutation_and_validation(command).expect("split combined command");
        assert_eq!(
            mutate_part,
            "cd /tmp/demo && nohup python3 -m http.server 65429 --bind 127.0.0.1 > /tmp/http.log 2>&1 &"
        );
        assert_eq!(
            validate_part,
            "sleep 2 && curl -s http://127.0.0.1:65429/ | grep -q ops-demo-ok && echo VALIDATION_PASSED || echo VALIDATION_FAILED"
        );
    }

    #[test]
    fn classify_service_status_as_validation() {
        let state = test_state();
        let effect = classify_skill_action_effect(
            &state,
            "service_control",
            &json!({"action":"status","target":"sing-box"}),
        );
        assert!(effect.observes);
        assert!(effect.validates);
    }

    #[test]
    fn validation_failure_moves_recipe_to_repair() {
        let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        });
        apply_action_effect_success(&mut recipe, ActionEffect::observe());
        apply_action_effect_success(&mut recipe, ActionEffect::mutate());
        assert_eq!(recipe.phase, ExecutionRecipePhase::Validate);
        apply_action_effect_failure(&mut recipe, ActionEffect::validate());
        assert_eq!(recipe.phase, ExecutionRecipePhase::Repair);
        assert_eq!(recipe.repair_count, 1);
    }

    #[test]
    fn combined_mutate_and_validate_marks_recipe_done() {
        let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        });
        apply_action_effect_success(&mut recipe, ActionEffect::observe());
        apply_action_effect_success(
            &mut recipe,
            ActionEffect {
                observes: true,
                mutates: true,
                validates: true,
            },
        );
        assert_eq!(recipe.phase, ExecutionRecipePhase::Done);
        assert!(recipe.saw_validation);
    }

    #[test]
    fn service_control_stopped_status_is_validation_failure() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "service_control",
            &json!({"action":"status","target":"telegramd"}),
            r#"{"status":"ok","service_name":"telegramd","requested_action":"status","pre_state":"telegramd=stopped","post_state":"telegramd=stopped","verified":true,"summary":"Status: telegramd=stopped"}"#,
        );
        assert!(matches!(observation, ValidationObservation::Failed(_)));
    }

    #[test]
    fn service_control_verify_running_is_validation_pass() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "service_control",
            &json!({"action":"verify","target":"telegramd"}),
            r#"{"status":"ok","service_name":"telegramd","requested_action":"verify","post_state":"running","verified":true,"summary":"Verify: running"}"#,
        );
        assert_eq!(observation, ValidationObservation::Passed);
    }

    #[test]
    fn health_check_with_closed_port_is_validation_failure() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "health_check",
            &json!({}),
            r#"{"clawd_process_count":1,"telegramd_process_count":1,"clawd_health_port_open":false}"#,
        );
        assert!(matches!(observation, ValidationObservation::Failed(_)));
    }

    #[test]
    fn run_cmd_active_output_is_validation_pass() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "run_cmd",
            &json!({"command":"systemctl is-active sing-box"}),
            "active\n",
        );
        assert_eq!(observation, ValidationObservation::Passed);
    }

    #[test]
    fn run_cmd_inactive_output_is_validation_failure() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "run_cmd",
            &json!({"command":"systemctl status sing-box"}),
            "inactive (dead)\n",
        );
        assert!(matches!(observation, ValidationObservation::Failed(_)));
    }

    #[test]
    fn run_cmd_sing_box_check_exit_zero_sentinel_is_validation_pass() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "run_cmd",
            &json!({"command":"sing-box check -c /tmp/config.json"}),
            "exit=0 command=sing-box check -c /tmp/config.json",
        );
        assert_eq!(observation, ValidationObservation::Passed);
    }

    #[test]
    fn run_cmd_nginx_test_ok_output_is_validation_pass() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "run_cmd",
            &json!({"command":"nginx -t"}),
            "nginx: the configuration file /etc/nginx/nginx.conf syntax is ok\nnginx: configuration file /etc/nginx/nginx.conf test is successful",
        );
        assert_eq!(observation, ValidationObservation::Passed);
    }

    #[test]
    fn run_cmd_ss_without_rows_is_validation_failure() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "run_cmd",
            &json!({"command":"ss -ltn sport = :8787"}),
            "State Recv-Q Send-Q Local Address:Port Peer Address:PortProcess",
        );
        assert!(matches!(observation, ValidationObservation::Failed(_)));
    }

    #[test]
    fn run_cmd_ss_with_listener_row_is_validation_pass() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "run_cmd",
            &json!({"command":"ss -ltn sport = :8787"}),
            "State Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\nLISTEN 0      128    127.0.0.1:8787      0.0.0.0:*",
        );
        assert_eq!(observation, ValidationObservation::Passed);
    }

    #[test]
    fn run_cmd_curl_exit_zero_sentinel_is_validation_pass() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "run_cmd",
            &json!({"command":"curl -fsS http://127.0.0.1:8787/v1/health -o /dev/null"}),
            "exit=0 command=curl -fsS http://127.0.0.1:8787/v1/health -o /dev/null",
        );
        assert_eq!(observation, ValidationObservation::Passed);
    }

    #[test]
    fn run_cmd_curl_validation_marker_output_is_validation_pass() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "run_cmd",
            &json!({"command":"curl -s http://127.0.0.1:8787/ | grep -q 'ops-demo-ok' && echo VALIDATION_PASSED || echo VALIDATION_FAILED"}),
            "VALIDATION_PASSED\n",
        );
        assert_eq!(observation, ValidationObservation::Passed);
    }

    #[test]
    fn run_cmd_curl_grep_match_output_is_validation_pass() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "run_cmd",
            &json!({"command":"curl -s http://127.0.0.1:8787/ | grep -o 'ops-demo-ok'"}),
            "ops-demo-ok\n",
        );
        assert_eq!(observation, ValidationObservation::Passed);
    }

    #[test]
    fn run_cmd_validation_marker_output_is_validation_pass() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "run_cmd",
            &json!({"command":"python3 -m http.server 65429 --bind 127.0.0.1 > /tmp/http.log 2>&1 & sleep 2 && curl -s http://127.0.0.1:65429/ | grep -q ops-demo-ok && echo VALIDATION_PASSED || echo VALIDATION_FAILED"}),
            "VALIDATION_PASSED\n",
        );
        assert_eq!(observation, ValidationObservation::Passed);
    }

    #[test]
    fn http_basic_2xx_is_validation_pass() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "http_basic",
            &json!({"action":"get","url":"http://127.0.0.1:8080/health"}),
            "status=200\n{\"ok\":true}\n",
        );
        assert_eq!(observation, ValidationObservation::Passed);
    }

    #[test]
    fn http_basic_missing_expected_content_is_validation_fail() {
        let state = test_state();
        let observation = assess_validation_output(
            &state,
            "http_basic",
            &json!({
                "action":"get",
                "url":"http://127.0.0.1:8080/health",
                "expect_contains":"ops-repair-ok"
            }),
            "status=200\nops-repair-bad\n",
        );
        assert!(matches!(observation, ValidationObservation::Failed(_)));
    }

    #[test]
    fn repair_budget_exhausted_after_limit() {
        let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        });
        recipe.saw_mutation = true;
        apply_action_effect_failure(&mut recipe, ActionEffect::validate());
        assert_eq!(
            stop_signal_for_validation_failure(&recipe),
            "recoverable_failure_continue_round"
        );
        apply_action_effect_failure(&mut recipe, ActionEffect::validate());
        assert_eq!(
            stop_signal_for_validation_failure(&recipe),
            "recoverable_failure_continue_round"
        );
        apply_action_effect_failure(&mut recipe, ActionEffect::validate());
        assert_eq!(
            stop_signal_for_validation_failure(&recipe),
            "recipe_repair_budget_exhausted"
        );
    }

    #[test]
    fn pre_mutation_validation_is_treated_as_inspect() {
        let recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        });
        let effect = effective_action_effect_for_recipe(recipe, ActionEffect::validate());
        assert!(effect.observes);
        assert!(!effect.validates);
    }

    #[test]
    fn pre_mutation_validation_failure_advances_to_apply_phase() {
        let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        });
        let effect = effective_action_effect_for_recipe(recipe, ActionEffect::validate());
        apply_action_effect_failure(&mut recipe, effect);
        assert!(recipe.saw_inspect);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Apply);
        assert_eq!(recipe.repair_count, 0);
    }

    #[test]
    fn pre_mutation_combined_mutate_and_validate_keeps_mutation() {
        let recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        });
        let effect = effective_action_effect_for_recipe(
            recipe,
            ActionEffect {
                observes: true,
                mutates: true,
                validates: true,
            },
        );
        assert!(effect.mutates);
        assert!(effect.validates);
    }

    #[test]
    fn failed_http_preflight_then_repair_mutate_then_validate_passes() {
        let state = test_state();
        let validate_args = json!({
            "action":"get",
            "url":"http://127.0.0.1:51179/",
            "expect_contains":"ops-repair-ok"
        });
        let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        });

        let preflight_effect = effective_action_effect_for_recipe(
            recipe,
            classify_skill_action_effect(&state, "http_basic", &validate_args),
        );
        let preflight_observation = assess_validation_output(
            &state,
            "http_basic",
            &validate_args,
            "status=200\nops-repair-bad\n",
        );
        assert!(matches!(
            preflight_observation,
            ValidationObservation::Failed(_)
        ));
        apply_action_effect_failure(&mut recipe, preflight_effect);
        assert_eq!(
            stop_signal_for_validation_failure(&recipe),
            "recoverable_failure_continue_round"
        );
        assert!(recipe.saw_inspect);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Apply);
        assert_eq!(recipe.repair_count, 0);

        let inspect_effect = classify_skill_action_effect(
            &state,
            "read_file",
            &json!({"path":"document/nl_ops_http_demo/index.html"}),
        );
        apply_action_effect_success(&mut recipe, inspect_effect);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Apply);

        let mutate_effect = classify_skill_action_effect(
            &state,
            "write_file",
            &json!({
                "path":"document/nl_ops_http_demo/index.html",
                "content":"ops-repair-ok\n"
            }),
        );
        apply_action_effect_success(&mut recipe, mutate_effect);
        assert!(recipe.saw_mutation);
        assert!(!recipe.saw_validation);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Validate);
        assert!(recipe.needs_validation());

        let post_repair_effect = effective_action_effect_for_recipe(
            recipe,
            classify_skill_action_effect(&state, "http_basic", &validate_args),
        );
        let post_repair_observation = assess_validation_output(
            &state,
            "http_basic",
            &validate_args,
            "status=200\nops-repair-ok\n",
        );
        assert_eq!(post_repair_observation, ValidationObservation::Passed);
        assert!(post_repair_effect.validates);
        apply_action_effect_success(&mut recipe, post_repair_effect);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Done);
        assert!(recipe.saw_validation);
        assert!(!recipe.needs_validation());
        assert_eq!(recipe.repair_count, 0);
    }

    #[test]
    fn failed_service_status_preflight_then_restart_then_verify_passes() {
        let state = test_state();
        let status_args = json!({"command":"systemctl status sing-box"});
        let verify_args = json!({"command":"systemctl is-active sing-box"});
        let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        });

        let preflight_effect = effective_action_effect_for_recipe(
            recipe,
            classify_skill_action_effect(&state, "run_cmd", &status_args),
        );
        let preflight_observation =
            assess_validation_output(&state, "run_cmd", &status_args, "inactive (dead)\n");
        assert!(matches!(
            preflight_observation,
            ValidationObservation::Failed(_)
        ));
        assert!(preflight_effect.observes);
        assert!(!preflight_effect.validates);
        apply_action_effect_failure(&mut recipe, preflight_effect);
        assert!(recipe.saw_inspect);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Apply);
        assert_eq!(recipe.repair_count, 0);

        let mutate_effect = classify_skill_action_effect(
            &state,
            "run_cmd",
            &json!({"command":"systemctl restart sing-box"}),
        );
        apply_action_effect_success(&mut recipe, mutate_effect);
        assert!(recipe.saw_mutation);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Validate);
        assert!(recipe.needs_validation());

        let verify_effect = effective_action_effect_for_recipe(
            recipe,
            classify_skill_action_effect(&state, "run_cmd", &verify_args),
        );
        let verify_observation =
            assess_validation_output(&state, "run_cmd", &verify_args, "active\n");
        assert_eq!(verify_observation, ValidationObservation::Passed);
        assert!(verify_effect.validates);
        apply_action_effect_success(&mut recipe, verify_effect);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Done);
        assert!(recipe.saw_validation);
        assert!(!recipe.needs_validation());
        assert_eq!(recipe.repair_count, 0);
    }

    #[test]
    fn failed_run_cmd_validation_then_repair_and_validate_passes() {
        let state = test_state();
        let preflight_args = json!({
            "command":"grep -q 'ops-repair-ok' document/nl_ops_http_demo/index.html && echo VALIDATION_PASSED || echo VALIDATION_FAILED"
        });
        let combined_repair = "printf 'ops-repair-ok\\n' > document/nl_ops_http_demo/index.html & sleep 1 && grep -q 'ops-repair-ok' document/nl_ops_http_demo/index.html && echo VALIDATION_PASSED || echo VALIDATION_FAILED";
        let (mutate_part, validate_part) =
            super::split_run_cmd_mutation_and_validation(combined_repair)
                .expect("split repair command");
        let mutate_args = json!({ "command": mutate_part });
        let validate_args = json!({ "command": validate_part });
        let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        });

        let preflight_effect = effective_action_effect_for_recipe(
            recipe,
            classify_skill_action_effect(&state, "run_cmd", &preflight_args),
        );
        let preflight_observation =
            assess_validation_output(&state, "run_cmd", &preflight_args, "VALIDATION_FAILED\n");
        assert!(matches!(
            preflight_observation,
            ValidationObservation::Failed(_)
        ));
        assert!(preflight_effect.observes);
        assert!(!preflight_effect.validates);
        apply_action_effect_failure(&mut recipe, preflight_effect);
        assert!(recipe.saw_inspect);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Apply);
        assert_eq!(recipe.repair_count, 0);

        let mutate_effect = classify_skill_action_effect(&state, "run_cmd", &mutate_args);
        apply_action_effect_success(&mut recipe, mutate_effect);
        assert!(recipe.saw_mutation);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Validate);

        let failed_validate_effect = effective_action_effect_for_recipe(
            recipe,
            classify_skill_action_effect(&state, "run_cmd", &validate_args),
        );
        let failed_validate_observation =
            assess_validation_output(&state, "run_cmd", &validate_args, "VALIDATION_FAILED\n");
        assert!(matches!(
            failed_validate_observation,
            ValidationObservation::Failed(_)
        ));
        assert!(failed_validate_effect.validates);
        apply_action_effect_failure(&mut recipe, failed_validate_effect);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Repair);
        assert_eq!(recipe.repair_count, 1);
        assert!(recipe.needs_validation());

        let retry_mutate_effect = classify_skill_action_effect(&state, "run_cmd", &mutate_args);
        apply_action_effect_success(&mut recipe, retry_mutate_effect);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Validate);
        assert!(!recipe.saw_validation);

        let passed_validate_effect = effective_action_effect_for_recipe(
            recipe,
            classify_skill_action_effect(&state, "run_cmd", &validate_args),
        );
        let passed_validate_observation =
            assess_validation_output(&state, "run_cmd", &validate_args, "VALIDATION_PASSED\n");
        assert_eq!(passed_validate_observation, ValidationObservation::Passed);
        apply_action_effect_success(&mut recipe, passed_validate_effect);
        assert_eq!(recipe.phase, ExecutionRecipePhase::Done);
        assert!(recipe.saw_validation);
        assert_eq!(recipe.repair_count, 1);
        assert!(!recipe.needs_validation());
    }
}
