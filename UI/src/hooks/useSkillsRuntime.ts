import { useEffect, useMemo, useRef, useState, type SetStateAction } from "react";

import { useUiDialog } from "../components/UiDialogProvider";
import {
  baseSkillNamesFromRegistry,
  filterSkillNamesBySearch,
  groupSkillNames,
  isUiHiddenSkill,
  isVisibleSkillName,
  normalizeSkillSearchQuery,
  visibleSkillNames,
} from "../lib/skill-display";
import { removableSkillNames, resolveSkillStoreActionName, skillStoreErrorMessage } from "../lib/skill-store";
import { formatUiError } from "../lib/ui-error";
import { hasSkillSwitchBaseline, readSkillSwitchAcknowledgement, skillSwitchChanges, skillSwitchErrorMessage, skillSwitchLocks } from "../lib/skill-switches";
import { SkillSwitchDraft } from "../lib/skill-switch-draft";
import { PageRequestLifecycle } from "../lib/page-request-lifecycle";
import { useRequestOwner } from "./useRequestOwner";
import type {
  ApiResponse,
  BrowserFileWithPath,
  ImportedSkillResponse,
  SkillPolicyReview,
  SkillListItem,
  SkillStoreDependencyResponse,
  SkillStoreOperationResponse,
  SkillStoreResponse,
  SkillsConfigResponse,
  SkillsResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type RestartSystem = () => Promise<boolean>;

export interface UseSkillsRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  requestScope: string;
}

