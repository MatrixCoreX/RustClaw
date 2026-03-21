use std::collections::HashMap;
use std::path::{Path, PathBuf};

use claw_core::config::{CommandIntentConfig, MemoryConfig, ScheduleConfig};
use toml::Value as TomlValue;
use tracing::{info, warn};

use crate::{
    load_prompt_template_for_vendor, CommandIntentRules, CommandIntentRuntime,
    MemoryConfigFileWrapper, ScheduleRuntime, SCHEDULE_INTENT_PROMPT_TEMPLATE_DEFAULT,
    SCHEDULE_INTENT_RULES_TEMPLATE_DEFAULT,
};

pub(crate) fn load_command_intent_runtime(
    workspace_root: &Path,
    cfg: &CommandIntentConfig,
) -> CommandIntentRuntime {
    let rules_dir = workspace_root.join(cfg.rules_dir.trim());
    for locale in ["zh-CN", "en-US"] {
        let path = rules_dir.join(format!("{locale}.toml"));
        match std::fs::read_to_string(&path) {
            Ok(raw) => match toml::from_str::<CommandIntentRules>(&raw) {
                Ok(_) => {}
                Err(err) => {
                    warn!(
                        "load command intent rules failed: path={} err={err}",
                        path.display()
                    );
                }
            },
            Err(err) => {
                warn!(
                    "read command intent rules failed: path={} err={err}",
                    path.display()
                );
            }
        }
    }

    CommandIntentRuntime {
        all_result_suffixes: Vec::new(),
    }
}

pub(crate) fn load_schedule_runtime(
    workspace_root: &Path,
    cfg: &ScheduleConfig,
    selected_vendor: Option<&str>,
) -> ScheduleRuntime {
    let timezone = if cfg.timezone.trim().is_empty() {
        "Asia/Shanghai".to_string()
    } else {
        cfg.timezone.trim().to_string()
    };

    let prompt_rel = if cfg.intent_prompt_path.trim().is_empty() {
        "prompts/schedule_intent_prompt.md"
    } else {
        cfg.intent_prompt_path.trim()
    };
    let (intent_prompt_template, intent_prompt_file) = load_prompt_template_for_vendor(
        workspace_root,
        selected_vendor,
        prompt_rel,
        SCHEDULE_INTENT_PROMPT_TEMPLATE_DEFAULT,
    );

    let rules_rel = if cfg.intent_rules_path.trim().is_empty() {
        "prompts/schedule_intent_rules.md"
    } else {
        cfg.intent_rules_path.trim()
    };
    let (intent_rules_template, _intent_rules_file) = load_prompt_template_for_vendor(
        workspace_root,
        selected_vendor,
        rules_rel,
        SCHEDULE_INTENT_RULES_TEMPLATE_DEFAULT,
    );

    let locale = if cfg.locale.trim().is_empty() {
        "zh-CN".to_string()
    } else {
        cfg.locale.trim().to_string()
    };
    let i18n_dir = if cfg.i18n_dir.trim().is_empty() {
        "configs/i18n".to_string()
    } else {
        cfg.i18n_dir.trim().to_string()
    };
    let i18n_path = workspace_root
        .join(&i18n_dir)
        .join(format!("schedule.{locale}.toml"));
    let mut i18n_dict = HashMap::new();
    match std::fs::read_to_string(&i18n_path) {
        Ok(raw) => match toml::from_str::<TomlValue>(&raw) {
            Ok(value) => {
                if let Some(table) = value.get("dict").and_then(|v| v.as_table()) {
                    for (k, v) in table {
                        if let Some(text) = v.as_str() {
                            i18n_dict.insert(k.to_string(), text.to_string());
                        }
                    }
                } else {
                    warn!(
                        "schedule i18n file missing [dict]: path={}",
                        i18n_path.display()
                    );
                }
            }
            Err(err) => {
                warn!(
                    "parse schedule i18n file failed: path={} err={err}",
                    i18n_path.display()
                );
            }
        },
        Err(err) => {
            warn!(
                "read schedule i18n file failed: path={} err={err}",
                i18n_path.display()
            );
        }
    }
    if i18n_dict.is_empty() {
        i18n_dict.insert(
            "schedule.desc.daily".to_string(),
            "daily {time}".to_string(),
        );
        i18n_dict.insert(
            "schedule.desc.weekly".to_string(),
            "weekly weekday={weekday} {time}".to_string(),
        );
        i18n_dict.insert(
            "schedule.desc.interval".to_string(),
            "every {minutes}m".to_string(),
        );
        i18n_dict.insert("schedule.desc.once".to_string(), "once".to_string());
        i18n_dict.insert("schedule.status.enabled".to_string(), "enabled".to_string());
        i18n_dict.insert("schedule.status.paused".to_string(), "paused".to_string());
        i18n_dict.insert(
            "schedule.msg.list_empty".to_string(),
            "There are no scheduled jobs right now.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.list_header".to_string(),
            "Scheduled jobs:\n{lines}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.delete_none".to_string(),
            "There are no scheduled jobs to delete.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.job_id_not_found".to_string(),
            "Job ID not found: {job_id}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.delete_all_ok".to_string(),
            "Deleted all scheduled jobs ({count} total).".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.delete_one_ok".to_string(),
            "Deleted scheduled job: {job_id}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.update_none".to_string(),
            "There are no scheduled jobs to update.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.resume_all_ok".to_string(),
            "Resumed all scheduled jobs ({count} total).".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.pause_all_ok".to_string(),
            "Paused all scheduled jobs ({count} total).".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.resume_one_ok".to_string(),
            "Resumed scheduled job: {job_id}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.pause_one_ok".to_string(),
            "Paused scheduled job: {job_id}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.create_fail_task_kind".to_string(),
            "Create failed: task.kind only supports ask or run_skill.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.cron_not_supported".to_string(),
            "Cron expressions are not supported in this version yet. Please use daily/weekly/every-N-minutes.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.cron_not_supported_with_expr".to_string(),
            "Cron expressions are not supported in this version yet ({cron}). Please use daily/weekly/every-N-minutes.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.create_fail_invalid_run_at".to_string(),
            "Create failed: invalid run_at for one-time job. Expected YYYY-MM-DD HH:MM[:SS]."
                .to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.create_fail_run_at_must_be_future".to_string(),
            "Create failed: execution time must be later than now.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.create_fail_cannot_compute_next_run".to_string(),
            "Create failed: cannot compute next run time; please check the time format."
                .to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.create_exists_same".to_string(),
            "An identical scheduled job already exists: {job_id}\nType: {type}\nTimezone: {timezone}\nNext run: {next_run_human}\nTask content: {task_content}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.update_existing_ok".to_string(),
            "Found an existing job for the same symbol; updated it: {job_id}\nType: {type}\nTimezone: {timezone}\nNext run: {next_run_human}\nTask content: {task_content}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.create_ok".to_string(),
            "Scheduled successfully: {job_id}\nType: {type}\nTimezone: {timezone}\nNext run: {next_run_human}\nTask content: {task_content}".to_string(),
        );
    }

    ScheduleRuntime {
        timezone,
        intent_prompt_template,
        intent_prompt_file,
        intent_rules_template,
        i18n_dict,
    }
}

