import type { SkillsConfigResponse } from "../types/api";
import { isVisibleSkillName } from "./skill-display";
import { formatUiError, type UiErrorTranslate } from "./ui-error";

type SwitchBaseline = Pick<SkillsConfigResponse, "skills_list" | "skill_switches">;

function booleanMap(value: unknown): value is Record<string, boolean> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    && Object.values(value).every((entry) => typeof entry === "boolean");
}

function stringList(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((name) => typeof name === "string");
}

/** Validate the read fields that establish the meaning of a switch edit. */
export function hasSkillSwitchBaseline(value: unknown): boolean {
  if (value === null || typeof value !== "object") return false;
  const data = value as Record<string, unknown>;
  const optionalLists = [
    "base_skill_names", "core_skill_names", "fixed_on_skill_names", "initial_core_skill_names",
    "deferred_skill_names", "tool_skill_names", "locked_skill_names", "external_skill_names", "uninstalled_skill_names",
  ];
  return [data.skills_list, data.managed_skills, data.effective_enabled_skills_preview, data.runtime_enabled_skills].every(stringList)
    && booleanMap(data.skill_switches)
    && optionalLists.every((key) => data[key] === undefined || stringList(data[key]))
    && (data.skill_items === undefined || (Array.isArray(data.skill_items) && data.skill_items.every((item) =>
      item !== null && typeof item === "object" && typeof item.name === "string")));
}

function valueFor(config: SwitchBaseline, switches: Readonly<Record<string, boolean>>, name: string): boolean {
  return Object.prototype.hasOwnProperty.call(switches, name)
    ? switches[name]
    : config.skills_list.includes(name);
}

export function skillSwitchIsChanged(
  config: SwitchBaseline | null,
  draft: Readonly<Record<string, boolean>>,
  name: string,
  locked: ReadonlySet<string>,
): boolean {
  return config !== null && isVisibleSkillName(name) && !locked.has(name)
    && valueFor(config, draft, name) !== valueFor(config, config.skill_switches, name);
}

/** Deleting an override restores the startup default; it is not an omitted edit. */
export function skillSwitchChanges(
  config: SwitchBaseline | null,
  draft: Readonly<Record<string, boolean>>,
  locked: ReadonlySet<string>,
): Record<string, boolean> {
  if (!config) return {};
  const names = new Set([...Object.keys(config.skill_switches), ...Object.keys(draft)]);
  return Object.fromEntries([...names]
    .filter((name) => skillSwitchIsChanged(config, draft, name, locked))
    .map((name) => [name, valueFor(config, draft, name)]));
}

export function confirmSkillSwitchUpdate(value: unknown, changes: Readonly<Record<string, boolean>>): boolean {
  if (value === null || typeof value !== "object") throw new Error("skill_config_update_invalid");
  const data = value as Record<string, unknown>;
  if (typeof data.restart_required !== "boolean" || !booleanMap(data.skill_switches)) {
    throw new Error("skill_config_update_invalid");
  }
  const applied = data.skill_switches;
  if (Object.entries(changes).some(([name, enabled]) =>
    !Object.prototype.hasOwnProperty.call(applied, name) || applied[name] !== enabled)) {
    throw new Error("skill_config_update_not_applied");
  }
  return data.restart_required;
}

export function skillSwitchErrorMessage(cause: unknown, t: UiErrorTranslate): string {
  const code = cause instanceof Error ? cause.message : "";
  if (code === "skill_admission_generation_changed") {
    return t("另一个操作更新了技能设置。请刷新状态，重新确认开关后再保存。", "Another operation changed the skill settings. Refresh, review the switches, and save again.");
  }
  if (code === "skill_config_update_not_applied") {
    return t("部分开关没有生效。请刷新状态，检查技能安装和授权后重试。", "Some switches were not applied. Refresh and check skill installation and authorization before trying again.");
  }
  return formatUiError(cause, t, "技能设置保存失败，请稍后重试。", "Skill settings could not be saved. Try again shortly.");
}