export function useSkillsRuntime({ apiFetch, t, requestScope }: UseSkillsRuntimeParams) {
  const { confirm: showConfirm } = useUiDialog();
  const isCurrent = useRequestOwner(requestScope);
  const owner = useMemo(() => ({
    draft: new SkillSwitchDraft(), configReads: new PageRequestLifecycle(), metadataReads: new PageRequestLifecycle(),
    policyReads: new PageRequestLifecycle(), policyWriting: false,
    saving: false, storeFetch: null as Promise<void> | null, storeFetchedAt: 0, storePage: 1, storeQuery: "",
    previousOperation: null as SkillStoreResponse["active_operation"],
  }), [isCurrent]);
  const [skillsData, setSkillsData] = useState<SkillsResponse | null>(null);
  const [skillsConfigLoading, setSkillsConfigLoading] = useState(false);
  const [skillsConfigError, setSkillsConfigError] = useState<string | null>(null);
  const [skillsConfigData, setSkillsConfigData] = useState<SkillsConfigResponse | null>(null);
  const [skillSwitchDraft, setSkillSwitchDraft] = useState<Record<string, boolean>>({});
  const [skillSwitchSaving, setSkillSwitchSaving] = useState(false);
  const [skillStoreData, setSkillStoreData] = useState<SkillStoreResponse | null>(null);
  const [skillStoreLoading, setSkillStoreLoading] = useState(false);
  const [skillStoreError, setSkillStoreError] = useState<string | null>(null);
  const [skillStoreMessage, setSkillStoreMessage] = useState<string | null>(null);
  const [localSkillStoreActionName, setLocalSkillStoreActionName] = useState<string | null>(null);
  const [skillSwitchSaveMessage, setSkillSwitchSaveMessage] = useState<string | null>(null);
  const [skillsSearchQuery, setSkillsSearchQuery] = useState("");
  const [skillImportSource, setSkillImportSource] = useState("");
  const [skillImportLoading, setSkillImportLoading] = useState(false);
  const [skillImportError, setSkillImportError] = useState<string | null>(null);
  const [skillImportMessage, setSkillImportMessage] = useState<string | null>(null);
  const [skillImportPreview, setSkillImportPreview] = useState<ImportedSkillResponse | null>(null);
  const [skillPolicyReviews, setSkillPolicyReviews] = useState<SkillPolicyReview[]>([]);
  const [recentImportedSkillName, setRecentImportedSkillName] = useState<string | null>(null);
  const [localImportPickerOpen, setLocalImportPickerOpen] = useState(false);
  const folderImportInputRef = useRef<HTMLInputElement | null>(null);
  const fileImportInputRef = useRef<HTMLInputElement | null>(null);
  const skillStoreActionName = resolveSkillStoreActionName(localSkillStoreActionName, skillStoreData);

  useEffect(() => {
    setSkillsData(null); setSkillsConfigData(null); setSkillSwitchDraft({});
    setSkillsConfigLoading(false); setSkillsConfigError(null); setSkillSwitchSaving(false); setSkillSwitchSaveMessage(null);
    setSkillStoreData(null); setSkillStoreLoading(false); setSkillStoreError(null); setSkillStoreMessage(null);
    setLocalSkillStoreActionName(null); setSkillsSearchQuery(""); setSkillImportSource("");
    setSkillImportLoading(false); setSkillImportError(null); setSkillImportMessage(null); setSkillImportPreview(null);
    setRecentImportedSkillName(null); setLocalImportPickerOpen(false); setSkillPolicyReviews([]);
    if (folderImportInputRef.current) folderImportInputRef.current.value = "";
    if (fileImportInputRef.current) fileImportInputRef.current.value = "";
  }, [owner]);

  const publishSwitchDraft = () => {
    setSkillsConfigData(owner.draft.baseline);
    setSkillSwitchDraft(owner.draft.values());
  };

  const fetchSkills = async () => {
    if (!isCurrent()) return;
    const request = owner.metadataReads.begin(false)!;
    try {
      const res = await apiFetch(`/v1/skills`);
      const body = (await res.json()) as ApiResponse<SkillsResponse>;
      if (!isCurrent() || !owner.metadataReads.isCurrent(request)) return;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `skills_fetch_http_${res.status}`);
      }
      setSkillsData(body.data);
    } catch {
      // The visible skill switch surface is driven by /v1/skills/config; keep stale runtime metadata if /v1/skills is transiently unavailable.
    }
  };

  const loadSkillsConfig = async (duringSave = false) => {
    if (!isCurrent() || (owner.saving && !duringSave)) return;
    const request = owner.configReads.begin(false)!;
    const ownsRead = () => isCurrent() && owner.configReads.isCurrent(request) && (!owner.saving || duringSave);
    setSkillsConfigLoading(true);
    setSkillsConfigError(null);
    try {
      const res = await apiFetch(`/v1/skills/config`);
      const body = (await res.json()) as ApiResponse<SkillsConfigResponse>;
      if (!ownsRead()) return;
      if (!res.ok || !body?.ok || !body.data || !hasSkillSwitchBaseline(body.data)) {
        throw new Error(body?.error || `skill_config_fetch_http_${res.status}`);
      }
      owner.draft.rebase(body.data);
      publishSwitchDraft();
    } catch (err) {
      if (!ownsRead()) return;
      const message = formatUiError(err, t, "技能设置暂时无法读取。", "Skill settings are temporarily unavailable.");
      setSkillsConfigError(message);
    } finally {
      if (ownsRead() && owner.configReads.finish(request)) setSkillsConfigLoading(false);
    }
  };
  const fetchSkillsConfig = () => loadSkillsConfig();

  const fetchSkillStore = (
    force = false,
    page = owner.storePage,
    query = owner.storeQuery,
  ): Promise<void> => {
    if (!isCurrent()) return Promise.resolve();
    const normalizedPage = Math.max(1, Math.trunc(page));
    const normalizedQuery = query.trim();
    const browsingChanged = normalizedPage !== owner.storePage || normalizedQuery !== owner.storeQuery;
    owner.storePage = normalizedPage;
    owner.storeQuery = normalizedQuery;
    const freshEnough = Date.now() - owner.storeFetchedAt < 3_000;
    if (!force && !browsingChanged && freshEnough) return Promise.resolve();
    if (owner.storeFetch) {
      const activeRequest = owner.storeFetch;
      return browsingChanged
        ? activeRequest.then(() => fetchSkillStore(true, normalizedPage, normalizedQuery))
        : activeRequest;
    }
    const request = (async () => {
      setSkillStoreLoading(true);
      setSkillStoreError(null);
      try {
        const params = new URLSearchParams({
          page: String(normalizedPage),
          page_size: "24",
        });
        if (normalizedQuery) params.set("q", normalizedQuery);
        const res = await apiFetch(`/v1/skills/store?${params.toString()}`);
        const body = (await res.json()) as ApiResponse<SkillStoreResponse>;
        if (!isCurrent()) return;
        if (!res.ok || !body.ok || !body.data) {
          throw new Error(skillStoreErrorMessage(body.error, t));
        }
        owner.storeFetchedAt = Date.now();
        const previousOperation = owner.previousOperation;
        const activeOperation = body.data.active_operation ?? null;
        owner.previousOperation = activeOperation;
        setSkillStoreData(body.data);
        if (previousOperation && !activeOperation) {
          const completedOperation = body.data.recent_operations?.find(
            (operation) => operation.operation_id === previousOperation.operation_id,
          );
          if (completedOperation?.status === "failure") {
            setSkillStoreError(skillStoreErrorMessage(completedOperation.failure?.error_code, t));
          } else if (completedOperation?.status === "cancelled") {
            setSkillStoreMessage(
              t(
                `${previousOperation.skill_name} 的操作已取消，没有继续执行。`,
                `The operation for ${previousOperation.skill_name} was cancelled.`,
              ),
            );
          } else {
            const item = body.data.items.find((candidate) => candidate.name === previousOperation.skill_name);
            const operationSucceeded = previousOperation.action === "remove" ? item?.configured_installed === false : item?.installed === true;
            const installOrigin =
              completedOperation?.result && typeof completedOperation.result === "object"
                ? completedOperation.result.install_origin
                : null;
            setSkillStoreMessage(
              operationSucceeded
                ? previousOperation.action !== "remove"
                  ? installOrigin === "platform_precompiled"
                    ? t(
                        `${previousOperation.skill_name} 已使用当前平台的预编译版本安装完成，可以直接使用。`,
                        `${previousOperation.skill_name} was installed from the precompiled package for this platform and is ready to use.`,
                      )
                    : t(
                        `${previousOperation.skill_name} 已完成安装并可以使用。`,
                        `${previousOperation.skill_name} finished installing and is ready to use.`,
                      )
                  : t(
                      `${previousOperation.skill_name} 已完成删除。`,
                      `${previousOperation.skill_name} finished removing.`,
                    )
                : t(
                    `${previousOperation.skill_name} 的操作已经结束，但未达到预期状态，请重试或查看服务日志。`,
                    `${previousOperation.skill_name} finished processing but did not reach the expected state. Retry or check the service log.`,
                  ),
            );
          }
          await fetchSkillsConfig();
          await fetchSkills();
        }
      } catch (err) {
        if (!isCurrent()) return;
        const message = formatUiError(err, t, "Skill Store 暂时无法读取。", "Skill Store is temporarily unavailable.");
        setSkillStoreError(message);
      } finally {
        if (isCurrent()) setSkillStoreLoading(false);
      }
    })();
    owner.storeFetch = request;
    void request.finally(() => {
      if (owner.storeFetch === request) owner.storeFetch = null;
    });
    return request;
  };

  const fetchSkillStoreDependencies = async (skillName: string): Promise<SkillStoreDependencyResponse> => {
    if (!isCurrent()) throw new Error("skill_request_scope_changed");
    const res = await apiFetch(`/v1/skills/store/${encodeURIComponent(skillName)}/dependencies`);
    const body = (await res.json()) as ApiResponse<SkillStoreDependencyResponse>;
    if (!isCurrent()) throw new Error("skill_request_scope_changed");
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(skillStoreErrorMessage(body.error, t));
    }
    return body.data;
  };

  const scrollToSkillRow = (skillName: string) => {
    window.setTimeout(() => {
      if (!isCurrent()) return;
      const row = document.getElementById(`skill-row-${skillName}`);
      row?.scrollIntoView({ behavior: "smooth", block: "center" });
    }, 180);
  };

  const managedSkills = useMemo(() => {
    const set = new Set<string>(skillsConfigData?.managed_skills ?? []);
    Object.keys(skillSwitchDraft).forEach((k) => set.add(k));
    return Array.from(set)
      .filter(isVisibleSkillName)
      .sort((a, b) => a.localeCompare(b));
  }, [skillsConfigData, skillSwitchDraft]);

  const baseSkillNamesSet = useMemo(() => {
    return new Set<string>(baseSkillNamesFromRegistry(skillsConfigData?.base_skill_names));
  }, [skillsConfigData?.base_skill_names]);

  const toolSkillNamesSet = useMemo(() => {
    return new Set<string>(visibleSkillNames(skillsConfigData?.tool_skill_names));
  }, [skillsConfigData?.tool_skill_names]);

  const lockedSkillNamesSet = useMemo(() => {
    return skillSwitchLocks(skillsConfigData);
  }, [skillsConfigData]);

  const externalSkillNamesSet = useMemo(() => {
    return new Set<string>(visibleSkillNames(skillsConfigData?.external_skill_names));
  }, [skillsConfigData?.external_skill_names]);

  const configuredEnabledSkills = useMemo(() => {
    const set = new Set<string>(visibleSkillNames(skillsConfigData?.skills_list));
    Object.entries(skillSwitchDraft).forEach(([name, value]) => {
      if (isUiHiddenSkill(name)) return;
      if (value) set.add(name);
      else set.delete(name);
    });
    lockedSkillNamesSet.forEach((name) => set.add(name));
    return set;
  }, [lockedSkillNamesSet, skillsConfigData, skillSwitchDraft]);

  const pendingSkillSwitchChanges = useMemo(
    () => skillSwitchChanges(skillsConfigData, skillSwitchDraft, lockedSkillNamesSet),
    [skillsConfigData, skillSwitchDraft, lockedSkillNamesSet],
  );
  const hasUnsavedSkillSwitchChanges = Object.keys(pendingSkillSwitchChanges).length > 0;

  const normalizedSkillsSearchQuery = useMemo(() => normalizeSkillSearchQuery(skillsSearchQuery), [skillsSearchQuery]);
  const filteredManagedSkills = useMemo(
    () => filterSkillNamesBySearch(managedSkills, normalizedSkillsSearchQuery),
    [managedSkills, normalizedSkillsSearchQuery],
  );

  const skillGroups = useMemo(
    () => groupSkillNames(managedSkills, baseSkillNamesSet, toolSkillNamesSet),
    [managedSkills, baseSkillNamesSet, toolSkillNamesSet],
  );
  const filteredSkillsTool = useMemo(() => filterSkillNamesBySearch(skillGroups.tool, normalizedSkillsSearchQuery), [skillGroups.tool, normalizedSkillsSearchQuery]);
  const filteredSkillsImage = useMemo(() => filterSkillNamesBySearch(skillGroups.image, normalizedSkillsSearchQuery), [skillGroups.image, normalizedSkillsSearchQuery]);
  const filteredSkillsAudio = useMemo(() => filterSkillNamesBySearch(skillGroups.audio, normalizedSkillsSearchQuery), [skillGroups.audio, normalizedSkillsSearchQuery]);
  const filteredSkillsMultimedia = useMemo(() => filterSkillNamesBySearch(skillGroups.multimedia, normalizedSkillsSearchQuery), [skillGroups.multimedia, normalizedSkillsSearchQuery]);
  const filteredSkillsBase = useMemo(() => filterSkillNamesBySearch(skillGroups.base, normalizedSkillsSearchQuery), [skillGroups.base, normalizedSkillsSearchQuery]);
  const filteredSkillsOther = useMemo(() => filterSkillNamesBySearch(skillGroups.other, normalizedSkillsSearchQuery), [skillGroups.other, normalizedSkillsSearchQuery]);
  const removableSkillNamesSet = useMemo(
    () => removableSkillNames(skillGroups.other, externalSkillNamesSet, lockedSkillNamesSet),
    [externalSkillNamesSet, lockedSkillNamesSet, skillGroups.other],
  );

  const skillItemsByName = useMemo(() => {
    const map = new Map<string, SkillListItem>();
    (skillsData?.skill_items ?? []).forEach((item) => {
      if (!isVisibleSkillName(item.name)) return;
      map.set(item.name, item);
    });
    (skillsConfigData?.skill_items ?? []).forEach((item) => {
      if (!isVisibleSkillName(item.name)) return;
      map.set(item.name, item);
    });
    return map;
  }, [skillsConfigData?.skill_items, skillsData?.skill_items]);

  useEffect(() => {
    if (!skillImportPreview) return;
    if (managedSkills.includes(skillImportPreview.skill_name)) return;
    setSkillImportPreview(null);
    if (recentImportedSkillName === skillImportPreview.skill_name) {
      setRecentImportedSkillName(null);
    }
  }, [managedSkills, recentImportedSkillName, skillImportPreview]);

  useEffect(() => {
    const activeOperation = skillStoreData?.active_operation;
    if (!isCurrent() || !activeOperation) return;
    const refreshWhenVisible = () => {
      if (document.visibilityState === "visible") void fetchSkillStore(true);
    };
    const timer = window.setInterval(() => {
      refreshWhenVisible();
    }, 1_500);
    document.addEventListener("visibilitychange", refreshWhenVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
    };
    // The operation identity is the server-owned polling boundary.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    isCurrent,
    skillStoreData?.active_operation?.operation_id,
    skillStoreData?.active_operation?.status,
    skillStoreData?.active_operation?.stage,
    skillStoreData?.active_operation?.action,
    skillStoreData?.active_operation?.skill_name,
    skillStoreData?.active_operation?.heartbeat_at_unix,
  ]);

  const saveSkillSwitches = async (restartSystem?: RestartSystem) => {
    if (!isCurrent() || owner.saving) return;
    setSkillSwitchSaveMessage(null);
    setSkillsConfigError(null);
    const baseline = owner.draft.baseline;
    if (!baseline) {
      setSkillsConfigError(t("技能设置尚未加载，请先刷新。", "Load the skill settings before saving."));
      return;
    }
    const changes = owner.draft.changes();
    if (Object.keys(changes).length === 0) {
      setSkillSwitchSaveMessage(t("当前没有需要保存的修改。", "There are no changes to save."));
      return;
    }
    owner.saving = true;
    const sequence = owner.draft.beginWrite();
    owner.configReads.invalidate();
    setSkillsConfigLoading(false);
    setSkillSwitchSaving(true);
    try {
      const res = await apiFetch(`/v1/skills/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ skill_switches: changes, expected_revision: baseline.revision }),
      });
      const body = (await res.json()) as ApiResponse<unknown>;
      if (!isCurrent()) return;
      if (!res.ok || !body?.ok) {
        throw new Error(body?.error || `skill_config_save_http_${res.status}`);
      }
      const ack = readSkillSwitchAcknowledgement(body.data, changes);
      owner.draft.acknowledge(ack, sequence, changes);
      publishSwitchDraft();
      const restartRequired = ack.restart_required;
      let savedMessage = t(
        "本次开关修改已生效。",
        "The submitted switch changes are active.",
      );
      if (restartRequired) {
        const confirmed = await showConfirm({
          title: t("重启 {product_name}", "Restart {product_name}"),
          message: t(
            "这些变更需要重启 {product_name} 才会生效。现在就自动重启吗？",
            "These changes need a {product_name} restart to take effect. Restart now?",
          ),
          confirmLabel: t("立即重启", "Restart now"),
        });
        if (!isCurrent()) return;
        if (confirmed) {
          savedMessage = t(
            "技能开关已保存，正在重启 {product_name}，请稍候。",
            "Skill switches were saved. Restarting {product_name} now.",
          );
        } else {
          savedMessage = t(
            "技能开关已保存。你可以稍后再重启 {product_name} 让它生效。",
            "Skill switches were saved. You can restart {product_name} later to apply them.",
          );
        }
        setSkillSwitchSaveMessage(savedMessage);
        await loadSkillsConfig(true);
        await fetchSkills();
        if (!isCurrent()) return;
        if (confirmed && restartSystem) {
          const restarted = await restartSystem();
          if (!isCurrent()) return;
          setSkillSwitchSaveMessage(
            restarted
              ? t("{product_name} 已重启完成，技能开关现在已经生效。", "{product_name} restarted successfully. Skill switches are now active.")
              : t("重启请求已经发出，请稍后刷新确认技能开关是否生效。", "Restart was requested. Please refresh shortly to confirm the skill switches are active."),
          );
        }
        return;
      }
      setSkillSwitchSaveMessage(savedMessage);
      await loadSkillsConfig(true);
      await fetchSkills();
    } catch (err) {
      if (!isCurrent()) return;
      const message = skillSwitchErrorMessage(err, t);
      setSkillsConfigError(message);
    } finally {
      owner.saving = false;
      owner.draft.finishWrite();
      if (isCurrent()) { publishSwitchDraft(); setSkillSwitchSaving(false); }
    }
  };

  const importExternalSkill = async () => {
    if (!isCurrent()) return;
    const source = skillImportSource.trim();
    if (!source) {
      setSkillImportError(t("请先输入包含 skill.toml 和 INTERFACE.md 的本地目录。", "Enter a local directory containing skill.toml and INTERFACE.md."));
      return;
    }
    const allowNetwork = await showConfirm({
      title: t("导入时是否允许联网？", "Allow network during import?"),
      message: t(
        "默认保持离线更安全。如果技能清单声明需要下载锁定依赖或验证远程 HTTPS 端点，可以允许本次隔离安装任务联网；运行时权限不会因此自动放开。",
        "Offline is safer by default. Allow this isolated install job to use the network only if the manifest needs locked dependencies or HTTPS endpoint validation; runtime permissions are not enabled automatically.",
      ),
      confirmLabel: t("允许本次联网", "Allow this time"),
      cancelLabel: t("保持离线导入", "Import offline"),
    });
    if (!isCurrent()) return;
    setSkillImportLoading(true);
    setSkillImportError(null);
    setSkillImportMessage(null);
    try {
      const res = await apiFetch(`/v1/skills/import`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ source, enabled: false, allow_network: allowNetwork }),
      });
      const body = (await res.json()) as ApiResponse<ImportedSkillResponse>;
      if (!isCurrent()) return;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error === "skill_store_precompiled_required"
          ? skillStoreErrorMessage(body.error, t)
          : body.error || `skill_import_http_${res.status}`);
      }
      setSkillImportPreview(body.data);
      if (body.data.policy_review) setSkillPolicyReviews([body.data.policy_review]);
      setRecentImportedSkillName(body.data.skill_name);
      setSkillImportMessage(
        t(
          `已安装 ${body.data.display_name}，请查看并批准运行权限后开启。`,
          `${body.data.display_name} is installed. Review and approve runtime permissions before enabling it.`,
        ),
      );
      setSkillsSearchQuery(body.data.skill_name);
      await fetchSkillsConfig();
      await fetchSkills();
      await fetchSkillStore(true);
      scrollToSkillRow(body.data.skill_name);
    } catch (err) {
      if (!isCurrent()) return;
      const message = formatUiError(err, t, "技能导入失败，请检查技能包后重试。", "Skill import failed. Check the package and try again.");
      setSkillImportError(message);
    } finally {
      if (isCurrent()) setSkillImportLoading(false);
    }
  };

  const reviewSkillPolicies = async () => {
    if (!isCurrent()) return;
    const request = owner.policyReads.begin(false)!;
    const ownsRead = () => isCurrent() && owner.policyReads.isCurrent(request);
    setSkillImportLoading(true); setSkillImportError(null);
    try {
      const res = await apiFetch("/v1/skills/policy-reviews"); const body = await res.json();
      if (!ownsRead()) return;
      if (!res.ok || !body.ok) throw new Error(body.error || "skill_policy_review_failed");
      setSkillPolicyReviews(body.data.reviews);
    } catch (error) { if (ownsRead()) setSkillImportError(formatUiError(error, t, "权限信息读取失败，请重试。", "Permission details could not be loaded. Try again.")); }
    finally { if (ownsRead() && owner.policyReads.finish(request) && !owner.policyWriting) setSkillImportLoading(false); }
  };

  const approveImportedSkillPolicy = async (selected?: SkillPolicyReview, decision: "approve" | "revoke" | "discard" = "approve") => {
    const review = selected || skillImportPreview?.policy_review;
    if (!review || !isCurrent() || owner.policyWriting) return;
    owner.policyWriting = true;
    const approved = await showConfirm({
      title: decision === "discard" ? t("移除待授权技能？", "Remove this unapproved skill?") : decision === "approve" ? t("批准运行权限并开启技能？", "Approve permissions and enable this skill?") : t("撤销这个技能的运行授权？", "Revoke this skill’s runtime permissions?"),
      message: decision === "discard" ? t("会移除安装记录，保留技能文件、配置和数据。", "The installation entry will be removed. Package files, configuration and data are retained.") : t("请确认下面列出的权限是完成技能工作所必需的。之后可以关闭技能或撤销授权。", "Confirm that the listed permissions are needed. You can disable the skill or revoke this approval later."),
      confirmLabel: decision === "discard" ? t("移除", "Remove") : decision === "approve" ? t("批准并开启", "Approve and enable") : t("撤销授权", "Revoke approval"), cancelLabel: t("暂不开启", "Not now"),
    });
    if (!approved || !isCurrent()) { owner.policyWriting = false; return; }
    owner.policyReads.invalidate();
    setSkillImportLoading(true);
    try {
      const res = await apiFetch(`/v1/skills/${encodeURIComponent(review.skill_name)}/policy`, {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ decision, enable: decision === "approve", registry_generation: review.registry_generation,
          manifest_digest: review.manifest_digest, capability_request_digest: review.capability_request_digest }),
      });
      const body = await res.json();
      if (!isCurrent()) return;
      if (!res.ok || !body.ok) throw new Error(body.error || "skill_policy_review_failed");
      setSkillImportPreview((previous) => previous?.skill_name === review.skill_name ? { ...previous, installed: decision !== "discard", enabled: decision === "approve", admission_state: body.data.admission_state, policy_review: body.data } : previous);
      setSkillImportMessage(decision === "discard" ? t("待授权安装已移除，文件和数据已保留。", "Unapproved installation removed. Files and data are retained.") : decision === "approve" ? t("权限已批准，技能已开启。", "Permissions approved. The skill is enabled.") : t("运行授权已撤销，技能已停用。", "Runtime approval revoked. The skill is disabled."));
      await reviewSkillPolicies();
      await fetchSkillsConfig(); await fetchSkills(); await fetchSkillStore(true);
    } catch (error) {
      if (isCurrent()) setSkillImportError(formatUiError(error, t, "授权未完成，请重新查看权限后重试。", "Approval did not complete. Refresh the permission review and try again."));
    } finally { owner.policyWriting = false; if (isCurrent()) setSkillImportLoading(false); }
  };

  const uploadImportedSkillFiles = async (fileList: FileList | null) => {
    if (!isCurrent()) return;
    const files = fileList ? Array.from(fileList) as BrowserFileWithPath[] : [];
    if (files.length === 0) {
      return;
    }
    const allowNetwork = await showConfirm({
      title: t("导入时是否允许联网？", "Allow network during import?"),
      message: t(
        "默认保持离线更安全。如果技能清单声明需要下载锁定依赖或验证远程 HTTPS 端点，可以允许本次隔离安装任务联网；运行时权限不会因此自动放开。",
        "Offline is safer by default. Allow this isolated install job to use the network only if the manifest needs locked dependencies or HTTPS endpoint validation; runtime permissions are not enabled automatically.",
      ),
      confirmLabel: t("允许本次联网", "Allow this time"),
      cancelLabel: t("保持离线导入", "Import offline"),
    });
    if (!isCurrent()) return;
    const firstFile = files[0];
    const guessedBundleName =
      firstFile.webkitRelativePath?.split("/")[0]?.trim() ||
      firstFile.name.replace(/\.[^.]+$/, "").trim() ||
      "uploaded-skill";
    const formData = new FormData();
    formData.append("bundle_name", guessedBundleName);
    formData.append("enabled", "false");
    formData.append("allow_network", String(allowNetwork));
    for (const file of files) {
      const relativePath = file.webkitRelativePath?.trim() || file.name;
      formData.append("files", file, relativePath);
    }

    setSkillImportLoading(true);
    setSkillImportError(null);
    setSkillImportMessage(null);
    try {
      const res = await apiFetch(`/v1/skills/import/upload`, {
        method: "POST",
        body: formData,
      });
      const body = (await res.json()) as ApiResponse<ImportedSkillResponse>;
      if (!isCurrent()) return;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error === "skill_store_precompiled_required"
          ? skillStoreErrorMessage(body.error, t)
          : body.error || `local_skill_import_http_${res.status}`);
      }
      setSkillImportPreview(body.data);
      if (body.data.policy_review) setSkillPolicyReviews([body.data.policy_review]);
      setRecentImportedSkillName(body.data.skill_name);
      setSkillImportMessage(
        t(
          `已安装 ${body.data.display_name}，请查看并批准运行权限后开启。`,
          `${body.data.display_name} is installed. Review and approve runtime permissions before enabling it.`,
        ),
      );
      setSkillsSearchQuery(body.data.skill_name);
      await fetchSkillsConfig();
      await fetchSkills();
      await fetchSkillStore(true);
      scrollToSkillRow(body.data.skill_name);
    } catch (err) {
      if (!isCurrent()) return;
      const message = formatUiError(err, t, "技能导入失败，请检查技能包后重试。", "Skill import failed. Check the package and try again.");
      setSkillImportError(message);
    } finally {
      if (isCurrent()) {
        setSkillImportLoading(false);
        setLocalImportPickerOpen(false);
        if (folderImportInputRef.current) folderImportInputRef.current.value = "";
        if (fileImportInputRef.current) fileImportInputRef.current.value = "";
      }
    }
  };

  const mutateSkillStoreItem = async (
    skillName: string,
    installed: boolean,
    preserveConfig = true,
    preserveData = true,
    allowNetwork = false,
  ) => {
    if (!isCurrent()) return;
    setLocalSkillStoreActionName(skillName);
    setSkillStoreError(null);
    setSkillStoreMessage(null);
    try {
      const action = installed ? "install" : "remove";
      const res = await apiFetch(`/v1/skills/store/${action}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          skill_name: skillName,
          ...(installed ? { allow_network: allowNetwork } : { preserve_config: preserveConfig, preserve_data: preserveData }),
        }),
      });
      const body = (await res.json()) as ApiResponse<SkillStoreOperationResponse>;
      if (!isCurrent()) return;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(skillStoreErrorMessage(body.error, t));
      }
      const operation = body.data.operation;
      owner.previousOperation = operation;
      setSkillStoreData((current) =>
        current ? { ...current, active_operation: operation } : current,
      );
      setSkillStoreMessage(
        installed
          ? t(`${skillName} 已进入安装队列。`, `${skillName} was added to the install queue.`)
          : t(`${skillName} 已进入删除队列。`, `${skillName} was added to the removal queue.`),
      );
      await fetchSkillStore(true);
    } catch (err) {
      if (!isCurrent()) return;
      const message = formatUiError(err, t, "技能操作未完成，请稍后重试。", "The skill operation did not complete. Try again shortly.");
      setSkillStoreError(message);
    } finally {
      if (isCurrent()) setLocalSkillStoreActionName(null);
    }
  };

  const installSkillFromStore = async (skillName: string) => {
    if (!isCurrent()) return;
    const item = skillStoreData?.items.find((candidate) => candidate.name === skillName);
    const allowNetwork = item?.build_network_policy === "approval_required";
    if (allowNetwork) {
      const hostDependencies = item?.host_dependencies ?? [];
      const runtimeAssets = item?.runtime_assets ?? [];
      const confirmed = await showConfirm({
        title: item?.installed
          ? t("修复技能与本地资源", "Repair skill and local resources")
          : t("允许安装完整依赖", "Allow complete dependency installation"),
        message: t(
          `${skillName} 的清单声明安装阶段需要联网。{product_name} 会安装经过宿主白名单确认的系统依赖${hostDependencies.length ? `（${hostDependencies.join("、")}）` : ""}、技能私有依赖${runtimeAssets.length ? `和 ${runtimeAssets.length} 个本地模型资源` : ""}；系统工具在卸载技能后仍会保留，私有资源默认随技能数据保留，运行时联网权限仍单独受控。是否继续？`,
          `${skillName} declares that installation needs network access. {product_name} will install host-approved system dependencies${hostDependencies.length ? ` (${hostDependencies.join(", ")})` : ""}, private skill dependencies${runtimeAssets.length ? `, and ${runtimeAssets.length} local model resource${runtimeAssets.length === 1 ? "" : "s"}` : ""}. Shared system tools remain after skill removal, private resources follow the skill-data retention choice, and runtime network access stays separately controlled. Continue?`,
        ),
        confirmLabel: item?.installed ? t("允许修复并启用", "Allow repair and enable") : t("允许并安装", "Allow and install"),
      });
      if (!isCurrent() || !confirmed) return;
    }
    await mutateSkillStoreItem(skillName, true, true, true, allowNetwork);
  };
  const removeSkillFromStore = (skillName: string, preserveConfig = true, preserveData = true) =>
    mutateSkillStoreItem(skillName, false, preserveConfig, preserveData);

  const cancelSkillStoreOperation = async (operationId: string) => {
    if (!isCurrent()) return;
    setSkillStoreError(null);
    try {
      const res = await apiFetch(`/v1/skills/store/operations/${operationId}/cancel`, {
        method: "POST",
      });
      const body = (await res.json()) as ApiResponse<SkillStoreOperationResponse>;
      if (!isCurrent()) return;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(skillStoreErrorMessage(body.error, t));
      }
      setSkillStoreData((current) =>
        current ? { ...current, active_operation: body.data?.operation ?? null } : current,
      );
      setSkillStoreMessage(t("正在安全取消操作…", "Cancelling the operation safely…"));
    } catch (err) {
      if (!isCurrent()) return;
      const message = formatUiError(err, t, "技能操作暂时无法取消，请稍后重试。", "The skill operation could not be cancelled. Try again shortly.");
      setSkillStoreError(message);
    }
  };

  const toggleSkillEnabled = (name: string, nextEnabled: boolean) => {
    if (!isCurrent()) return;
    owner.draft.set(name, nextEnabled);
    publishSwitchDraft();
  };

  const clearSkillsConfigError = () => { if (isCurrent()) setSkillsConfigError(null); };

  return {
    skillImportSource,
    setSkillImportSource: (value: SetStateAction<string>) => { if (isCurrent()) setSkillImportSource(value); },
    skillImportLoading,
    skillImportError,
    skillImportMessage,
    skillImportPreview,
    setSkillImportPreview: (value: SetStateAction<ImportedSkillResponse | null>) => { if (isCurrent()) setSkillImportPreview(value); },
    localImportPickerOpen,
    setLocalImportPickerOpen: (value: SetStateAction<boolean>) => { if (isCurrent()) setLocalImportPickerOpen(value); },
    folderImportInputRef,
    fileImportInputRef,
    skillsConfigData,
    skillsConfigLoading,
    skillsConfigError,
    skillSwitchSaving,
    skillSwitchSaveMessage,
    hasUnsavedSkillSwitchChanges,
    managedSkills,
    filteredManagedSkills,
    filteredSkillsTool,
    filteredSkillsBase,
    filteredSkillsImage,
    filteredSkillsAudio,
    filteredSkillsMultimedia,
    filteredSkillsOther,
    normalizedSkillsSearchQuery,
    skillsSearchQuery,
    setSkillsSearchQuery: (value: SetStateAction<string>) => { if (isCurrent()) setSkillsSearchQuery(value); },
    skillItemsByName,
    configuredEnabledSkills,
    skillSwitchDraft,
    recentImportedSkillName,
    externalSkillNamesSet,
    lockedSkillNamesSet,
    toolSkillNamesSet,
    baseSkillNamesSet,
    removableSkillNamesSet,
    skillStoreData,
    skillStoreLoading,
    skillStoreError,
    skillStoreMessage,
    skillStoreActionName,
    fetchSkills,
    fetchSkillsConfig,
    fetchSkillStore,
    fetchSkillStoreDependencies,
    saveSkillSwitches,
    importExternalSkill,
    approveImportedSkillPolicy,
    reviewSkillPolicies,
    skillPolicyReviews,
    uploadImportedSkillFiles,
    installSkillFromStore,
    removeSkillFromStore,
    cancelSkillStoreOperation,
    toggleSkillEnabled,
    clearSkillsConfigError,
  };
}
