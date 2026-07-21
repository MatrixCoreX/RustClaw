import type { SkillListItem } from "../types/api";

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

/** 基本技能（与后端 base_skill_names 一致），API 未返回时用此兜底 */
const FALLBACK_BASE_SKILL_NAMES = [
  "run_cmd",
  "fs_basic",
  "config_basic",
  "read_file",
  "write_file",
  "list_dir",
  "make_dir",
  "remove_file",
  "schedule",
  "extension_manager",
  "kb",
  "system_basic",
  "process_basic",
  "config_guard",
  "fs_search",
  "git_basic",
  "service_control",
  "archive_basic",
];

const SKILL_SUMMARY: Record<string, { zh: string; en: string }> = {
  archive_basic: { zh: "压缩、解压和整理归档文件。", en: "Compress, extract, and organize archives." },
  audio_synthesize: { zh: "把文字转成语音。", en: "Turn text into speech." },
  audio_transcribe: { zh: "把语音转成文字。", en: "Turn speech into text." },
  browser_web: { zh: "打开网页并提取页面内容。", en: "Open webpages and extract page content." },
  config_guard: { zh: "检查配置是否缺项或明显不合理。", en: "Check configs for missing or risky values." },
  config_basic: { zh: "读取并校验结构化配置字段。", en: "Read and validate structured config fields." },
  crypto: { zh: "查看币价、账户、订单和交易相关能力。", en: "Handle crypto quotes, balances, orders, and trading tasks." },
  db_basic: { zh: "查看和处理数据库里的基础数据。", en: "Inspect and work with basic database data." },
  doc_parse: { zh: "解析文档内容，提取可读文本。", en: "Parse documents and extract readable text." },
  docker_basic: { zh: "查看和操作 Docker 容器、镜像与服务。", en: "Inspect and control Docker containers, images, and services." },
  extension_manager: { zh: "管理外部扩展技能的接入。", en: "Manage external skill extensions." },
  fs_search: { zh: "在文件里搜索关键词或定位内容。", en: "Search files and locate content." },
  fs_basic: { zh: "处理文件、目录、路径事实和文本搜索。", en: "Handle files, directories, path facts, and text search." },
  git_basic: { zh: "查看提交、分支和常见 Git 操作。", en: "Inspect commits, branches, and common Git actions." },
  health_check: { zh: "快速检查系统和服务是否正常。", en: "Run quick health checks for the system and services." },
  http_basic: { zh: "发起 HTTP 请求并查看返回结果。", en: "Send HTTP requests and inspect responses." },
  image_edit: { zh: "修改、扩图或局部编辑图片。", en: "Edit, extend, or patch images." },
  image_generate: { zh: "根据描述生成图片。", en: "Generate images from prompts." },
  image_vision: { zh: "识别和理解图片内容。", en: "Analyze and understand image content." },
  install_module: { zh: "安装或补齐项目依赖模块。", en: "Install or restore project dependencies." },
  invest_copy: { zh: "整理调研材料并生成投资文案。", en: "Turn research material into investment copy." },
  kb: { zh: "查询和维护本地知识库内容。", en: "Query and maintain local knowledge base content." },
  list_dir: { zh: "查看目录结构和文件列表。", en: "List directories and files." },
  log_analyze: { zh: "分析日志，定位错误和异常。", en: "Analyze logs and find issues." },
  make_dir: { zh: "创建新目录。", en: "Create directories." },
  map_merchant: { zh: "按位置推荐商家或地点。", en: "Recommend nearby merchants or places." },
  music_generate: { zh: "根据描述和歌词生成音乐。", en: "Generate music from prompts and lyrics." },
  package_manager: { zh: "处理包管理、安装与版本问题。", en: "Manage packages, installs, and versions." },
  photo_organize: { zh: "整理照片文件并生成分类建议。", en: "Organize photos and suggest categories." },
  process_basic: { zh: "查看和管理进程。", en: "Inspect and manage processes." },
  read_file: { zh: "读取文件内容。", en: "Read file contents." },
  remove_file: { zh: "删除文件。", en: "Remove files." },
  rss_fetch: { zh: "抓取和整理 RSS 资讯。", en: "Fetch and summarize RSS feeds." },
  run_cmd: { zh: "运行命令行命令。", en: "Run shell commands." },
  schedule: { zh: "创建、查询或管理定时任务。", en: "Create, inspect, or manage scheduled tasks." },
  service_control: { zh: "启动、停止或重启服务。", en: "Start, stop, or restart services." },
  stock: { zh: "股票市场技能。", en: "Stock market skill." },
  task_control: { zh: "查看、取消当前会话未完成任务。", en: "List and cancel unfinished tasks in the current chat." },
  system_basic: { zh: "查看系统信息和基础环境。", en: "Inspect system information and environment basics." },
  transform: { zh: "转换文本、数据或文件格式。", en: "Transform text, data, or file formats." },
  video_generate: { zh: "根据描述或图片生成视频。", en: "Generate videos from prompts or images." },
  weather: { zh: "查询天气和基础预报信息。", en: "Check weather and basic forecasts." },
  web_search_extract: { zh: "搜索网页并提取关键内容。", en: "Search the web and extract key content." },
  write_file: { zh: "写入或修改文件内容。", en: "Write or update file contents." },
  x: { zh: "xurl调用技能。", en: "xurl invocation skill." },
};

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

export function baseSkillNamesWithFallback(names?: string[] | null): string[] {
  const source = names && names.length > 0 ? names : FALLBACK_BASE_SKILL_NAMES;
  return visibleSkillNames(source);
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

export function skillDescription(name: string, lang: UiLanguage, itemDescription?: string | null): string {
  const description = itemDescription?.trim();
  if (description) return description;
  const summary = SKILL_SUMMARY[name];
  if (summary) return copy(lang, summary.zh, summary.en);
  return copy(lang, "该技能无简短说明。", "No short description for this skill.");
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
