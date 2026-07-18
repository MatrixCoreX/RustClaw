use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use claw_core::config::{CommandIntentConfig, MemoryConfig, ScheduleConfig};
use toml::Value as TomlValue;
use tracing::{info, warn};

use super::prompts::{load_required_prompt_template_for_vendor, RequiredPromptLoadError};
use crate::{CommandIntentRuntime, MemoryConfigFileWrapper, ScheduleRuntime};

fn locale_i18n_paths(i18n_root: &Path, locale: &str) -> Vec<PathBuf> {
    let suffix = format!(".{locale}.toml");
    let schedule_path = i18n_root.join(format!("schedule.{locale}.toml"));
    let mut paths = Vec::new();
    match std::fs::read_dir(i18n_root) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if file_name.ends_with(&suffix) {
                    paths.push(path);
                }
            }
        }
        Err(err) => {
            warn!(
                "read i18n dir failed: path={} err={err}",
                i18n_root.display()
            );
        }
    }
    if !paths.iter().any(|path| path == &schedule_path) {
        paths.push(schedule_path);
    }
    paths.sort();
    paths.dedup();
    paths
}

fn load_i18n_dict_from_path(path: &Path) -> Option<HashMap<String, String>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) => {
            warn!("read i18n file failed: path={} err={err}", path.display());
            return None;
        }
    };
    let value = match toml::from_str::<TomlValue>(&raw) {
        Ok(value) => value,
        Err(err) => {
            warn!("parse i18n file failed: path={} err={err}", path.display());
            return None;
        }
    };
    let Some(table) = value.get("dict").and_then(|v| v.as_table()) else {
        warn!("i18n file missing [dict]: path={}", path.display());
        return None;
    };
    let mut dict = HashMap::new();
    for (k, v) in table {
        collect_i18n_dict_entries(k, v, &mut dict);
    }
    Some(dict)
}

fn collect_i18n_dict_entries(prefix: &str, value: &TomlValue, out: &mut HashMap<String, String>) {
    if let Some(text) = value.as_str() {
        out.insert(prefix.to_string(), text.to_string());
        return;
    }
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, child) in table {
        let child_key = format!("{prefix}.{key}");
        collect_i18n_dict_entries(&child_key, child, out);
    }
}

fn insert_schedule_i18n_defaults(i18n_dict: &mut HashMap<String, String>) {
    i18n_dict
        .entry("schedule.desc.daily".to_string())
        .or_insert_with(|| "daily {time}".to_string());
    i18n_dict
        .entry("schedule.desc.weekly".to_string())
        .or_insert_with(|| "weekly weekday={weekday} {time}".to_string());
    i18n_dict
        .entry("schedule.desc.interval".to_string())
        .or_insert_with(|| "every {minutes}m".to_string());
    i18n_dict
        .entry("schedule.desc.once".to_string())
        .or_insert_with(|| "once".to_string());
    i18n_dict
        .entry("schedule.status.enabled".to_string())
        .or_insert_with(|| "enabled".to_string());
    i18n_dict
        .entry("schedule.status.paused".to_string())
        .or_insert_with(|| "paused".to_string());
    i18n_dict
        .entry("schedule.msg.list_empty".to_string())
        .or_insert_with(|| "There are no scheduled jobs right now.".to_string());
    i18n_dict
        .entry("schedule.msg.list_header".to_string())
        .or_insert_with(|| "Scheduled jobs:\n{lines}".to_string());
    i18n_dict
        .entry("schedule.msg.delete_none".to_string())
        .or_insert_with(|| "There are no scheduled jobs to delete.".to_string());
    i18n_dict
        .entry("schedule.msg.job_id_not_found".to_string())
        .or_insert_with(|| "Job ID not found: {job_id}".to_string());
    i18n_dict
        .entry("schedule.msg.delete_all_ok".to_string())
        .or_insert_with(|| "Deleted all scheduled jobs ({count} total).".to_string());
    i18n_dict
        .entry("schedule.msg.delete_one_ok".to_string())
        .or_insert_with(|| "Deleted scheduled job: {job_id}".to_string());
    i18n_dict
        .entry("schedule.msg.update_none".to_string())
        .or_insert_with(|| "There are no scheduled jobs to update.".to_string());
    i18n_dict
        .entry("schedule.msg.resume_all_ok".to_string())
        .or_insert_with(|| "Resumed all scheduled jobs ({count} total).".to_string());
    i18n_dict
        .entry("schedule.msg.pause_all_ok".to_string())
        .or_insert_with(|| "Paused all scheduled jobs ({count} total).".to_string());
    i18n_dict
        .entry("schedule.msg.resume_one_ok".to_string())
        .or_insert_with(|| "Resumed scheduled job: {job_id}".to_string());
    i18n_dict
        .entry("schedule.msg.pause_one_ok".to_string())
        .or_insert_with(|| "Paused scheduled job: {job_id}".to_string());
    i18n_dict
        .entry("schedule.msg.create_fail_task_kind".to_string())
        .or_insert_with(|| "Create failed: task.kind only supports ask or run_skill.".to_string());
    i18n_dict
        .entry("schedule.msg.cron_not_supported".to_string())
        .or_insert_with(|| {
            "Cron expressions are not supported in this version yet. Please use daily/weekly/every-N-minutes.".to_string()
        });
    i18n_dict
        .entry("schedule.msg.cron_not_supported_with_expr".to_string())
        .or_insert_with(|| {
            "Cron expressions are not supported in this version yet ({cron}). Please use daily/weekly/every-N-minutes.".to_string()
        });
    i18n_dict
        .entry("schedule.msg.create_fail_invalid_run_at".to_string())
        .or_insert_with(|| {
            "Create failed: invalid run_at for one-time job. Expected YYYY-MM-DD HH:MM[:SS]."
                .to_string()
        });
    i18n_dict
        .entry("schedule.msg.create_fail_run_at_must_be_future".to_string())
        .or_insert_with(|| "Create failed: execution time must be later than now.".to_string());
    i18n_dict
        .entry("schedule.msg.create_fail_cannot_compute_next_run".to_string())
        .or_insert_with(|| {
            "Create failed: cannot compute next run time; please check the time format.".to_string()
        });
    i18n_dict
        .entry("schedule.msg.create_exists_same".to_string())
        .or_insert_with(|| {
            "An identical scheduled job already exists: {job_id}\nType: {type}\nTimezone: {timezone}\nNext run: {next_run_human}\nTask content: {task_content}".to_string()
        });
    i18n_dict
        .entry("schedule.msg.update_existing_ok".to_string())
        .or_insert_with(|| {
            "Found an existing job for the same symbol; updated it: {job_id}\nType: {type}\nTimezone: {timezone}\nNext run: {next_run_human}\nTask content: {task_content}".to_string()
        });
    i18n_dict
        .entry("schedule.msg.create_ok".to_string())
        .or_insert_with(|| {
            "Scheduled successfully: {job_id}\nType: {type}\nTimezone: {timezone}\nNext run: {next_run_human}\nTask content: {task_content}".to_string()
        });
}