pub(crate) fn load_memory_runtime_config(
    workspace_root: &Path,
    cfg: &MemoryConfig,
) -> MemoryConfig {
    let path_raw = cfg.config_path.trim();
    if path_raw.is_empty() {
        return cfg.clone();
    }
    let path = if Path::new(path_raw).is_absolute() {
        PathBuf::from(path_raw)
    } else {
        workspace_root.join(path_raw)
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(v) => v,
        Err(err) => {
            warn!(
                "read memory config failed: path={} err={err}; fallback to main config values",
                path.display()
            );
            return cfg.clone();
        }
    };
    match toml::from_str::<MemoryConfig>(&raw) {
        Ok(mut loaded) => {
            loaded.config_path = path_raw.to_string();
            info!(
                "loaded memory runtime config: path={} recall_limit={} prompt_recall_limit={}",
                path.display(),
                loaded.recall_limit,
                loaded.prompt_recall_limit
            );
            loaded
        }
        Err(_) => match toml::from_str::<MemoryConfigFileWrapper>(&raw) {
            Ok(mut wrapped) => {
                wrapped.memory.config_path = path_raw.to_string();
                info!(
                    "loaded wrapped memory runtime config: path={} recall_limit={} prompt_recall_limit={}",
                    path.display(),
                    wrapped.memory.recall_limit,
                    wrapped.memory.prompt_recall_limit
                );
                wrapped.memory
            }
            Err(err) => {
                warn!(
                    "parse memory config failed: path={} err={err}; fallback to main config values",
                    path.display()
                );
                cfg.clone()
            }
        },
    }
}

pub(crate) fn trim_command_text(mut s: String) -> String {
    s = s.trim().to_string();
    while s.ends_with(|c: char| {
        matches!(
            c,
            '。' | '，' | ',' | ';' | '；' | ':' | '：' | '!' | '！' | '?' | '？'
        )
    }) {
        s.pop();
        s = s.trim_end().to_string();
    }
    if (s.starts_with('`') && s.ends_with('`')) || (s.starts_with('"') && s.ends_with('"')) {
        s = s[1..s.len().saturating_sub(1)].trim().to_string();
    }
    s
}

pub(crate) fn strip_result_suffixes(command: &str, _suffixes: &[String]) -> String {
    trim_command_text(command.trim().to_string())
}

pub(crate) fn sanitize_command_before_execute(
    runtime: &CommandIntentRuntime,
    command: &str,
) -> String {
    strip_result_suffixes(command, &runtime.all_result_suffixes)
}
