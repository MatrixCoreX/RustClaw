import type { SkillStoreItem, SkillStoreResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;

const SKILL_STORE_ERROR_MESSAGES: Record<string, readonly [string, string]> = {
  skill_store_name_required: ["没有识别到要操作的技能，请刷新后重试。", "No skill was selected. Refresh the page and try again."],
  skill_store_unknown_skill: ["这个技能已不在当前商店中，请刷新技能列表。", "This skill is no longer in the current store. Refresh the skill list."],
  skill_store_locked_skill: ["这是 RustClaw 的基础能力，不能从运行环境中删除。", "This is a core RustClaw capability and cannot be removed from the runtime."],
  skill_store_registry_unavailable: ["技能目录暂时不可用，请稍后刷新重试。", "The skill catalog is temporarily unavailable. Refresh and try again shortly."],
  skill_store_config_read_failed: ["无法读取技能设置，请检查服务状态后重试。", "RustClaw could not read the skill settings. Check the service status and try again."],
  skill_store_config_write_failed: ["无法保存技能设置，请检查磁盘空间和文件权限后重试。", "RustClaw could not save the skill settings. Check disk space and file permissions, then try again."],
  skill_store_runtime_reload_failed: ["技能设置已更新，但运行状态刷新失败，请重启 RustClaw 后确认。", "The skill settings were updated, but the runtime could not refresh them. Restart RustClaw and check again."],
  skill_store_invalid_runner_name: ["这个技能的运行文件配置无效，暂时不能安装。", "This skill has an invalid runner configuration and cannot be installed yet."],
  skill_store_install_not_on_demand: ["这个技能不支持从 Skill Store 按需安装。", "This skill does not support on-demand installation from Skill Store."],
  skill_store_invalid_install_package: ["这个技能的安装包配置无效，暂时不能安装。", "This skill has an invalid package configuration and cannot be installed yet."],
  skill_store_unsafe_config_path: ["这个技能声明了不安全的配置路径，已停止操作。", "This skill declares an unsafe configuration path, so the operation was stopped."],
  skill_store_build_start_failed: ["无法启动技能编译，请确认 Rust 工具链可用后重试。", "RustClaw could not start the skill build. Check the Rust toolchain and try again."],
  skill_store_build_failed: ["技能编译失败，请查看服务日志中的编译详情。", "The skill build failed. Check the service log for build details."],
  skill_store_build_binary_missing: ["技能编译结束但没有找到运行文件，请查看服务日志。", "The skill build finished without producing its runner. Check the service log."],
  skill_store_binary_remove_failed: ["技能已停用，但运行文件删除失败，请检查文件权限。", "The skill was disabled, but its runner could not be removed. Check file permissions."],
  skill_store_config_remove_failed: ["技能已停用，但配置文件删除失败，请检查文件权限。", "The skill was disabled, but its configuration could not be removed. Check file permissions."],
  skill_store_data_remove_failed: ["技能已停用，但私有数据删除失败，请检查文件权限和服务状态。", "The skill was disabled, but its private data could not be removed. Check file permissions and service status."],
  skill_store_operation_busy: ["另一个技能正在安装或删除，请等待完成后重试。", "Another skill is being installed or removed. Wait for it to finish, then try again."],
};

export function filterSkillStoreItems(items: SkillStoreItem[], query: string): SkillStoreItem[] {
  const storeItems = items.filter((item) => item.catalog_section === "other");
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return storeItems;
  return storeItems.filter((item) =>
    [item.name, item.description, item.group, item.source_kind]
      .filter(Boolean)
      .some((value) => String(value).toLocaleLowerCase().includes(normalized)),
  );
}

export function skillStoreInstallState(item: SkillStoreItem): "installed" | "repair_required" | "not_installed" {
  if (item.installed) return "installed";
  return item.installation_issue === "runner_missing" ? "repair_required" : "not_installed";
}

export function resolveSkillStoreActionName(
  localActionName: string | null,
  store: SkillStoreResponse | null,
): string | null {
  return localActionName || store?.active_operation?.skill_name || null;
}

export function removableSkillNames(
  otherGroupNames: readonly string[],
  externalSkillNames: ReadonlySet<string>,
  lockedSkillNames: ReadonlySet<string>,
): Set<string> {
  const names = new Set(otherGroupNames);
  externalSkillNames.forEach((name) => names.add(name));
  lockedSkillNames.forEach((name) => names.delete(name));
  return names;
}

export function skillStoreErrorMessage(errorCode: string | undefined, t: Translate): string {
  const message = errorCode ? SKILL_STORE_ERROR_MESSAGES[errorCode] : undefined;
  if (message) return t(message[0], message[1]);
  return t(
    "Skill Store 暂时无法完成这个操作，请稍后重试。",
    "Skill Store could not complete this operation. Try again shortly.",
  );
}
