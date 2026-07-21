import { useEffect, useMemo, useRef, useState } from "react";

import {
  baseSkillNamesWithFallback,
  filterSkillNamesBySearch,
  groupSkillNames,
  isUiHiddenSkill,
  isVisibleSkillName,
  normalizeSkillSearchQuery,
  visibleSkillNames,
} from "../lib/skill-display";
import type {
  ApiResponse,
  BrowserFileWithPath,
  ImportedSkillResponse,
  SkillListItem,
  SkillsConfigResponse,
  SkillsResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type RestartSystem = () => Promise<boolean>;

export interface UseSkillsRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
}

export function useSkillsRuntime({ apiFetch, t }: UseSkillsRuntimeParams) {
  const [skillsData, setSkillsData] = useState<SkillsResponse | null>(null);
  const [skillsConfigLoading, setSkillsConfigLoading] = useState(false);
  const [skillsConfigError, setSkillsConfigError] = useState<string | null>(null);
  const [skillsConfigData, setSkillsConfigData] = useState<SkillsConfigResponse | null>(null);
  const [skillSwitchDraft, setSkillSwitchDraft] = useState<Record<string, boolean>>({});
  const [skillSwitchSaving, setSkillSwitchSaving] = useState(false);
  const [skillUninstallingName, setSkillUninstallingName] = useState<string | null>(null);
  const [skillSwitchSaveMessage, setSkillSwitchSaveMessage] = useState<string | null>(null);
  const [skillsSearchQuery, setSkillsSearchQuery] = useState("");
  const [skillImportSource, setSkillImportSource] = useState("");
  const [skillImportLoading, setSkillImportLoading] = useState(false);
  const [skillImportError, setSkillImportError] = useState<string | null>(null);
  const [skillImportMessage, setSkillImportMessage] = useState<string | null>(null);
  const [skillImportPreview, setSkillImportPreview] = useState<ImportedSkillResponse | null>(null);
  const [recentImportedSkillName, setRecentImportedSkillName] = useState<string | null>(null);
  const [localImportPickerOpen, setLocalImportPickerOpen] = useState(false);
  const folderImportInputRef = useRef<HTMLInputElement | null>(null);
  const fileImportInputRef = useRef<HTMLInputElement | null>(null);

  const fetchSkills = async () => {
    try {
      const res = await apiFetch(`/v1/skills`);
      const body = (await res.json()) as ApiResponse<SkillsResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `skills fetch failed (${res.status})`);
      }
      setSkillsData(body.data);
    } catch {
      // The visible skill switch surface is driven by /v1/skills/config; keep stale runtime metadata if /v1/skills is transiently unavailable.
    }
  };

  const fetchSkillsConfig = async () => {
    setSkillsConfigLoading(true);
    setSkillsConfigError(null);
    try {
      const res = await apiFetch(`/v1/skills/config`);
      const body = (await res.json()) as ApiResponse<SkillsConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `skill config fetch failed (${res.status})`);
      }
      setSkillsConfigData(body.data);
      const nextSwitchDraft = { ...(body.data.skill_switches || {}) };
      (body.data.locked_skill_names || body.data.core_skill_names || []).forEach((name) => {
        if (nextSwitchDraft[name] === false) nextSwitchDraft[name] = true;
      });
      setSkillSwitchDraft(nextSwitchDraft);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setSkillsConfigError(message);
    } finally {
      setSkillsConfigLoading(false);
    }
  };

  const scrollToSkillRow = (skillName: string) => {
    window.setTimeout(() => {
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
    return new Set<string>(baseSkillNamesWithFallback(skillsConfigData?.base_skill_names));
  }, [skillsConfigData?.base_skill_names]);

  const toolSkillNamesSet = useMemo(() => {
    return new Set<string>(visibleSkillNames(skillsConfigData?.tool_skill_names));
  }, [skillsConfigData?.tool_skill_names]);

  const lockedSkillNamesSet = useMemo(() => {
    const list = skillsConfigData?.locked_skill_names;
    const useList = list && list.length > 0 ? list : [...Array.from(baseSkillNamesSet), ...Array.from(toolSkillNamesSet)];
    return new Set<string>(visibleSkillNames(useList));
  }, [baseSkillNamesSet, skillsConfigData?.locked_skill_names, toolSkillNamesSet]);

  const externalSkillNamesSet = useMemo(() => {
    return new Set<string>(visibleSkillNames(skillsConfigData?.external_skill_names));
  }, [skillsConfigData?.external_skill_names]);

  const baseEnabledSkills = useMemo(() => {
    return new Set<string>(visibleSkillNames(skillsConfigData?.skills_list));
  }, [skillsConfigData]);

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

  const hasUnsavedSkillSwitchChanges = useMemo(() => {
    const persisted = skillsConfigData?.skill_switches ?? {};
    const keys = new Set<string>([
      ...Object.keys(persisted).filter(isVisibleSkillName),
      ...Object.keys(skillSwitchDraft).filter(isVisibleSkillName),
    ]);
    for (const key of keys) {
      if (persisted[key] !== skillSwitchDraft[key]) {
        return true;
      }
    }
    return false;
  }, [skillsConfigData, skillSwitchDraft]);

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

  const saveSkillSwitches = async (restartSystem?: RestartSystem) => {
    setSkillSwitchSaving(true);
    setSkillSwitchSaveMessage(null);
    setSkillsConfigError(null);
    try {
      const res = await apiFetch(`/v1/skills/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ skill_switches: skillSwitchDraft }),
      });
      const body = (await res.json()) as ApiResponse<{
        restart_required?: boolean;
      }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `skill config save failed (${res.status})`);
      }
      const restartRequired = body.data?.restart_required ?? true;
      let savedMessage = t(
        "技能开关已保存到 config.toml。",
        "Skill switches were saved to config.toml.",
      );
      if (restartRequired) {
        const confirmed = window.confirm(
          t(
            "这些变更需要重启 RustClaw 才会生效。现在就自动重启吗？",
            "These changes need a RustClaw restart to take effect. Restart now?",
          ),
        );
        if (confirmed) {
          savedMessage = t(
            "技能开关已保存，正在重启 RustClaw，请稍候。",
            "Skill switches were saved. Restarting RustClaw now.",
          );
        } else {
          savedMessage = t(
            "技能开关已保存。你可以稍后再重启 RustClaw 让它生效。",
            "Skill switches were saved. You can restart RustClaw later to apply them.",
          );
        }
        setSkillSwitchSaveMessage(savedMessage);
        await fetchSkillsConfig();
        await fetchSkills();
        if (confirmed && restartSystem) {
          const restarted = await restartSystem();
          setSkillSwitchSaveMessage(
            restarted
              ? t("RustClaw 已重启完成，技能开关现在已经生效。", "RustClaw restarted successfully. Skill switches are now active.")
              : t("重启请求已经发出，请稍后刷新确认技能开关是否生效。", "Restart was requested. Please refresh shortly to confirm the skill switches are active."),
          );
        }
        return;
      }
      setSkillSwitchSaveMessage(savedMessage);
      await fetchSkillsConfig();
      await fetchSkills();
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setSkillsConfigError(message);
    } finally {
      setSkillSwitchSaving(false);
    }
  };

  const importExternalSkill = async () => {
    const source = skillImportSource.trim();
    if (!source) {
      setSkillImportError(t("请先输入 skill 链接或本地目录。", "Please enter a skill link or local bundle path first."));
      return;
    }
    setSkillImportLoading(true);
    setSkillImportError(null);
    setSkillImportMessage(null);
    try {
      const res = await apiFetch(`/v1/skills/import`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ source, enabled: true }),
      });
      const body = (await res.json()) as ApiResponse<ImportedSkillResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `skill import failed (${res.status})`);
      }
      setSkillImportPreview(body.data);
      setRecentImportedSkillName(body.data.skill_name);
      setSkillImportMessage(
        t(
          `已导入 ${body.data.display_name}。下一步：在下面找到高亮的 ${body.data.skill_name}，点“设为开启”，再点“保存开关”。`,
          `${body.data.display_name} was imported. Next: find the highlighted ${body.data.skill_name} below, choose Enable, then click Save Switches.`,
        ),
      );
      setSkillsSearchQuery(body.data.skill_name);
      await fetchSkillsConfig();
      await fetchSkills();
      scrollToSkillRow(body.data.skill_name);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setSkillImportError(message);
    } finally {
      setSkillImportLoading(false);
    }
  };

  const uploadImportedSkillFiles = async (fileList: FileList | null) => {
    const files = fileList ? Array.from(fileList) as BrowserFileWithPath[] : [];
    if (files.length === 0) {
      return;
    }
    const firstFile = files[0];
    const guessedBundleName =
      firstFile.webkitRelativePath?.split("/")[0]?.trim() ||
      firstFile.name.replace(/\.[^.]+$/, "").trim() ||
      "uploaded-skill";
    const formData = new FormData();
    formData.append("bundle_name", guessedBundleName);
    formData.append("enabled", "true");
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
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `local skill import failed (${res.status})`);
      }
      setSkillImportPreview(body.data);
      setRecentImportedSkillName(body.data.skill_name);
      setSkillImportMessage(
        t(
          `已导入 ${body.data.display_name}。下一步：在下面找到高亮的 ${body.data.skill_name}，点“设为开启”，再点“保存开关”。`,
          `${body.data.display_name} was imported. Next: find the highlighted ${body.data.skill_name} below, choose Enable, then click Save Switches.`,
        ),
      );
      setSkillsSearchQuery(body.data.skill_name);
      await fetchSkillsConfig();
      await fetchSkills();
      scrollToSkillRow(body.data.skill_name);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setSkillImportError(message);
    } finally {
      setSkillImportLoading(false);
      setLocalImportPickerOpen(false);
      if (folderImportInputRef.current) folderImportInputRef.current.value = "";
      if (fileImportInputRef.current) fileImportInputRef.current.value = "";
    }
  };

  const uninstallExternalSkill = async (skillName: string) => {
    const confirmed = window.confirm(
      t(
        `卸载 ${skillName} 后，会删除它导入进来的文件和注册信息。确认继续吗？`,
        `Uninstall ${skillName}? Its imported files and registration will be removed.`,
      ),
    );
    if (!confirmed) return;
    setSkillUninstallingName(skillName);
    setSkillImportError(null);
    setSkillImportMessage(null);
    setSkillsConfigError(null);
    try {
      const res = await apiFetch(`/v1/skills/uninstall`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ skill_name: skillName }),
      });
      const body = (await res.json()) as ApiResponse<{ skill_name: string }>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `skill uninstall failed (${res.status})`);
      }
      if (recentImportedSkillName === skillName) {
        setRecentImportedSkillName(null);
      }
      if (skillImportPreview?.skill_name === skillName) {
        setSkillImportPreview(null);
      }
      if (skillsSearchQuery.trim().toLowerCase() === skillName.toLowerCase()) {
        setSkillsSearchQuery("");
      }
      setSkillImportMessage(
        t(
          `${skillName} 已卸载，现在已经从技能列表里移除。`,
          `${skillName} was uninstalled and removed from the skill list.`,
        ),
      );
      await fetchSkillsConfig();
      await fetchSkills();
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setSkillsConfigError(message);
    } finally {
      setSkillUninstallingName(null);
    }
  };

  const toggleSkillEnabled = (name: string, nextEnabled: boolean) => {
    if (isUiHiddenSkill(name)) return;
    if (lockedSkillNamesSet.has(name)) return;
    setSkillSwitchDraft((prev) => {
      const next = { ...prev };
      const baseEnabled = baseEnabledSkills.has(name);
      if (nextEnabled === baseEnabled) {
        delete next[name];
      } else {
        next[name] = nextEnabled;
      }
      return next;
    });
  };

  const clearSkillsConfigError = () => setSkillsConfigError(null);

  return {
    skillImportSource,
    setSkillImportSource,
    skillImportLoading,
    skillImportError,
    skillImportMessage,
    skillImportPreview,
    setSkillImportPreview,
    localImportPickerOpen,
    setLocalImportPickerOpen,
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
    setSkillsSearchQuery,
    skillItemsByName,
    configuredEnabledSkills,
    skillSwitchDraft,
    recentImportedSkillName,
    externalSkillNamesSet,
    lockedSkillNamesSet,
    toolSkillNamesSet,
    baseSkillNamesSet,
    skillUninstallingName,
    fetchSkills,
    fetchSkillsConfig,
    saveSkillSwitches,
    importExternalSkill,
    uploadImportedSkillFiles,
    uninstallExternalSkill,
    toggleSkillEnabled,
    clearSkillsConfigError,
  };
}
