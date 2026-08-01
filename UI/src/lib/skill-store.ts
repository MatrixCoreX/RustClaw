import type { SkillStoreItem, SkillStoreResponse } from "../types/api";
import { productCopy } from "./product-identity";

type Translate = (zh: string, en: string) => string;

const SKILL_STORE_ERROR_MESSAGES: Record<string, readonly [string, string]> = {
  skill_store_name_required: ["没有识别到要操作的技能，请刷新后重试。", "No skill was selected. Refresh the page and try again."],
  skill_store_unknown_skill: ["这个技能已不在当前商店中，请刷新技能列表。", "This skill is no longer in the current store. Refresh the skill list."],
  skill_store_locked_skill: ["这是 {product_name} 的基础能力，不能从运行环境中删除。", "This is a core {product_name} capability and cannot be removed from the runtime."],
  skill_store_registry_unavailable: ["技能目录暂时不可用，请稍后刷新重试。", "The skill catalog is temporarily unavailable. Refresh and try again shortly."],
  skill_store_config_read_failed: ["无法读取技能设置，请检查服务状态后重试。", "{product_name} could not read the skill settings. Check the service status and try again."],
  skill_store_config_write_failed: ["无法保存技能设置，请检查磁盘空间和文件权限后重试。", "{product_name} could not save the skill settings. Check disk space and file permissions, then try again."],
  skill_store_runtime_reload_failed: ["技能设置已更新，但运行状态刷新失败，请重启 {product_name} 后确认。", "The skill settings were updated, but the runtime could not refresh them. Restart {product_name} and check again."],
  skill_store_install_not_on_demand: ["这个技能不支持从 Skill Store 按需安装。", "This skill does not support on-demand installation from Skill Store."],
  skill_store_manifest_missing: ["这个技能缺少安装清单，暂时不能安装。", "This skill is missing its install manifest and cannot be installed yet."],
  skill_store_manifest_invalid: ["这个技能的安装清单无效，请等待开发者修复。", "This skill has an invalid install manifest. Its developer needs to fix it."],
  skill_store_network_approval_required: ["这个技能安装时需要联网，请确认联网说明后再安装。", "This skill needs network access during installation. Review the network notice and confirm before installing."],
  skill_store_host_dependency_admin_required: ["这个技能需要安装系统依赖，请使用管理员账号操作。", "This skill needs system dependencies. Sign in with an administrator account to install it."],
  skill_store_host_dependency_unknown: ["安装清单声明了宿主不认识的系统依赖，已安全停止。", "The manifest declares a system dependency this host does not recognize, so installation stopped safely."],
  skill_store_host_dependency_install_failed: ["系统依赖没有安装完成，请展开诊断信息检查包管理器、网络或管理员权限。", "A system dependency could not be installed. Open diagnostics and check the package manager, network, or administrator privileges."],
  skill_store_runtime_asset_unknown: ["安装清单声明了宿主不认识的本地资源，已安全停止。", "The manifest declares a local resource this host does not recognize, so installation stopped safely."],
  skill_store_runtime_asset_install_failed: ["本地资源没有准备完成，技能未启用。请展开诊断信息检查网络和可用磁盘后重试。", "A local resource could not be prepared, so the skill was not enabled. Open diagnostics, check the network and free disk space, then try again."],
  skill_store_resource_insufficient: ["这台机器当前的内存或可用磁盘不足，无法完整安装这个技能。", "This machine does not currently have enough memory or free disk space to install the complete skill."],
  skill_store_unsafe_config_path: ["这个技能声明了不安全的配置路径，已停止操作。", "This skill declares an unsafe configuration path, so the operation was stopped."],
  skill_store_install_start_failed: ["无法启动技能安装，请检查服务状态后重试。", "{product_name} could not start the skill installation. Check the service status and try again."],
  skill_store_install_failed: ["技能安装未完成，请展开诊断信息查看缺少的运行环境或依赖。", "The skill installation did not finish. Open diagnostics to check for a missing runtime or dependency."],
  skill_store_package_remove_failed: ["技能已停用，但安装包删除失败，请检查文件权限。", "The skill was disabled, but its installed package could not be removed. Check file permissions."],
  skill_store_config_remove_failed: ["技能已停用，但配置文件删除失败，请检查文件权限。", "The skill was disabled, but its configuration could not be removed. Check file permissions."],
  skill_store_data_remove_failed: ["技能已停用，但私有数据删除失败，请检查文件权限和服务状态。", "The skill was disabled, but its private data could not be removed. Check file permissions and service status."],
  skill_store_operation_busy: ["另一个技能正在安装或删除，请等待完成后重试。", "Another skill is being installed or removed. Wait for it to finish, then try again."],
  skill_store_operation_state_failed: ["无法保存技能操作状态，请检查磁盘空间后重试。", "{product_name} could not save the skill operation state. Check disk space and try again."],
  skill_store_operation_not_found: ["这个技能操作已不存在，请刷新页面。", "This skill operation no longer exists. Refresh the page."],
  skill_store_rollback_unavailable: ["没有可恢复的上一版本。", "There is no previous verified version to restore."],
};

export function filterSkillStoreItems(items: SkillStoreItem[], query: string): SkillStoreItem[] {
  const storeItems = items.filter((item) => item.catalog_section === "other");
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return storeItems;
  return storeItems.filter((item) =>
    [item.name, item.description, item.description_zh, item.group, item.source_kind]
      .filter(Boolean)
      .some((value) => String(value).toLocaleLowerCase().includes(normalized)),
  );
}

export function skillStoreInstallState(item: SkillStoreItem): "installed" | "repair_required" | "not_installed" {
  if (item.installed) return "installed";
  return item.installation_issue === "package_missing" ? "repair_required" : "not_installed";
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
  if (message) return productCopy(t(message[0], message[1]));
  return productCopy(t(
    "Skill Store 暂时无法完成这个操作，请稍后重试。",
    "Skill Store could not complete this operation. Try again shortly.",
  ));
}
