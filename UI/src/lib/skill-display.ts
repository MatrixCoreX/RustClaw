import type { PlannerCapabilityDisplayItem, SkillListItem } from "../types/api";

export type UiLanguage = "zh" | "en";

export interface SkillGroups {
  tool: string[];
  image: string[];
  audio: string[];
  multimedia: string[];
  base: string[];
  other: string[];
}

const UI_HIDDEN_SKILLS = new Set<string>(["chat"]);

function copy(lang: UiLanguage, zh: string, en: string): string {
  return lang === "zh" ? zh : en;
}

function sortSkillNames(names: string[]): string[] {
  return [...names].sort((a, b) => a.localeCompare(b));
}

export function isUiHiddenSkill(name?: string | null): boolean {
  return Boolean(name && UI_HIDDEN_SKILLS.has(name));
}

export function isVisibleSkillName(name?: string | null): name is string {
  return Boolean(name && !isUiHiddenSkill(name));
}

export function visibleSkillNames(names?: string[] | null): string[] {
  return (names ?? []).filter(isVisibleSkillName);
}

export function baseSkillNamesFromRegistry(names?: string[] | null): string[] {
  return visibleSkillNames(names);
}

export function normalizeSkillSearchQuery(query: string): string {
  return query.trim().toLowerCase();
}

export function filterSkillNamesBySearch(names: string[], normalizedQuery: string): string[] {
  if (!normalizedQuery) return names;
  return names.filter((name) => name.toLowerCase().includes(normalizedQuery));
}

export function groupSkillNames(
  managedSkills: string[],
  baseSkillNamesSet: ReadonlySet<string>,
  toolSkillNamesSet: ReadonlySet<string>,
): SkillGroups {
  const isMultimedia = (name: string) => name.startsWith("video_") || name.startsWith("music_");
  return {
    tool: sortSkillNames(managedSkills.filter((name) => toolSkillNamesSet.has(name))),
    image: sortSkillNames(managedSkills.filter((name) => name.startsWith("image_") && !toolSkillNamesSet.has(name))),
    audio: sortSkillNames(managedSkills.filter((name) => name.startsWith("audio_") && !toolSkillNamesSet.has(name))),
    multimedia: sortSkillNames(managedSkills.filter((name) => isMultimedia(name) && !toolSkillNamesSet.has(name))),
    base: sortSkillNames(managedSkills.filter((name) => baseSkillNamesSet.has(name) && !toolSkillNamesSet.has(name))),
    other: sortSkillNames(
      managedSkills.filter(
        (name) =>
          !name.startsWith("image_") &&
          !name.startsWith("audio_") &&
          !isMultimedia(name) &&
          !baseSkillNamesSet.has(name) &&
          !toolSkillNamesSet.has(name),
      ),
    ),
  };
}

export function skillDescription(lang: UiLanguage, itemDescription?: string | null): string {
  const description = itemDescription?.trim();
  if (description) return description;
  return copy(lang, "该技能暂无说明。", "No description is available for this skill.");
}

function capabilityTopic(detail: PlannerCapabilityDisplayItem, lang: UiLanguage): string {
  return detail.description?.trim() || skillPlannerCapabilityLabel(detail.capability, lang);
}

function requiredFieldHint(detail: PlannerCapabilityDisplayItem, lang: UiLanguage): string {
  const required = (detail.required ?? []).filter(Boolean);
  if (required.length === 0) return "";
  return copy(
    lang,
    `，并提供 ${required.join("、")}`,
    ` and provide ${required.join(", ")}`,
  );
}

function capabilityExample(detail: PlannerCapabilityDisplayItem, lang: UiLanguage): string {
  const topic = capabilityTopic(detail, lang);
  const required = requiredFieldHint(detail, lang);
  switch (detail.effect) {
    case "mutate":
      return copy(
        lang,
        `请完成这项更改：${topic}${required}；需要授权时先让我确认。`,
        `Make this change: ${topic}${required}; ask for confirmation when authorization is required.`,
      );
    case "external":
      return copy(
        lang,
        `请执行这项外部操作：${topic}${required}；先说明实际影响。`,
        `Perform this external action: ${topic}${required}; explain the real-world impact first.`,
      );
    case "validate":
      return copy(
        lang,
        `请检查并验证：${topic}${required}，然后告诉我发现的问题。`,
        `Check and validate this: ${topic}${required}, then report any issues found.`,
      );
    default:
      return copy(
        lang,
        `请查看并告诉我实际结果：${topic}${required}。`,
        `Inspect this and tell me the observed result: ${topic}${required}.`,
      );
  }
}