pub(crate) fn load_command_intent_runtime(cfg: &CommandIntentConfig) -> CommandIntentRuntime {
    let default_locale = if cfg.default_locale.trim().is_empty() {
        "zh-CN".to_string()
    } else {
        cfg.default_locale.trim().to_string()
    };
    CommandIntentRuntime { default_locale }
}

pub(crate) fn load_schedule_runtime(
    workspace_root: &Path,
    cfg: &ScheduleConfig,
    selected_vendor: Option<&str>,
) -> Result<ScheduleRuntime, RequiredPromptLoadError> {
    let timezone = if cfg.timezone.trim().is_empty() {
        "Asia/Shanghai".to_string()
    } else {
        cfg.timezone.trim().to_string()
    };

    let intent_prompt_logical_path = if cfg.intent_prompt_path.trim().is_empty() {
        "prompts/schedule_intent_prompt.md"
    } else {
        cfg.intent_prompt_path.trim()
    };
    let (intent_prompt_template, intent_prompt_source) = load_required_prompt_template_for_vendor(
        workspace_root,
        selected_vendor,
        intent_prompt_logical_path,
    )?;

    let intent_rules_logical_path = if cfg.intent_rules_path.trim().is_empty() {
        "prompts/schedule_intent_rules.md"
    } else {
        cfg.intent_rules_path.trim()
    };
    let (intent_rules_template, _intent_rules_file) = load_required_prompt_template_for_vendor(
        workspace_root,
        selected_vendor,
        intent_rules_logical_path,
    )?;

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
    let i18n_root = workspace_root.join(&i18n_dir);
    let schedule_file_name = format!("schedule.{locale}.toml");
    let mut i18n_dict = HashMap::new();
    let mut schedule_i18n_loaded = false;
    for path in locale_i18n_paths(&i18n_root, &locale) {
        let is_schedule_file = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == schedule_file_name);
        let Some(entries) = load_i18n_dict_from_path(&path) else {
            continue;
        };
        if is_schedule_file && !entries.is_empty() {
            schedule_i18n_loaded = true;
        }
        for (key, text) in entries {
            i18n_dict.insert(key, text);
        }
    }
    if !schedule_i18n_loaded {
        insert_schedule_i18n_defaults(&mut i18n_dict);
    }

    Ok(ScheduleRuntime {
        timezone,
        intent_prompt_template: Arc::new(RwLock::new(intent_prompt_template)),
        intent_prompt_source,
        intent_rules_template: Arc::new(RwLock::new(intent_rules_template)),
        locale,
        i18n_dir,
        i18n_dict,
    })
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

pub(crate) fn sanitize_command_before_execute(
    _runtime: &CommandIntentRuntime,
    command: &str,
) -> String {
    trim_command_text(command.trim().to_string())
}