export function skillUsageExamples(item: SkillListItem | undefined, lang: UiLanguage): readonly string[] {
  if (!item) return [];
  const examples = (item.planner_capability_details ?? [])
    .slice(0, 5)
    .map((detail) => capabilityExample(detail, lang));
  const description = skillDescription(lang, item.description);
  const generic = lang === "zh"
    ? [
        `请帮我处理这项需求：${description}`,
        `先检查这项能力是否可用，并给出只读预览：${description}`,
        `根据实际执行结果完成任务，并告诉我下一步：${description}`,
      ]
    : [
        `Help me with this request: ${description}`,
        `Check whether this capability is available and provide a read-only preview: ${description}`,
        `Complete the task from observed execution results and tell me the next step: ${description}`,
      ];
  for (const example of generic) {
    if (examples.length >= 3) break;
    if (!examples.includes(example)) examples.push(example);
  }
  return examples.slice(0, 5);
}

export function skillRiskLabel(risk: string | null | undefined, lang: UiLanguage): string {
  switch ((risk || "").toLowerCase()) {
    case "low":
      return copy(lang, "低风险", "Low risk");
    case "medium":
      return copy(lang, "中风险", "Medium risk");
    case "high":
      return copy(lang, "高风险", "High risk");
    default:
      return copy(lang, "风险未声明", "Risk not declared");
  }
}

export function skillCapabilityLabel(capability: string, lang: UiLanguage): string {
  switch (capability) {
    case "llm":
      return copy(lang, "会调用模型", "Uses model");
    case "net":
      return copy(lang, "访问网络", "Network");
    case "fs.read":
      return copy(lang, "读取文件", "Reads files");
    case "fs.write":
      return copy(lang, "改写文件", "Changes files");
    case "exec":
      return copy(lang, "运行命令", "Runs commands");
    case "exec.sudo":
      return copy(lang, "可提权执行", "Can use sudo");
    default:
      return capability.startsWith("secrets.") ? copy(lang, "需要密钥", "Needs secret") : capability;
  }
}

export function formatCapabilityToken(token: string): string {
  return token
    .split(".")
    .map((part) => part.replace(/_/g, " "))
    .join(" / ");
}

export function skillPlannerCapabilityLabel(capability: string, lang: UiLanguage): string {
  const [domain, ...rest] = capability.split(".");
  const readable = formatCapabilityToken(rest.join(".") || capability);
  const domainLabel = {
    filesystem: copy(lang, "文件", "Files"),
    config: copy(lang, "配置", "Config"),
    system: copy(lang, "系统", "System"),
    database: copy(lang, "数据库", "Database"),
  }[domain];
  return domainLabel ? `${domainLabel}: ${readable}` : formatCapabilityToken(capability);
}

export function skillIsolationLabels(item: SkillListItem | undefined, lang: UiLanguage): string[] {
  const policies = item?.planner_capability_policies ?? [];
  const labels: string[] = [];
  const push = (label: string) => {
    if (!labels.includes(label)) labels.push(label);
  };
  for (const policy of policies) {
    switch (policy.isolation_profile) {
      case "read_only":
        push(copy(lang, "只读", "Read-only"));
        break;
      case "local_current_workspace":
        push(copy(lang, "当前工作区", "Current workspace"));
        break;
      case "local_worktree":
        push(copy(lang, "独立工作树", "Separate worktree"));
        break;
      case "local_temp_workspace":
        push(copy(lang, "临时工作区", "Temp workspace"));
        break;
      case "remote_executor":
        push(copy(lang, "外部执行", "External execution"));
        break;
    }
    if (policy.network_access) push(copy(lang, "访问网络", "Network"));
    if (policy.filesystem_write) push(copy(lang, "可改文件", "Can edit files"));
    if (policy.external_publish) push(copy(lang, "可对外发布", "Can publish"));
    if (policy.credential_access) push(copy(lang, "使用密钥", "Uses keys"));
    if (policy.subprocess) push(copy(lang, "运行子进程", "Runs subprocesses"));
    if (policy.package_install) push(copy(lang, "安装软件包", "Installs packages"));
    if (policy.privilege_escalation) push(copy(lang, "可能提权", "May elevate privileges"));
  }
  return labels;
}

export function skillRuntimeIssue(item: SkillListItem | undefined, lang: UiLanguage): string | null {
  if (!item || item.runtime_available !== false) return null;
  if (item.unavailable_reason === "skill_disabled" || item.enabled === false) {
    return copy(lang, "该技能当前未开启", "This skill is currently disabled");
  }
  if (item.unsupported_os?.length) {
    return copy(
      lang,
      `当前系统 ${item.current_os || "unknown"} 不在支持列表：${item.unsupported_os.join(", ")}`,
      `Current OS ${item.current_os || "unknown"} is not supported: ${item.unsupported_os.join(", ")}`,
    );
  }
  if (item.missing_required_bins?.length) {
    return copy(
      lang,
      `缺少本地工具：${item.missing_required_bins.join(", ")}`,
      `Missing local tools: ${item.missing_required_bins.join(", ")}`,
    );
  }
  return copy(lang, "当前设备暂不可用", "Unavailable on this device");
}
