import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  AlertCircle,
  BellRing,
  ChevronDown,
  Database,
  FileText,
  LayoutDashboard,
  Loader2,
  MessageCircle,
  Moon,
  RefreshCw,
  Sparkles,
  SquareTerminal,
  Server,
  Sun,
  Timer,
  Trash2,
  Wrench,
  X,
} from "lucide-react";
import ReactMarkdown from "react-markdown";

interface ApiResponse<T> {
  ok: boolean;
  data?: T;
  error?: string;
}

interface HealthResponse {
  version: string;
  queue_length: number;
  worker_state: string;
  uptime_seconds: number;
  memory_rss_bytes?: number | null;
  running_length: number;
  task_timeout_seconds: number;
  running_oldest_age_seconds: number;
  telegramd_healthy?: boolean | null;
  telegramd_process_count?: number | null;
  telegramd_memory_rss_bytes?: number | null;
  whatsappd_healthy?: boolean | null;
  whatsappd_process_count?: number | null;
  whatsappd_memory_rss_bytes?: number | null;
  telegram_bot_healthy?: boolean | null;
  telegram_bot_process_count?: number | null;
  telegram_bot_memory_rss_bytes?: number | null;
  whatsapp_cloud_healthy?: boolean | null;
  whatsapp_cloud_process_count?: number | null;
  whatsapp_cloud_memory_rss_bytes?: number | null;
  whatsapp_web_healthy?: boolean | null;
  whatsapp_web_process_count?: number | null;
  whatsapp_web_memory_rss_bytes?: number | null;
  feishud_healthy?: boolean | null;
  feishud_process_count?: number | null;
  feishud_memory_rss_bytes?: number | null;
  larkd_healthy?: boolean | null;
  larkd_process_count?: number | null;
  larkd_memory_rss_bytes?: number | null;
  user_count?: number;
  bound_channel_count?: number;
  future_adapters_enabled?: string[];
}

interface TaskQueryResponse {
  task_id: string;
  status: "queued" | "running" | "succeeded" | "failed" | "canceled" | "timeout";
  result_json?: unknown | null;
  error_text?: string | null;
}

interface SubmitTaskResponse {
  task_id: string;
}

interface LocalInteractionContextResponse {
  user_id: number;
  chat_id: number;
  role: string;
}

interface AuthIdentityResponse extends LocalInteractionContextResponse {
  user_key: string;
}

interface AuthKeyListItem {
  key_id: number;
  user_key_masked: string;
  role: string;
  enabled: boolean;
  created_at: string;
  last_used_at: string | null;
}

interface ResolveChannelBindingResponse {
  bound: boolean;
  identity?: AuthIdentityResponse | null;
}

interface SkillsResponse {
  skills: string[];
  skill_runner_path?: string;
}

interface SkillsConfigResponse {
  config_path: string;
  skills_list: string[];
  skill_switches: Record<string, boolean>;
  managed_skills: string[];
  /** 基本技能（由 tool 转换的系统基本能力），UI 归类为「基本技能」，不建议关闭 */
  base_skill_names?: string[];
  external_skill_names?: string[];
  effective_enabled_skills_preview: string[];
  runtime_enabled_skills: string[];
  restart_required: boolean;
}

interface ImportedSkillResponse {
  skill_name: string;
  display_name: string;
  description: string;
  external_kind: string;
  bundle_dir: string;
  entry_file: string;
  runtime?: string | null;
  require_bins: string[];
  require_py_modules: string[];
  prompt_file: string;
  source: string;
}

interface LlmVendorOption {
  name: string;
  default_model: string;
  models: string[];
  base_url: string;
  api_key_configured: boolean;
  api_key_masked?: string | null;
}

interface LlmRuntimeInfo {
  vendor: string;
  model: string;
  provider_name?: string;
  provider_type?: string;
}

interface LlmConfigResponse {
  config_path: string;
  selected_vendor: string;
  selected_model: string;
  vendors: LlmVendorOption[];
  runtime?: LlmRuntimeInfo | null;
  restart_required: boolean;
}

interface ModelConfigItem {
  vendor: string;
  model: string;
  base_url?: string;
  api_key?: string;
}

interface ModelConfigResponse {
  llm: ModelConfigItem;
  image_edit: ModelConfigItem;
  image_generation: ModelConfigItem;
  image_vision: ModelConfigItem;
  audio_transcribe: ModelConfigItem;
  audio_synthesize: ModelConfigItem;
  restart_required: boolean;
}

interface LogLatestResponse {
  file: string;
  lines: number;
  text: string;
}

interface WhatsappWebLoginStatus {
  connected?: boolean;
  qr_ready?: boolean;
  qr_data_url?: string | null;
  last_update_ts?: number;
  last_error?: string | null;
}

interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  text: string;
  ts: number;
  images?: Array<{ name: string; dataUrl: string }>;
}

type BrowserFileWithPath = File & {
  webkitRelativePath?: string;
};

interface ChatImageAttachment {
  name: string;
  dataUrl: string;
}

interface AdapterHealthRow {
  key: string;
  label: string;
  serviceName: "telegramd" | "whatsappd" | "whatsapp_webd" | "feishud" | "larkd";
  healthy: boolean | null | undefined;
  processCount: number | null | undefined;
  memoryRssBytes: number | null | undefined;
}

interface ChannelPreset {
  summary: string;
  userHint: string;
  chatHint: string;
  exampleUser: string;
  exampleChat: string;
  note: string;
}

interface ServiceStatusRow extends AdapterHealthRow {
  category: "ready" | "attention" | "stopped" | "unknown";
  statusLabel: string;
  detail: string;
}

type ChannelName = "telegram" | "whatsapp" | "ui" | "feishu" | "lark";
type ConsolePage = "dashboard" | "chat" | "services" | "channels" | "models" | "skills" | "logs" | "tasks";
type ThemeMode = "dark" | "light";
const CONSOLE_PAGES: ConsolePage[] = ["dashboard", "chat", "services", "channels", "models", "skills", "logs", "tasks"];

const UI_HIDDEN_SKILLS = new Set<string>(["chat"]);
/** 基本技能（与后端 base_skill_names 一致，由 tool 转换），API 未返回时用此兜底 */
const FALLBACK_BASE_SKILL_NAMES = ["run_cmd", "read_file", "write_file", "list_dir", "make_dir", "remove_file"];
const SKILL_SUMMARY: Record<string, { zh: string; en: string }> = {
  archive_basic: { zh: "压缩、解压和整理归档文件。", en: "Compress, extract, and organize archives." },
  audio_synthesize: { zh: "把文字转成语音。", en: "Turn text into speech." },
  audio_transcribe: { zh: "把语音转成文字。", en: "Turn speech into text." },
  config_guard: { zh: "检查配置是否缺项或明显不合理。", en: "Check configs for missing or risky values." },
  crypto: { zh: "查看币价、账户、订单和交易相关能力。", en: "Handle crypto quotes, balances, orders, and trading tasks." },
  db_basic: { zh: "查看和处理数据库里的基础数据。", en: "Inspect and work with basic database data." },
  docker_basic: { zh: "查看和操作 Docker 容器、镜像与服务。", en: "Inspect and control Docker containers, images, and services." },
  fs_search: { zh: "在文件里搜索关键词或定位内容。", en: "Search files and locate content." },
  git_basic: { zh: "查看提交、分支和常见 Git 操作。", en: "Inspect commits, branches, and common Git actions." },
  health_check: { zh: "快速检查系统和服务是否正常。", en: "Run quick health checks for the system and services." },
  http_basic: { zh: "发起 HTTP 请求并查看返回结果。", en: "Send HTTP requests and inspect responses." },
  image_edit: { zh: "修改、扩图或局部编辑图片。", en: "Edit, extend, or patch images." },
  image_generate: { zh: "根据描述生成图片。", en: "Generate images from prompts." },
  image_vision: { zh: "识别和理解图片内容。", en: "Analyze and understand image content." },
  install_module: { zh: "安装或补齐项目依赖模块。", en: "Install or restore project dependencies." },
  list_dir: { zh: "查看目录结构和文件列表。", en: "List directories and files." },
  log_analyze: { zh: "分析日志，定位错误和异常。", en: "Analyze logs and find issues." },
  make_dir: { zh: "创建新目录。", en: "Create directories." },
  package_manager: { zh: "处理包管理、安装与版本问题。", en: "Manage packages, installs, and versions." },
  process_basic: { zh: "查看和管理进程。", en: "Inspect and manage processes." },
  read_file: { zh: "读取文件内容。", en: "Read file contents." },
  remove_file: { zh: "删除文件。", en: "Remove files." },
  rss_fetch: { zh: "抓取和整理 RSS 资讯。", en: "Fetch and summarize RSS feeds." },
  run_cmd: { zh: "运行命令行命令。", en: "Run shell commands." },
  service_control: { zh: "启动、停止或重启服务。", en: "Start, stop, or restart services." },
  stock: { zh: "股票市场技能。", en: "Stock market skill." },
  task_control: { zh: "查看、取消当前会话未完成任务。", en: "List and cancel unfinished tasks in the current chat." },
  system_basic: { zh: "查看系统信息和基础环境。", en: "Inspect system information and environment basics." },
  write_file: { zh: "写入或修改文件内容。", en: "Write or update file contents." },
  x: { zh: "xurl调用技能。", en: "xurl invocation skill." },
};

const STORAGE_KEYS = {
  baseUrl: "rustclaw.monitor.baseUrl",
  userKey: "rustclaw.monitor.userKey",
  polling: "rustclaw.monitor.pollingSeconds",
  queueWarn: "rustclaw.monitor.queueWarn",
  ageWarn: "rustclaw.monitor.ageWarnSeconds",
  lang: "rustclaw.monitor.lang",
  currentPage: "rustclaw.monitor.currentPage",
  themeMode: "rustclaw.monitor.themeMode",
} as const;

/** 根据当前页面地址推断 clawd API 的默认 baseUrl；获取不到主机名时用 127.0.0.1 */
function getDefaultClawdBaseUrl(): string {
  if (typeof window === "undefined" || !window.location) return "http://127.0.0.1:8787";
  const loc = window.location;
  let hostname = (loc.hostname && loc.hostname.trim()) || "";
  if (!hostname && loc.host) {
    hostname = loc.host.split(":")[0]?.trim() || "";
  }
  const protocol = loc.protocol && loc.protocol !== "file:" ? loc.protocol : "http:";
  if (hostname) return `${protocol}//${hostname}:8787`;
  return "http://127.0.0.1:8787";
}

function readNumber(key: string, fallback: number): number {
  const raw = window.localStorage.getItem(key);
  if (!raw) return fallback;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function formatBytes(value?: number | null): string {
  if (value == null || Number.isNaN(value)) return "--";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = value;
  let idx = 0;
  while (size >= 1024 && idx < units.length - 1) {
    size /= 1024;
    idx += 1;
  }
  return `${size.toFixed(idx === 0 ? 0 : 2)} ${units[idx]}`;
}

function formatDuration(totalSeconds?: number): string {
  if (typeof totalSeconds !== "number" || Number.isNaN(totalSeconds)) return "--";
  const days = Math.floor(totalSeconds / 86400);
  const hours = Math.floor((totalSeconds % 86400) / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = Math.floor(totalSeconds % 60);
  if (days > 0) return `${days}d ${hours}h ${minutes}m`;
  if (hours > 0) return `${hours}h ${minutes}m ${seconds}s`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

function toLocalTime(ts: number): string {
  return new Date(ts).toLocaleTimeString();
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function fileToDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") resolve(reader.result);
      else reject(new Error("读取图片失败"));
    };
    reader.onerror = () => reject(new Error("读取图片失败"));
    reader.readAsDataURL(file);
  });
}

function formatVisionResultText(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed.startsWith("{")) return raw;
  try {
    const parsed = JSON.parse(trimmed) as {
      summary?: unknown;
      objects?: unknown;
      visible_text?: unknown;
      uncertainties?: unknown;
    };
    const lines: string[] = [];
    if (typeof parsed.summary === "string" && parsed.summary.trim()) {
      lines.push(parsed.summary.trim());
    }
    if (Array.isArray(parsed.objects) && parsed.objects.length > 0) {
      lines.push(`Objects: ${parsed.objects.join(", ")}`);
    }
    if (Array.isArray(parsed.visible_text) && parsed.visible_text.length > 0) {
      lines.push(`Visible text: ${parsed.visible_text.join(" ; ")}`);
    }
    if (Array.isArray(parsed.uncertainties) && parsed.uncertainties.length > 0) {
      lines.push(`Uncertainties: ${parsed.uncertainties.join(" ; ")}`);
    }
    return lines.length > 0 ? lines.join("\n\n") : raw;
  } catch {
    return raw;
  }
}

function maskStoredKey(value: string, keep = 6): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  const visible = trimmed.slice(0, Math.max(1, keep));
  return `${visible}${"*".repeat(Math.max(4, trimmed.length - visible.length))}`;
}

function extractTaskText(result: TaskQueryResponse): string {
  if (result.result_json && typeof result.result_json === "object") {
    const maybeText = (result.result_json as { text?: unknown }).text;
    if (typeof maybeText === "string" && maybeText.trim()) {
      return maybeText;
    }
  }
  if (result.error_text) {
    return result.error_text;
  }
  return JSON.stringify(result.result_json ?? null, null, 2);
}

function QuickActionCard({
  title,
  desc,
  cta,
  onClick,
  icon,
}: {
  title: string;
  desc?: string;
  cta: string;
  onClick: () => void;
  icon: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="rounded-2xl border border-white/10 bg-black/20 px-4 py-3.5 text-left transition hover:bg-white/8"
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2 text-sm font-semibold text-white/90">
            <span className="theme-icon-soft">{icon}</span>
            <span>{title}</span>
          </div>
          {desc ? <p className="mt-2 text-sm leading-6 text-white/60">{desc}</p> : null}
        </div>
        <span className="pt-0.5 text-xs font-medium text-[#ffb08a]">{cta}</span>
      </div>
    </button>
  );
}

export default function App() {
  const [lang, setLang] = useState<"zh" | "en">(() => {
    const saved = window.localStorage.getItem(STORAGE_KEYS.lang);
    return saved === "en" ? "en" : "zh";
  });
  const [themeMode, setThemeMode] = useState<ThemeMode>(() => {
    const saved = window.localStorage.getItem(STORAGE_KEYS.themeMode);
    return saved === "light" ? "light" : "dark";
  });
  const [baseUrl, setBaseUrl] = useState(() => {
    const saved = window.localStorage.getItem(STORAGE_KEYS.baseUrl);
    if (saved != null && saved.trim() !== "") return saved.trim();
    return getDefaultClawdBaseUrl();
  });
  const apiBase = baseUrl || getDefaultClawdBaseUrl();
  const [uiKey, setUiKey] = useState(() => window.localStorage.getItem(STORAGE_KEYS.userKey)?.trim() ?? "");
  const [uiKeyDraft, setUiKeyDraft] = useState("");
  const [uiAuthReady, setUiAuthReady] = useState(false);
  const [uiAuthLoading, setUiAuthLoading] = useState(false);
  const [uiAuthError, setUiAuthError] = useState<string | null>(null);
  const [authIdentity, setAuthIdentity] = useState<AuthIdentityResponse | null>(null);
  const [authMeLoading, setAuthMeLoading] = useState(false);
  const [authMeError, setAuthMeError] = useState<string | null>(null);
  const [pollingSeconds, setPollingSeconds] = useState(() => {
    return readNumber(STORAGE_KEYS.polling, 5);
  });
  const [queueWarn, setQueueWarn] = useState(() => {
    return readNumber(STORAGE_KEYS.queueWarn, 20);
  });
  const [ageWarnSeconds, setAgeWarnSeconds] = useState(() => {
    return readNumber(STORAGE_KEYS.ageWarn, 600);
  });
  const [loading, setLoading] = useState(false);
  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [skillsLoading, setSkillsLoading] = useState(false);
  const [skillsError, setSkillsError] = useState<string | null>(null);
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
  const [llmConfigLoading, setLlmConfigLoading] = useState(false);
  const [llmConfigError, setLlmConfigError] = useState<string | null>(null);
  const [llmConfigData, setLlmConfigData] = useState<LlmConfigResponse | null>(null);
  const [llmDraftVendor, setLlmDraftVendor] = useState("");
  const [llmDraftModel, setLlmDraftModel] = useState("");
  const [llmConfigSaving, setLlmConfigSaving] = useState(false);
  const [llmConfigSaveMessage, setLlmConfigSaveMessage] = useState<string | null>(null);
  const [llmDraftBaseUrl, setLlmDraftBaseUrl] = useState("");
  const [llmDraftApiKey, setLlmDraftApiKey] = useState("");
  const [multimodalConfigData, setMultimodalConfigData] = useState<ModelConfigResponse | null>(null);
  const [multimodalConfigLoading, setMultimodalConfigLoading] = useState(false);
  const [multimodalConfigError, setMultimodalConfigError] = useState<string | null>(null);
  const [multimodalDraft, setMultimodalDraft] = useState<Record<string, ModelConfigItem>>({});
  const [multimodalConfigSaving, setMultimodalConfigSaving] = useState(false);
  const [multimodalConfigSaveMessage, setMultimodalConfigSaveMessage] = useState<string | null>(null);
  const [systemRestarting, setSystemRestarting] = useState(false);
  const [systemRestartMessage, setSystemRestartMessage] = useState<string | null>(null);

  const [taskId, setTaskId] = useState("");
  const [taskLoading, setTaskLoading] = useState(false);
  const [taskResult, setTaskResult] = useState<TaskQueryResponse | null>(null);
  const [taskError, setTaskError] = useState<string | null>(null);
  const [trackingTaskId, setTrackingTaskId] = useState<string | null>(null);

  const [interactionKind, setInteractionKind] = useState<"ask" | "run_skill">("ask");
  const [interactionChannel, setInteractionChannel] = useState<ChannelName>("ui");
  const [interactionExternalUserId, setInteractionExternalUserId] = useState("");
  const [interactionExternalChatId, setInteractionExternalChatId] = useState("");
  const [interactionAdapter, setInteractionAdapter] = useState("");
  const [interactionUserId, setInteractionUserId] = useState<number | null>(null);
  const [interactionChatId, setInteractionChatId] = useState<number | null>(null);
  const [interactionRole, setInteractionRole] = useState<string>("-");
  const [localContextLoading, setLocalContextLoading] = useState(false);
  const [localContextError, setLocalContextError] = useState<string | null>(null);
  const [interactionAskText, setInteractionAskText] = useState("你好，请汇报当前系统状态");
  const [interactionAgentMode, setInteractionAgentMode] = useState(false);
  const [interactionSkillName, setInteractionSkillName] = useState("health_check");
  const [interactionSkillArgs, setInteractionSkillArgs] = useState("{\"target\":\"self\"}");
  const [interactionLoading, setInteractionLoading] = useState(false);
  const [interactionError, setInteractionError] = useState<string | null>(null);
  const [interactionSubmittedTaskId, setInteractionSubmittedTaskId] = useState<string | null>(null);
  const [chatMessages, setChatMessages] = useState<ChatMessage[]>([
    {
      id: "chat-system-welcome",
      role: "system",
      text: "会话窗口已连接 clawd。发送消息后会自动提交 ask 任务并轮询结果。",
      ts: Date.now(),
    },
  ]);
  const [chatInput, setChatInput] = useState("");
  const [chatImageAttachments, setChatImageAttachments] = useState<ChatImageAttachment[]>([]);
  const [chatAgentMode, setChatAgentMode] = useState(true);
  const [chatSending, setChatSending] = useState(false);
  const [chatError, setChatError] = useState<string | null>(null);
  const [serviceActionLoading, setServiceActionLoading] = useState<Record<string, boolean>>({});
  const [serviceActionMessage, setServiceActionMessage] = useState<string | null>(null);
  const [waLoginDialogOpen, setWaLoginDialogOpen] = useState(false);
  const [waLoginLoading, setWaLoginLoading] = useState(false);
  const [waLoginError, setWaLoginError] = useState<string | null>(null);
  const [waLoginStatus, setWaLoginStatus] = useState<WhatsappWebLoginStatus | null>(null);
  const [waWebBridgeReachable, setWaWebBridgeReachable] = useState(false);
  const [waLogoutLoading, setWaLogoutLoading] = useState(false);
  const [channelBindingChannel, setChannelBindingChannel] = useState<ChannelName>("telegram");
  const [channelBindingExternalUserId, setChannelBindingExternalUserId] = useState("");
  const [channelBindingExternalChatId, setChannelBindingExternalChatId] = useState("");
  const [channelResolveLoading, setChannelResolveLoading] = useState(false);
  const [channelResolveError, setChannelResolveError] = useState<string | null>(null);
  const [channelResolveResult, setChannelResolveResult] = useState<ResolveChannelBindingResponse | null>(null);
  const [channelBindLoading, setChannelBindLoading] = useState(false);
  const [channelBindError, setChannelBindError] = useState<string | null>(null);
  const [channelBindMessage, setChannelBindMessage] = useState<string | null>(null);
  const [authKeysList, setAuthKeysList] = useState<AuthKeyListItem[]>([]);
  const [authKeysLoading, setAuthKeysLoading] = useState(false);
  const [authKeysError, setAuthKeysError] = useState<string | null>(null);
  const [authKeyCreateLoading, setAuthKeyCreateLoading] = useState(false);
  const [authKeyCreateError, setAuthKeyCreateError] = useState<string | null>(null);
  const [authKeyActionLoading, setAuthKeyActionLoading] = useState<number | null>(null);
  const [authKeyActionError, setAuthKeyActionError] = useState<string | null>(null);
  const [newlyCreatedKey, setNewlyCreatedKey] = useState<string | null>(null);
  const [diagnosticsRefreshing, setDiagnosticsRefreshing] = useState(false);
  const [selectedLogFile, setSelectedLogFile] = useState("clawd.log");
  const [logTailLines, setLogTailLines] = useState(200);
  const [logLoading, setLogLoading] = useState(false);
  const [logError, setLogError] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  const [logLastUpdated, setLogLastUpdated] = useState<number | null>(null);
  const [logFollowTail, setLogFollowTail] = useState(true);
  const [currentPage, setCurrentPage] = useState<ConsolePage>(() => {
    const saved = window.localStorage.getItem(STORAGE_KEYS.currentPage);
    return saved && CONSOLE_PAGES.includes(saved as ConsolePage) ? (saved as ConsolePage) : "dashboard";
  });
  const [navDropdownOpen, setNavDropdownOpen] = useState(false);
  const logContainerRef = useRef<HTMLPreElement | null>(null);
  const chatImageInputRef = useRef<HTMLInputElement | null>(null);
  const navDropdownRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!navDropdownOpen) return;
    const onMouseDown = (e: MouseEvent) => {
      if (navDropdownRef.current?.contains(e.target as Node)) return;
      setNavDropdownOpen(false);
    };
    document.addEventListener("mousedown", onMouseDown);
    return () => document.removeEventListener("mousedown", onMouseDown);
  }, [navDropdownOpen]);

  const t = (zh: string, en: string) => (lang === "zh" ? zh : en);
  const isAdminIdentity = authIdentity?.role?.toLowerCase() === "admin";
  const tSlash = (mixed: string) => {
    const [zh, en] = mixed.split(" / ");
    return lang === "zh" ? zh : en ?? zh;
  };
  const channelLabel = (channel: ChannelName) => {
    const labels: Record<ChannelName, string> = {
      telegram: "Telegram",
      whatsapp: "WhatsApp",
      ui: "UI",
      feishu: "Feishu",
      lark: "Lark",
    };
    return labels[channel];
  };
  const formatDateTimeHuman = (raw: string | null | undefined) => {
    if (!raw) return "--";
    const date = new Date(raw);
    if (Number.isNaN(date.getTime())) return raw;
    return new Intl.DateTimeFormat(lang === "zh" ? "zh-CN" : "en-US", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    }).format(date);
  };
  const serviceDisplayName = (key: AdapterHealthRow["key"]) => {
    const labels: Record<AdapterHealthRow["key"], string> = {
      telegram_bot: t("Telegram 机器人", "Telegram Bot"),
      whatsapp_web: t("WhatsApp 网页版", "WhatsApp Web"),
      whatsapp_cloud: t("WhatsApp 云接口", "WhatsApp Cloud"),
      feishu_bot: t("飞书机器人", "Feishu Bot"),
      lark_bot: t("Lark 机器人", "Lark Bot"),
    };
    return labels[key];
  };
  const channelPresets = useMemo<Record<ChannelName, ChannelPreset>>(
    () => ({
      telegram: {
        summary: t("适合绑定 Telegram 私聊或群聊身份。", "Best for binding Telegram private chats or group identities."),
        userHint: t("通常填写 Telegram 用户 ID。", "Usually the Telegram user ID."),
        chatHint: t("群聊或频道场景建议补 chat_id。", "For groups or channels, provide chat_id as well."),
        exampleUser: "123456789",
        exampleChat: "-1001234567890",
        note: t("如果只是单聊排查，先填 external_user_id 往往就够。", "For direct chats, starting with external_user_id is usually enough."),
      },
      whatsapp: {
        summary: t("适合绑定 WhatsApp Cloud 或 Web 渠道身份。", "Best for binding WhatsApp Cloud or Web identities."),
        userHint: t("通常填写发送方/联系人标识。", "Usually the sender or contact identifier."),
        chatHint: t("群组或线程场景建议同时填写 external_chat_id。", "For groups or threaded chats, external_chat_id is recommended too."),
        exampleUser: "8613800138000",
        exampleChat: "1203630xxxxxxxxx@g.us",
        note: t("如果同一个号码在多个会话里复用，chat_id 能减少误绑。", "If one number appears across multiple threads, chat_id helps avoid mismatches."),
      },
      ui: {
        summary: t("用于本地 UI 会话身份排查。", "Useful for debugging the local UI identity."),
        userHint: t("通常不需要额外填写 external_user_id。", "external_user_id is usually unnecessary."),
        chatHint: t("一般也不需要 external_chat_id。", "external_chat_id is usually unnecessary too."),
        exampleUser: "",
        exampleChat: "",
        note: t("这个渠道更多是验证当前 key 与本地上下文是否一致。", "This channel is mainly for verifying the current key against local context."),
      },
      feishu: {
        summary: t("适合绑定飞书账号或会话。", "Best for binding Feishu identities or chats."),
        userHint: t("通常填写飞书用户标识，如 open_id / user_id。", "Usually a Feishu user identifier such as open_id / user_id."),
        chatHint: t("群聊或机器人会话建议同时填写 chat_id。", "For groups or bot threads, include chat_id as well."),
        exampleUser: "ou_xxxxxxxxxxxxx",
        exampleChat: "oc_xxxxxxxxxxxxx",
        note: t("如果你不确定字段来源，先从日志或 webhook 事件里复制原值。", "If unsure where the fields come from, copy the raw values from logs or webhook events."),
      },
      lark: {
        summary: t("适合绑定国际版 Lark 账号或会话。", "Best for binding international Lark identities or chats."),
        userHint: t("通常填写 Lark 用户标识。", "Usually a Lark user identifier."),
        chatHint: t("群聊场景建议补充 chat_id。", "For group chats, add chat_id as well."),
        exampleUser: "ou_xxxxxxxxxxxxx",
        exampleChat: "oc_xxxxxxxxxxxxx",
        note: t("字段形状通常和飞书接近，但建议以实际事件 payload 为准。", "The field shape is often similar to Feishu, but the real event payload should be your source of truth."),
      },
    }),
    [lang],
  );

  const isOnline = Boolean(health) && !error;
  const queuePressureHigh = (health?.queue_length ?? 0) >= queueWarn;
  const runningTooOld = (health?.running_oldest_age_seconds ?? 0) >= ageWarnSeconds;
  const authHeaders = uiKey ? { "X-RustClaw-Key": uiKey } : {};
  const normalizeFetchError = (err: unknown, targetUrl: string) => {
    const fallback = t("未知错误", "Unknown error");
    if (!(err instanceof Error)) return fallback;
    const raw = err.message || fallback;
    const lower = raw.toLowerCase();
    const looksLikeNetworkError =
      lower.includes("failed to fetch") || lower.includes("networkerror") || lower.includes("load failed");
    if (!looksLikeNetworkError) return raw;

    try {
      const pageProtocol = window.location.protocol;
      const apiProtocol = new URL(targetUrl, window.location.href).protocol;
      if (pageProtocol === "https:" && apiProtocol === "http:") {
        return t(
          `无法连接到服务：当前页面是 HTTPS，但服务地址是 HTTP（${targetUrl}）。请改成 HTTPS 服务地址，或改用 HTTP 打开前端。`,
          `Cannot reach backend: current page is HTTPS but API is HTTP (${targetUrl}). Use an HTTPS API URL or open the UI over HTTP.`,
        );
      }
    } catch {
      // ignore URL parse failures and use generic network guidance
    }

    return t(
      `无法连接到服务：${targetUrl}。请检查服务是否启动、Base URL 是否正确、以及浏览器是否拦截跨域请求。`,
      `Cannot reach backend: ${targetUrl}. Check backend is running, Base URL is correct, and whether browser/CORS policy blocks the request.`,
    );
  };

  const safeFetch = async (path: string, init?: RequestInit, withAuth = true) => {
    const targetUrl = `${apiBase.replace(/\/$/, "")}${path}`;
    try {
      return await fetch(targetUrl, {
        ...init,
        headers: {
          ...(init?.headers ?? {}),
          ...(withAuth ? authHeaders : {}),
        },
      });
    } catch (err) {
      throw new Error(normalizeFetchError(err, targetUrl));
    }
  };

  const apiFetch = (path: string, init?: RequestInit) => safeFetch(path, init, true);
  const publicApiFetch = (path: string, init?: RequestInit) => safeFetch(path, init, false);

  const applyIdentity = (identity: AuthIdentityResponse) => {
    setAuthIdentity(identity);
    setInteractionUserId(identity.user_id);
    setInteractionChatId(identity.chat_id);
    setInteractionRole(identity.role);
  };

  const fetchAuthMe = async (silent = false) => {
    if (!silent) {
      setAuthMeLoading(true);
      setAuthMeError(null);
    }
    try {
      const res = await apiFetch(`/v1/auth/me`);
      const body = (await res.json()) as ApiResponse<AuthIdentityResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `auth/me 请求失败 (${res.status})`);
      }
      applyIdentity(body.data);
      if (!silent) {
        setAuthMeError(null);
      }
      return body.data;
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      if (!silent) {
        setAuthMeError(message);
      }
      return null;
    } finally {
      if (!silent) {
        setAuthMeLoading(false);
      }
    }
  };

  const verifyUiKey = async (candidate: string, persist = true) => {
    const normalized = candidate.trim();
    if (!normalized) {
      setUiAuthReady(false);
      setUiAuthError(t("请输入 key", "Please enter a key"));
      return false;
    }
    setUiAuthLoading(true);
    setUiAuthError(null);
    try {
      const res = await publicApiFetch(`/v1/auth/ui-key/verify`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ user_key: normalized }),
      });
      const body = (await res.json()) as ApiResponse<AuthIdentityResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `key 校验失败 (${res.status})`);
      }
      setUiKey(normalized);
      setUiKeyDraft(normalized);
      setUiAuthReady(true);
      setAuthMeError(null);
      applyIdentity(body.data);
      if (persist) {
        window.localStorage.setItem(STORAGE_KEYS.userKey, normalized);
      }
      return true;
    } catch (err) {
      setUiAuthReady(false);
      setUiKey("");
      setAuthIdentity(null);
      setInteractionUserId(null);
      setInteractionChatId(null);
      setInteractionRole("-");
      const message = err instanceof Error ? err.message : "未知错误";
      setUiAuthError(message);
      window.localStorage.removeItem(STORAGE_KEYS.userKey);
      return false;
    } finally {
      setUiAuthLoading(false);
    }
  };

  const logout = () => {
    window.localStorage.removeItem(STORAGE_KEYS.userKey);
    setUiKey("");
    setUiKeyDraft("");
    setUiAuthReady(false);
    setUiAuthError(null);
    setAuthIdentity(null);
    setAuthMeError(null);
    setInteractionUserId(null);
    setInteractionChatId(null);
    setInteractionRole("-");
  };

  const fetchHealth = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await apiFetch(`/v1/health`);
      const body = (await res.json()) as ApiResponse<HealthResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `health 请求失败 (${res.status})`);
      }
      setHealth(body.data);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setError(message);
    } finally {
      setLoading(false);
    }
  };

  const controlService = async (
    serviceName: "telegramd" | "whatsappd" | "whatsapp_webd" | "feishud" | "larkd",
    action: "start" | "stop" | "restart",
  ) => {
    setServiceActionMessage(null);
    setServiceActionLoading((prev) => ({ ...prev, [serviceName]: true }));
    try {
      const res = await apiFetch(`/v1/services/${serviceName}/${action}`, {
        method: "POST",
      });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `${action} ${serviceName} failed (${res.status})`);
      }
      setServiceActionMessage(
        t(
          `服务操作成功：${serviceName} -> ${action}`,
          `Service action succeeded: ${serviceName} -> ${action}`,
        ),
      );
      await sleep(800);
      await fetchHealth();
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setServiceActionMessage(`${t("服务操作失败", "Service action failed")}: ${message}`);
    } finally {
      setServiceActionLoading((prev) => ({ ...prev, [serviceName]: false }));
    }
  };

  const fetchWhatsappWebLoginStatus = async (silent = false) => {
    if (!silent) {
      setWaLoginLoading(true);
      setWaLoginError(null);
    }
    try {
      const res = await apiFetch(`/v1/whatsapp-web/login-status`);
      const body = (await res.json()) as ApiResponse<WhatsappWebLoginStatus>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `获取 WhatsApp 登录状态失败 (${res.status})`);
      }
      setWaLoginStatus(body.data);
      setWaWebBridgeReachable(true);
      if (!silent) {
        setWaLoginError(null);
      }
    } catch (err) {
      setWaWebBridgeReachable(false);
      const message = err instanceof Error ? err.message : "未知错误";
      if (!silent) {
        setWaLoginError(message);
      }
    } finally {
      if (!silent) {
        setWaLoginLoading(false);
      }
    }
  };

  const logoutWhatsappWeb = async () => {
    setWaLogoutLoading(true);
    setWaLoginError(null);
    try {
      const res = await apiFetch(`/v1/whatsapp-web/logout`, {
        method: "POST",
      });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `退出登录失败 (${res.status})`);
      }
      await sleep(1200);
      await fetchWhatsappWebLoginStatus();
      setServiceActionMessage(t("已发起 WhatsApp Web 退出登录", "WhatsApp Web logout requested"));
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setWaLoginError(message);
    } finally {
      setWaLogoutLoading(false);
    }
  };

  const fetchLocalInteractionContext = async () => {
    setLocalContextLoading(true);
    setLocalContextError(null);
    try {
      const res = await apiFetch(`/v1/local/interaction-context`);
      const body = (await res.json()) as ApiResponse<LocalInteractionContextResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `本地上下文获取失败 (${res.status})`);
      }
      setInteractionUserId(body.data.user_id);
      setInteractionChatId(body.data.chat_id);
      setInteractionRole(body.data.role);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setLocalContextError(message);
    } finally {
      setLocalContextLoading(false);
    }
  };

  const resolveChannelBinding = async () => {
    setChannelResolveLoading(true);
    setChannelResolveError(null);
    setChannelBindMessage(null);
    try {
      const body: Record<string, unknown> = {
        channel: channelBindingChannel,
      };
      const externalUserId = channelBindingExternalUserId.trim();
      const externalChatId = channelBindingExternalChatId.trim();
      if (externalUserId) {
        body.external_user_id = externalUserId;
      }
      if (externalChatId) {
        body.external_chat_id = externalChatId;
      }
      const res = await apiFetch(`/v1/auth/channel/resolve`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      const resp = (await res.json()) as ApiResponse<ResolveChannelBindingResponse>;
      if (!res.ok || !resp.ok || !resp.data) {
        throw new Error(resp.error || `渠道绑定查询失败 (${res.status})`);
      }
      setChannelResolveResult(resp.data);
      return resp.data;
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setChannelResolveError(message);
      return null;
    } finally {
      setChannelResolveLoading(false);
    }
  };

  const bindChannelToCurrentKey = async () => {
    setChannelBindLoading(true);
    setChannelBindError(null);
    setChannelBindMessage(null);
    try {
      const body: Record<string, unknown> = {
        channel: channelBindingChannel,
        user_key: uiKey,
      };
      const externalUserId = channelBindingExternalUserId.trim();
      const externalChatId = channelBindingExternalChatId.trim();
      if (externalUserId) {
        body.external_user_id = externalUserId;
      }
      if (externalChatId) {
        body.external_chat_id = externalChatId;
      }
      const res = await apiFetch(`/v1/auth/channel/bind`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      const resp = (await res.json()) as ApiResponse<AuthIdentityResponse>;
      if (!res.ok || !resp.ok || !resp.data) {
        throw new Error(resp.error || `渠道绑定失败 (${res.status})`);
      }
      setChannelResolveResult({ bound: true, identity: resp.data });
      setChannelBindMessage(
        t(
          `绑定成功：${channelLabel(channelBindingChannel)} 已绑定到当前 key`,
          `${channelLabel(channelBindingChannel)} has been bound to the current key`,
        ),
      );
      applyIdentity(resp.data);
      await fetchHealth();
      return resp.data;
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setChannelBindError(message);
      return null;
    } finally {
      setChannelBindLoading(false);
    }
  };

  const fetchAuthKeys = async () => {
    setAuthKeysLoading(true);
    setAuthKeysError(null);
    setAuthKeyActionError(null);
    try {
      const res = await apiFetch("/v1/admin/auth-keys");
      const body = (await res.json()) as ApiResponse<{ keys: AuthKeyListItem[] }>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `Key 列表获取失败 (${res.status})`);
      }
      setAuthKeysList(body.data.keys);
    } catch (err) {
      setAuthKeysError(err instanceof Error ? err.message : "未知错误");
    } finally {
      setAuthKeysLoading(false);
    }
  };

  const createAuthKey = async (role: "user" | "admin" = "user") => {
    setAuthKeyCreateLoading(true);
    setAuthKeyCreateError(null);
    setNewlyCreatedKey(null);
    try {
      const res = await apiFetch("/v1/admin/auth-keys", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ role }),
      });
      const body = (await res.json()) as ApiResponse<{ user_key: string }>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `生成 Key 失败 (${res.status})`);
      }
      setNewlyCreatedKey(body.data.user_key);
      await fetchAuthKeys();
    } catch (err) {
      setAuthKeyCreateError(err instanceof Error ? err.message : "未知错误");
    } finally {
      setAuthKeyCreateLoading(false);
    }
  };

  const updateAuthKey = async (keyId: number, patch: { role?: "user" | "admin"; enabled?: boolean }) => {
    setAuthKeyActionLoading(keyId);
    setAuthKeyActionError(null);
    try {
      const res = await apiFetch(`/v1/admin/auth-keys/${keyId}`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(patch),
      });
      const body = (await res.json()) as ApiResponse<{ updated: boolean }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `更新 Key 失败 (${res.status})`);
      }
      await fetchAuthKeys();
    } catch (err) {
      setAuthKeyActionError(err instanceof Error ? err.message : "未知错误");
    } finally {
      setAuthKeyActionLoading(null);
    }
  };

  const deleteAuthKey = async (row: AuthKeyListItem) => {
    const ok = window.confirm(
      t(
        `确认删除 ${row.user_key_masked}？删除后将移除该 Key 及关联绑定。`,
        `Delete ${row.user_key_masked}? This will remove the key and related bindings.`,
      ),
    );
    if (!ok) return;
    setAuthKeyActionLoading(row.key_id);
    setAuthKeyActionError(null);
    try {
      const res = await apiFetch(`/v1/admin/auth-keys/${row.key_id}`, { method: "DELETE" });
      const body = (await res.json()) as ApiResponse<{ deleted: boolean }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `删除 Key 失败 (${res.status})`);
      }
      await fetchAuthKeys();
    } catch (err) {
      setAuthKeyActionError(err instanceof Error ? err.message : "未知错误");
    } finally {
      setAuthKeyActionLoading(null);
    }
  };

  const refreshDiagnostics = async () => {
    setDiagnosticsRefreshing(true);
    try {
      await Promise.all([
        fetchHealth(),
        fetchLocalInteractionContext(),
        fetchAuthMe(),
        fetchWhatsappWebLoginStatus(true),
      ]);
    } finally {
      setDiagnosticsRefreshing(false);
    }
  };

  const fetchSkills = async () => {
    setSkillsLoading(true);
    setSkillsError(null);
    try {
      const res = await apiFetch(`/v1/skills`);
      const body = (await res.json()) as ApiResponse<SkillsResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `技能列表获取失败 (${res.status})`);
      }
      setSkillsData(body.data);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setSkillsError(message);
    } finally {
      setSkillsLoading(false);
    }
  };

  const fetchSkillsConfig = async () => {
    setSkillsConfigLoading(true);
    setSkillsConfigError(null);
    try {
      const res = await apiFetch(`/v1/skills/config`);
      const body = (await res.json()) as ApiResponse<SkillsConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `技能配置获取失败 (${res.status})`);
      }
      setSkillsConfigData(body.data);
      setSkillSwitchDraft(body.data.skill_switches || {});
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setSkillsConfigError(message);
    } finally {
      setSkillsConfigLoading(false);
    }
  };

  const fetchLlmConfig = async () => {
    setLlmConfigLoading(true);
    setLlmConfigError(null);
    try {
      const res = await apiFetch(`/v1/llm/config`);
      const body = (await res.json()) as ApiResponse<LlmConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `模型配置获取失败 (${res.status})`);
      }
      setLlmConfigData(body.data);
      setLlmDraftVendor(body.data.selected_vendor || "");
      setLlmDraftModel(body.data.selected_model || "");
      const selectedVendor = body.data.vendors.find((vendor) => vendor.name === (body.data.selected_vendor || ""));
      setLlmDraftBaseUrl(selectedVendor?.base_url || "");
      setLlmDraftApiKey(selectedVendor?.api_key_masked || "");
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setLlmConfigError(message);
    } finally {
      setLlmConfigLoading(false);
    }
  };

  const scrollToSkillRow = (skillName: string) => {
    window.setTimeout(() => {
      const row = document.getElementById(`skill-row-${skillName}`);
      row?.scrollIntoView({ behavior: "smooth", block: "center" });
    }, 180);
  };

  const saveSkillSwitches = async () => {
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
        throw new Error(body.error || `技能配置保存失败 (${res.status})`);
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
        if (confirmed) {
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
      const message = err instanceof Error ? err.message : "未知错误";
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
        throw new Error(body.error || `技能导入失败 (${res.status})`);
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
      const message = err instanceof Error ? err.message : "未知错误";
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
        throw new Error(body.error || `本地导入失败 (${res.status})`);
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
      const message = err instanceof Error ? err.message : "未知错误";
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
        throw new Error(body.error || `技能卸载失败 (${res.status})`);
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
      const message = err instanceof Error ? err.message : "未知错误";
      setSkillsConfigError(message);
    } finally {
      setSkillUninstallingName(null);
    }
  };

  const saveLlmConfig = async () => {
    setLlmConfigSaving(true);
    setLlmConfigSaveMessage(null);
    setLlmConfigError(null);
    setSystemRestartMessage(null);
    try {
      const res = await apiFetch(`/v1/llm/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          selected_vendor: llmDraftVendor,
          selected_model: llmDraftModel,
          vendor_base_url: llmDraftBaseUrl,
          vendor_api_key: llmDraftApiKey.trim(),
        }),
      });
      const body = (await res.json()) as ApiResponse<{
        restart_required?: boolean;
      }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `模型配置保存失败 (${res.status})`);
      }
      setLlmConfigSaveMessage(
        t(
          "大模型设置已保存到 config.toml（需重启 clawd 生效）",
          "LLM settings saved to config.toml (restart clawd to apply)",
        ),
      );
      await fetchLlmConfig();
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setLlmConfigError(message);
    } finally {
      setLlmConfigSaving(false);
    }
  };

  const MULTIMODAL_KEYS = ["image_edit", "image_generation", "image_vision", "audio_synthesize", "audio_transcribe"] as const;

  const fetchMultimodalConfig = async () => {
    setMultimodalConfigLoading(true);
    setMultimodalConfigError(null);
    try {
      const res = await apiFetch("/v1/admin/model-config");
      const body = (await res.json()) as ApiResponse<ModelConfigResponse>;
      if (!res.ok || !body.ok || !body.data) throw new Error(body.error || "fetch failed");
      setMultimodalConfigData(body.data);
      const draft: Record<string, ModelConfigItem> = {};
      for (const k of MULTIMODAL_KEYS) {
        const item = body.data[k];
        draft[k] = {
          vendor: item?.vendor ?? "",
          model: item?.model ?? "",
          base_url: item?.base_url ?? "",
          api_key: item?.api_key ?? "",
        };
      }
      setMultimodalDraft(draft);
    } catch (err) {
      setMultimodalConfigError(err instanceof Error ? err.message : "Unknown");
    } finally {
      setMultimodalConfigLoading(false);
    }
  };

  const saveMultimodalConfig = async () => {
    setMultimodalConfigSaving(true);
    setMultimodalConfigSaveMessage(null);
    setMultimodalConfigError(null);
    try {
      const payload: Record<string, ModelConfigItem | undefined> = {};
      for (const k of MULTIMODAL_KEYS) {
        const d = multimodalDraft[k];
        if (d)
          payload[k] = {
            vendor: d.vendor.trim() || d.vendor,
            model: d.model.trim() || d.model,
            base_url: d.base_url?.trim() ?? "",
            api_key: d.api_key?.trim() ?? "",
          };
      }
      const res = await apiFetch("/v1/admin/model-config", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const body = (await res.json()) as ApiResponse<{ restart_required?: boolean }>;
      if (!res.ok || !body.ok) throw new Error(body.error || "save failed");
      setMultimodalConfigSaveMessage(t("图像与语音模块配置已保存，需重启 clawd 生效。", "Image/audio config saved. Restart clawd to apply."));
      await fetchMultimodalConfig();
    } catch (err) {
      setMultimodalConfigError(err instanceof Error ? err.message : "Unknown");
    } finally {
      setMultimodalConfigSaving(false);
    }
  };

  const setMultimodalDraftKey = (key: (typeof MULTIMODAL_KEYS)[number], field: keyof ModelConfigItem, value: string) => {
    setMultimodalDraft((prev) => ({ ...prev, [key]: { ...(prev[key] ?? { vendor: "", model: "", base_url: "", api_key: "" }), [field]: value } }));
  };

  const hasUnsavedMultimodalChanges = useMemo(() => {
    if (!multimodalConfigData) return false;
    for (const k of MULTIMODAL_KEYS) {
      const a = multimodalConfigData[k];
      const b = multimodalDraft[k];
      if (!b) continue;
      if ((a?.vendor ?? "") !== (b.vendor ?? "") || (a?.model ?? "") !== (b.model ?? "")) return true;
      if ((a?.base_url ?? "") !== (b.base_url ?? "") || (a?.api_key ?? "") !== (b.api_key ?? "")) return true;
    }
    return false;
  }, [multimodalConfigData, multimodalDraft]);

  const restartSystem = async () => {
    setSystemRestarting(true);
    setSystemRestartMessage(null);
    setLlmConfigError(null);
    setSkillsConfigError(null);
    let restartAccepted = false;
    try {
      const res = await apiFetch(`/v1/system/restart`, {
        method: "POST",
      });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `重启失败 (${res.status})`);
      }
      restartAccepted = true;
      setSystemRestartMessage(
        t(
          "已发起重启，页面会短暂断开，稍后会自动恢复。",
          "Restart requested. The page may disconnect briefly and then recover.",
        ),
      );
      await sleep(1800);
      let recovered = false;
      for (let attempt = 0; attempt < 12; attempt += 1) {
        try {
          const probe = await apiFetch(`/v1/health`);
          const body = (await probe.json()) as ApiResponse<HealthResponse>;
          if (probe.ok && body.ok && body.data) {
            recovered = true;
            setHealth(body.data);
            setError(null);
            break;
          }
        } catch {
          // The restart window is expected to cause transient failures while clawd comes back up.
        }
        await sleep(1500);
      }

      if (recovered) {
        await Promise.allSettled([fetchLlmConfig(), fetchMultimodalConfig(), fetchSkillsConfig(), fetchSkills()]);
        setSystemRestartMessage(
          t(
            "RustClaw 已重启完成，当前页面已经恢复。",
            "RustClaw restarted successfully and the page is back online.",
          ),
        );
      } else {
        setSystemRestartMessage(
          t(
            "重启请求已经发出，但暂时还没等到服务恢复。请稍后手动刷新。",
            "Restart was requested, but the service has not recovered yet. Please refresh shortly.",
          ),
        );
      }
      setSystemRestarting(false);
      return recovered;
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setSystemRestartMessage(`${t("重启失败", "Restart failed")}: ${message}`);
      return false;
    } finally {
      if (!restartAccepted) {
        setSystemRestarting(false);
      }
    }
  };

  const fetchLatestLog = async () => {
    setLogLoading(true);
    setLogError(null);
    try {
      const params = new URLSearchParams({
        file: selectedLogFile,
        lines: String(logTailLines),
      });
      const res = await apiFetch(`/v1/logs/latest?${params.toString()}`);
      const body = (await res.json()) as ApiResponse<LogLatestResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `日志读取失败 (${res.status})`);
      }
      setLogText(body.data.text || "");
      setLogLastUpdated(Date.now());
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setLogError(message);
    } finally {
      setLogLoading(false);
    }
  };

  const fetchTaskById = async (id: string): Promise<TaskQueryResponse> => {
    const res = await apiFetch(`/v1/tasks/${id.trim()}`);
    const body = (await res.json()) as ApiResponse<TaskQueryResponse>;
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(body.error || `任务查询失败 (${res.status})`);
    }
    return body.data;
  };

  const queryTaskById = async (id: string, resetBeforeLoad = true): Promise<TaskQueryResponse | null> => {
    if (!id.trim()) return null;
    if (resetBeforeLoad) {
      setTaskLoading(true);
      setTaskError(null);
      setTaskResult(null);
    }
    try {
      const result = await fetchTaskById(id);
      setTaskResult(result);
      setTaskError(null);
      return result;
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setTaskError(message);
      return null;
    } finally {
      if (resetBeforeLoad) {
        setTaskLoading(false);
      }
    }
  };

  const queryTask = async () => {
    if (!taskId.trim()) return;
    setTaskLoading(true);
    await queryTaskById(taskId, false);
    setTaskLoading(false);
  };

  const submitInteractionTask = async () => {
    setInteractionLoading(true);
    setInteractionError(null);
    setInteractionSubmittedTaskId(null);
    try {
      let payload: Record<string, unknown>;
      if (interactionKind === "ask") {
        payload = {
          text: interactionAskText.trim(),
          agent_mode: interactionAgentMode,
        };
      } else {
        let parsedArgs: unknown = interactionSkillArgs;
        try {
          parsedArgs = JSON.parse(interactionSkillArgs);
        } catch {
          // keep raw string as args when not valid JSON
        }
        payload = {
          skill_name: interactionSkillName.trim(),
          args: parsedArgs,
        };
      }
      const adapterName = interactionAdapter.trim();
      if (adapterName) {
        payload.adapter = adapterName;
      }

      const body: Record<string, unknown> = {
        user_key: uiKey,
        channel: interactionChannel,
        kind: interactionKind,
        payload,
      };
      const externalUserId = interactionExternalUserId.trim();
      if (externalUserId) {
        body.external_user_id = externalUserId;
      }
      const externalChatId = interactionExternalChatId.trim();
      if (externalChatId) {
        body.external_chat_id = externalChatId;
      }

      const res = await apiFetch(`/v1/tasks`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      const resp = (await res.json()) as ApiResponse<SubmitTaskResponse>;
      if (!res.ok || !resp.ok || !resp.data?.task_id) {
        throw new Error(resp.error || `提交任务失败 (${res.status})`);
      }

      setInteractionSubmittedTaskId(resp.data.task_id);
      setTaskId(resp.data.task_id);
      setTrackingTaskId(resp.data.task_id);
      setTaskResult(null);
      setTaskError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setInteractionError(message);
    } finally {
      setInteractionLoading(false);
    }
  };

  const handleChatImageSelection = async (fileList: FileList | null) => {
    if (!fileList || fileList.length === 0) return;
    try {
      const selected = Array.from(fileList).filter((file) => file.type.startsWith("image/"));
      if (selected.length === 0) {
        setChatError(t("请选择图片文件。", "Please choose image files."));
        return;
      }
      const nextImages = await Promise.all(
        selected.map(async (file) => ({
          name: file.name,
          dataUrl: await fileToDataUrl(file),
        })),
      );
      setChatImageAttachments((prev) => [...prev, ...nextImages].slice(0, 6));
      setChatError(null);
    } catch (err) {
      setChatError(err instanceof Error ? err.message : t("读取图片失败。", "Failed to read images."));
    }
  };

  const removeChatImageAttachment = (index: number) => {
    setChatImageAttachments((prev) => prev.filter((_, i) => i !== index));
  };

  const sendChatMessage = async () => {
    const text = chatInput.trim();
    const attachedImages = chatImageAttachments;
    if ((!text && attachedImages.length === 0) || chatSending) return;
    const requestText = text || t("请描述这张图片", "Please describe this image");
    setChatSending(true);
    setChatError(null);
    const userMsg: ChatMessage = {
      id: `u-${Date.now()}`,
      role: "user",
      text: text || t("发送了一张图片", "Sent an image"),
      ts: Date.now(),
      images: attachedImages,
    };
    setChatMessages((prev) => [...prev, userMsg]);
    setChatInput("");
    setChatImageAttachments([]);
    if (chatImageInputRef.current) {
      chatImageInputRef.current.value = "";
    }

    try {
      const adapterName = interactionAdapter.trim();
      const submitBody: Record<string, unknown> = {
        user_key: uiKey,
        channel: interactionChannel,
        kind: "ask",
        ...(interactionExternalUserId.trim() ? { external_user_id: interactionExternalUserId.trim() } : {}),
        ...(interactionExternalChatId.trim() ? { external_chat_id: interactionExternalChatId.trim() } : {}),
        payload: {
          text: requestText,
          agent_mode: chatAgentMode,
          ...(adapterName ? { adapter: adapterName } : {}),
          ...(attachedImages.length > 0
            ? {
                images: attachedImages.map((image) => ({ base64: image.dataUrl })),
                response_language: lang === "zh" ? "zh-CN" : "en",
              }
            : {}),
        },
      };
      const submitRes = await apiFetch(`/v1/tasks`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(submitBody),
      });
      const submitData = (await submitRes.json()) as ApiResponse<SubmitTaskResponse>;
      if (!submitRes.ok || !submitData.ok || !submitData.data?.task_id) {
        throw new Error(submitData.error || `提交任务失败 (${submitRes.status})`);
      }

      const submittedTaskId = submitData.data.task_id;
      setTaskId(submittedTaskId);
      setTrackingTaskId(submittedTaskId);

      let finalResult: TaskQueryResponse | null = null;
      for (let i = 0; i < 90; i += 1) {
        const current = await fetchTaskById(submittedTaskId);
        if (["succeeded", "failed", "canceled", "timeout"].includes(current.status)) {
          finalResult = current;
          break;
        }
        await sleep(1200);
      }
      if (!finalResult) {
        throw new Error("轮询超时：任务仍在运行，请稍后在任务查询区查看。");
      }
      setTaskResult(finalResult);
      setTrackingTaskId(
        ["succeeded", "failed", "canceled", "timeout"].includes(finalResult.status) ? null : submittedTaskId,
      );

      const assistantMsg: ChatMessage = {
        id: `a-${Date.now()}`,
        role: "assistant",
        text: attachedImages.length > 0 ? formatVisionResultText(extractTaskText(finalResult)) : extractTaskText(finalResult),
        ts: Date.now(),
      };
      setChatMessages((prev) => [...prev, assistantMsg]);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setChatError(message);
      const systemErrMsg: ChatMessage = {
        id: `e-${Date.now()}`,
        role: "system",
        text: `发送失败：${message}`,
        ts: Date.now(),
      };
      setChatMessages((prev) => [...prev, systemErrMsg]);
    } finally {
      setChatSending(false);
    }
  };

  const handleChatInputKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void sendChatMessage();
    }
  };

  useEffect(() => {
    if (!uiKey) {
      setUiAuthReady(false);
      setAuthIdentity(null);
      setInteractionUserId(null);
      setInteractionChatId(null);
      setInteractionRole("-");
      return;
    }
    void verifyUiKey(uiKey, false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase]);

  useEffect(() => {
    if (!uiAuthReady || pollingSeconds <= 0) return;
    void fetchHealth();
    void fetchAuthMe();
    void fetchSkills();
    void fetchSkillsConfig();
    void fetchLlmConfig();
    void fetchMultimodalConfig();
    void fetchLocalInteractionContext();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady || pollingSeconds <= 0) return;
    if (pollingSeconds <= 0) return;
    const timer = window.setInterval(() => {
      void fetchHealth();
    }, pollingSeconds * 1000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, pollingSeconds, uiAuthReady]);

  useEffect(() => {
    window.localStorage.setItem(STORAGE_KEYS.baseUrl, baseUrl);
  }, [baseUrl]);

  useEffect(() => {
    if (uiKey) {
      window.localStorage.setItem(STORAGE_KEYS.userKey, uiKey);
    } else {
      window.localStorage.removeItem(STORAGE_KEYS.userKey);
    }
  }, [uiKey]);

  useEffect(() => {
    window.localStorage.setItem(STORAGE_KEYS.polling, String(pollingSeconds));
  }, [pollingSeconds]);

  useEffect(() => {
    window.localStorage.setItem(STORAGE_KEYS.queueWarn, String(queueWarn));
  }, [queueWarn]);

  useEffect(() => {
    window.localStorage.setItem(STORAGE_KEYS.ageWarn, String(ageWarnSeconds));
  }, [ageWarnSeconds]);

  useEffect(() => {
    window.localStorage.setItem(STORAGE_KEYS.lang, lang);
  }, [lang]);

  useEffect(() => {
    window.localStorage.setItem(STORAGE_KEYS.themeMode, themeMode);
    document.documentElement.dataset.theme = themeMode;
  }, [themeMode]);

  useEffect(() => {
    window.localStorage.setItem(STORAGE_KEYS.currentPage, currentPage);
  }, [currentPage]);

  // 切换导航页时仅将主内容区滚动到顶部，不移动导航栏（不调用 scrollIntoView，避免小屏横向导航条滚动或整页抖动）
  useEffect(() => {
    window.scrollTo({ top: 0, left: 0, behavior: "instant" });
  }, [currentPage]);

  useEffect(() => {
    if (!uiAuthReady) return;
    void fetchAuthMe(true);
    void fetchSkills();
    void fetchSkillsConfig();
    void fetchLlmConfig();
    void fetchLocalInteractionContext();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!trackingTaskId) return;
    const interval = window.setInterval(async () => {
      const result = await queryTaskById(trackingTaskId, false);
      if (!result) return;
      if (["succeeded", "failed", "canceled", "timeout"].includes(result.status)) {
        setTrackingTaskId(null);
      }
    }, 2000);
    return () => window.clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [trackingTaskId, apiBase, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (currentPage === "channels") {
      void fetchAuthKeys();
    }
  }, [currentPage, uiAuthReady, isAdminIdentity]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (currentPage !== "logs") return;
    void fetchLatestLog();
    const timer = window.setInterval(() => {
      void fetchLatestLog();
    }, Math.max(2, pollingSeconds) * 1000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPage, apiBase, selectedLogFile, logTailLines, pollingSeconds, uiAuthReady]);

  useEffect(() => {
    if (!logFollowTail) return;
    const el = logContainerRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [logText, logFollowTail]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!waLoginDialogOpen) return;
    void fetchWhatsappWebLoginStatus();
    const timer = window.setInterval(() => {
      void fetchWhatsappWebLoginStatus(true);
    }, 2000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [waLoginDialogOpen, apiBase, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady) return;
    // Keep whatsapp web login status fresh for row actions.
    void fetchWhatsappWebLoginStatus(true);
    const timer = window.setInterval(() => {
      void fetchWhatsappWebLoginStatus(true);
    }, 5000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady]);

  const maskedSavedUiKey = useMemo(() => maskStoredKey(uiKey), [uiKey]);
  const adapterHealthRows = useMemo<AdapterHealthRow[]>(() => {
    const rows: AdapterHealthRow[] = [
      {
        key: "telegram_bot",
        label: serviceDisplayName("telegram_bot"),
        serviceName: "telegramd",
        healthy: health?.telegram_bot_healthy ?? health?.telegramd_healthy,
        processCount: health?.telegram_bot_process_count ?? health?.telegramd_process_count,
        memoryRssBytes: health?.telegram_bot_memory_rss_bytes ?? health?.telegramd_memory_rss_bytes,
      },
      {
        key: "whatsapp_web",
        label: serviceDisplayName("whatsapp_web"),
        serviceName: "whatsapp_webd",
        healthy: health?.whatsapp_web_healthy,
        processCount: health?.whatsapp_web_process_count,
        memoryRssBytes: health?.whatsapp_web_memory_rss_bytes,
      },
      {
        key: "whatsapp_cloud",
        label: serviceDisplayName("whatsapp_cloud"),
        serviceName: "whatsappd",
        healthy: health?.whatsapp_cloud_healthy ?? health?.whatsappd_healthy,
        processCount: health?.whatsapp_cloud_process_count ?? health?.whatsappd_process_count,
        memoryRssBytes: health?.whatsapp_cloud_memory_rss_bytes ?? health?.whatsappd_memory_rss_bytes,
      },
      {
        key: "feishu_bot",
        label: serviceDisplayName("feishu_bot"),
        serviceName: "feishud",
        healthy: health?.feishud_healthy,
        processCount: health?.feishud_process_count,
        memoryRssBytes: health?.feishud_memory_rss_bytes,
      },
      {
        key: "lark_bot",
        label: serviceDisplayName("lark_bot"),
        serviceName: "larkd",
        healthy: health?.larkd_healthy,
        processCount: health?.larkd_process_count,
        memoryRssBytes: health?.larkd_memory_rss_bytes,
      },
    ];
    // 运行的（healthy）排上面，未运行的排下面；同组内按 key 稳定排序
    return [...rows].sort((a, b) => {
      const aUp = a.healthy === true ? 1 : 0;
      const bUp = b.healthy === true ? 1 : 0;
      if (bUp !== aUp) return bUp - aUp;
      return (a.key || "").localeCompare(b.key || "");
    });
  }, [health, lang]);
  const serviceStatusRows = useMemo<ServiceStatusRow[]>(() => {
    return adapterHealthRows.map((row) => {
      if (row.key === "whatsapp_web") {
        if (row.healthy === true && waLoginStatus?.connected === true) {
          return {
            ...row,
            category: "ready",
            statusLabel: t("已登录可用", "Connected and ready"),
            detail: t("进程正常，WhatsApp Web 已完成登录。", "Daemon is healthy and WhatsApp Web is connected."),
          };
        }
        if (row.healthy === true) {
          return {
            ...row,
            category: "attention",
            statusLabel: t("进程已起，待登录", "Running, login required"),
            detail:
              waLoginStatus?.qr_ready
                ? t("二维码已就绪，可以直接扫码。", "QR is ready and can be scanned now.")
                : t("进程已启动，但还没有可用登录态。", "Daemon is up, but no active login session is available yet."),
          };
        }
        if (row.healthy === false) {
          return {
            ...row,
            category: "stopped",
            statusLabel: t("进程未运行", "Daemon stopped"),
            detail: t("先启动 whatsapp_webd，再检查二维码或登录态。", "Start whatsapp_webd first, then check QR or login state."),
          };
        }
        return {
          ...row,
          category: "unknown",
          statusLabel: t("状态未知", "Unknown"),
          detail: t("暂时无法判断 whatsapp_webd 当前状态。", "Unable to determine the current whatsapp_webd state."),
        };
      }

      if (row.healthy === true) {
        return {
          ...row,
          category: "ready",
          statusLabel: t("进程已起", "Daemon running"),
          detail: t("至少从健康探针看，进程已经起来了。", "The health probe indicates the daemon process is up."),
        };
      }
      if (row.healthy === false) {
        return {
          ...row,
          category: "stopped",
          statusLabel: t("进程未运行", "Daemon stopped"),
          detail: t("当前没有检测到对应进程。", "The corresponding daemon process was not detected."),
        };
      }
      return {
        ...row,
        category: "unknown",
        statusLabel: t("状态未知", "Unknown"),
        detail: t("当前还拿不到足够的进程状态。", "There is not enough process state information yet."),
      };
    });
  }, [adapterHealthRows, lang, waLoginStatus]);
  const healthyServiceCount = useMemo(
    () => adapterHealthRows.filter((row) => row.healthy === true).length,
    [adapterHealthRows],
  );
  const unavailableServiceCount = useMemo(
    () => adapterHealthRows.filter((row) => row.healthy === false).length,
    [adapterHealthRows],
  );
  const serviceGroupCounts = useMemo(() => {
    return serviceStatusRows.reduce(
      (acc, row) => {
        acc[row.category] += 1;
        return acc;
      },
      { ready: 0, attention: 0, stopped: 0, unknown: 0 },
    );
  }, [serviceStatusRows]);
  const selectedChannelPreset = useMemo(() => channelPresets[channelBindingChannel], [channelBindingChannel, channelPresets]);
  const managedSkills = useMemo(() => {
    const set = new Set<string>(skillsConfigData?.managed_skills ?? []);
    Object.keys(skillSwitchDraft).forEach((k) => set.add(k));
    return Array.from(set)
      .filter((name) => !UI_HIDDEN_SKILLS.has(name))
      .sort((a, b) => a.localeCompare(b));
  }, [skillsConfigData, skillSwitchDraft]);
  const baseSkillNamesSet = useMemo(() => {
    const list = skillsConfigData?.base_skill_names;
    const useList = list && list.length > 0 ? list : FALLBACK_BASE_SKILL_NAMES;
    return new Set<string>(useList);
  }, [skillsConfigData?.base_skill_names]);
  const externalSkillNamesSet = useMemo(() => {
    return new Set<string>((skillsConfigData?.external_skill_names ?? []).filter((name) => !UI_HIDDEN_SKILLS.has(name)));
  }, [skillsConfigData?.external_skill_names]);
  const baseEnabledSkills = useMemo(() => {
    return new Set<string>((skillsConfigData?.skills_list ?? []).filter((name) => !UI_HIDDEN_SKILLS.has(name)));
  }, [skillsConfigData]);
  const configuredEnabledSkills = useMemo(() => {
    const set = new Set<string>((skillsConfigData?.skills_list ?? []).filter((name) => !UI_HIDDEN_SKILLS.has(name)));
    Object.entries(skillSwitchDraft).forEach(([name, value]) => {
      if (UI_HIDDEN_SKILLS.has(name)) return;
      if (value) set.add(name);
      else set.delete(name);
    });
    return set;
  }, [skillsConfigData, skillSwitchDraft]);
  const hasUnsavedSkillSwitchChanges = useMemo(() => {
    const persisted = skillsConfigData?.skill_switches ?? {};
    const keys = new Set<string>([
      ...Object.keys(persisted).filter((name) => !UI_HIDDEN_SKILLS.has(name)),
      ...Object.keys(skillSwitchDraft).filter((name) => !UI_HIDDEN_SKILLS.has(name)),
    ]);
    for (const key of keys) {
      if (persisted[key] !== skillSwitchDraft[key]) {
        return true;
      }
    }
    return false;
  }, [skillsConfigData, skillSwitchDraft]);
  const selectedLlmVendorInfo = useMemo(
    () => llmConfigData?.vendors.find((vendor) => vendor.name === llmDraftVendor) ?? null,
    [llmConfigData, llmDraftVendor],
  );
  const hasCustomLlmVendor = useMemo(
    () => (llmConfigData?.vendors ?? []).some((vendor) => vendor.name === "custom"),
    [llmConfigData],
  );
  const hasUnsavedLlmChanges = useMemo(() => {
    if (!llmConfigData) return false;
    const selectedVendorConfig = llmConfigData.vendors.find((vendor) => vendor.name === llmConfigData.selected_vendor);
    return (
      llmDraftVendor.trim() !== (llmConfigData.selected_vendor || "").trim() ||
      llmDraftModel.trim() !== (llmConfigData.selected_model || "").trim() ||
      llmDraftBaseUrl.trim() !== (selectedVendorConfig?.base_url || "").trim() ||
      llmDraftApiKey !== (selectedVendorConfig?.api_key_masked || "")
    );
  }, [llmConfigData, llmDraftApiKey, llmDraftBaseUrl, llmDraftModel, llmDraftVendor]);
  const llmRuntimeLabel = useMemo(() => {
    if (!llmConfigData?.runtime?.vendor || !llmConfigData.runtime.model) {
      return t("当前还没有读到运行中的模型信息", "Runtime model info is not available yet");
    }
    return `${llmConfigData.runtime.vendor} / ${llmConfigData.runtime.model}`;
  }, [llmConfigData, lang]);
  const llmSavedLabel = useMemo(() => {
    if (!llmConfigData?.selected_vendor || !llmConfigData.selected_model) {
      return t("当前还没有保存好的模型配置", "No saved model config yet");
    }
    return `${llmConfigData.selected_vendor} / ${llmConfigData.selected_model}`;
  }, [llmConfigData, lang]);
  const llmRestartPending = useMemo(() => {
    if (!llmConfigData) return false;
    const runtimeVendor = llmConfigData.runtime?.vendor?.trim() || "";
    const runtimeModel = llmConfigData.runtime?.model?.trim() || "";
    const savedVendor = llmConfigData.selected_vendor?.trim() || "";
    const savedModel = llmConfigData.selected_model?.trim() || "";
    return llmConfigData.restart_required || runtimeVendor !== savedVendor || runtimeModel !== savedModel;
  }, [llmConfigData]);
  const normalizedSkillsSearchQuery = useMemo(() => skillsSearchQuery.trim().toLowerCase(), [skillsSearchQuery]);
  const filteredManagedSkills = useMemo(
    () => managedSkills.filter((name) => !normalizedSkillsSearchQuery || name.toLowerCase().includes(normalizedSkillsSearchQuery)),
    [managedSkills, normalizedSkillsSearchQuery],
  );

  /** 技能分组：图片 / 语音 / 基础（不建议关闭）/ 其他 */
  const skillsImage = useMemo(
    () => managedSkills.filter((n) => n.startsWith("image_")).sort((a, b) => a.localeCompare(b)),
    [managedSkills],
  );
  const skillsAudio = useMemo(
    () => managedSkills.filter((n) => n.startsWith("audio_")).sort((a, b) => a.localeCompare(b)),
    [managedSkills],
  );
  const skillsBase = useMemo(
    () => managedSkills.filter((n) => baseSkillNamesSet.has(n)).sort((a, b) => a.localeCompare(b)),
    [managedSkills, baseSkillNamesSet],
  );
  const skillsOther = useMemo(
    () =>
      managedSkills
        .filter((n) => !n.startsWith("image_") && !n.startsWith("audio_") && !baseSkillNamesSet.has(n))
        .sort((a, b) => a.localeCompare(b)),
    [managedSkills, baseSkillNamesSet],
  );
  const filterBySearch = (list: string[]) =>
    list.filter((name) => !normalizedSkillsSearchQuery || name.toLowerCase().includes(normalizedSkillsSearchQuery));
  const filteredSkillsImage = useMemo(() => filterBySearch(skillsImage), [skillsImage, normalizedSkillsSearchQuery]);
  const filteredSkillsAudio = useMemo(() => filterBySearch(skillsAudio), [skillsAudio, normalizedSkillsSearchQuery]);
  const filteredSkillsBase = useMemo(() => filterBySearch(skillsBase), [skillsBase, normalizedSkillsSearchQuery]);
  const filteredSkillsOther = useMemo(() => filterBySearch(skillsOther), [skillsOther, normalizedSkillsSearchQuery]);
  useEffect(() => {
    if (!skillImportPreview) return;
    if (managedSkills.includes(skillImportPreview.skill_name)) return;
    setSkillImportPreview(null);
    if (recentImportedSkillName === skillImportPreview.skill_name) {
      setRecentImportedSkillName(null);
    }
  }, [managedSkills, recentImportedSkillName, skillImportPreview]);
  const visibleRuntimeSkills = useMemo(
    () => (skillsData?.skills ?? []).filter((name) => !UI_HIDDEN_SKILLS.has(name)),
    [skillsData],
  );
  const describeSkill = (name: string) =>
    SKILL_SUMMARY[name]
      ? t(SKILL_SUMMARY[name].zh, SKILL_SUMMARY[name].en)
      : t("该技能无简短说明。", "No short description for this skill.");
  const applyLlmVendorDraft = (nextVendor: string) => {
    const vendorInfo = llmConfigData?.vendors.find((vendor) => vendor.name === nextVendor);
    setLlmDraftVendor(nextVendor);
    if (!vendorInfo) {
      setLlmDraftModel("");
      setLlmDraftBaseUrl("");
      setLlmDraftApiKey("");
      return;
    }
    const nextModel = vendorInfo.default_model || vendorInfo.models[0] || "";
    setLlmDraftModel(nextModel);
    setLlmDraftBaseUrl(vendorInfo.base_url || "");
    setLlmDraftApiKey(vendorInfo.api_key_masked || "");
  };

  const toggleSkillEnabled = (name: string, nextEnabled: boolean) => {
    if (UI_HIDDEN_SKILLS.has(name)) return;
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
  const pageMeta = useMemo(
    () => ({
      dashboard: {
        title: t("首页", "Home"),
        desc: t("先看现在能不能用、下一步该点哪里，再决定要不要进更高级的页面。", "See whether things are working, what to do next, and only then move into advanced pages."),
      },
      chat: {
        title: t("对话交互", "Chat Interaction"),
        desc: t("和 RustClaw 直接对话，快速确认回复是否正常。", "Chat with RustClaw directly to verify responses."),
      },
      services: {
        title: t("连接状态", "Connections"),
        desc: t("这里看 Telegram、WhatsApp、飞书这些连接服务有没有正常工作。", "Check whether Telegram, WhatsApp, Feishu, and similar connection services are working properly."),
      },
      channels: {
        title: t("绑定账号", "Bind Accounts"),
        desc: t("列出后端所有 Key，生成新 Key。仅 admin 可操作。使用 Key 绑定通信端。", "List all keys and generate new keys. Admin only. Use a key to bind channel accounts."),
      },
      models: {
        title: t("模型设置", "Model Settings"),
        desc: t("这里设置 RustClaw 主要用哪家模型、模型名，以及该厂商的接口地址和密钥。", "Choose RustClaw's main LLM vendor, model, endpoint, and API key here."),
      },
      skills: {
        title: t("技能设置", "Skill Settings"),
        desc: t("这里单独管理技能开关和运行中的技能列表。", "Manage skill switches and the current runtime skill list here."),
      },
      logs: {
        title: t("查看日志", "Logs"),
        desc: t("当服务异常时再来看这里。正常使用时可以先不用碰。", "Come here when something breaks. In normal use, you usually do not need this page first."),
      },
      tasks: {
        title: t("手动任务", "Manual Tasks"),
        desc: t("这是手动测试和高级排查页，适合需要精确控制任务参数的时候。", "This is the manual testing and advanced troubleshooting page for when you need precise control over task parameters."),
      },
    }),
    [lang],
  );
  const navItems = useMemo(
    () => [
      {
        id: "dashboard" as const,
        label: t("首页", "Home"),
        hint: t("先看", "start here"),
        icon: <LayoutDashboard className="h-4 w-4" />,
      },
      {
        id: "chat" as const,
        label: t("对话交互", "Chat Interaction"),
        hint: t("发消息", "send message"),
        icon: <MessageCircle className="h-4 w-4" />,
      },
      {
        id: "services" as const,
        label: t("连接状态", "Connections"),
        hint: t("查连接", "service health"),
        icon: <Server className="h-4 w-4" />,
      },
      {
        id: "channels" as const,
        label: t("绑定账号", "Bind Accounts"),
        hint: t("连账号", "connect accounts"),
        icon: <Database className="h-4 w-4" />,
      },
      {
        id: "models" as const,
        label: t("模型设置", "Models"),
        hint: t("选模型", "llm config"),
        icon: <Sparkles className="h-4 w-4" />,
      },
      {
        id: "skills" as const,
        label: t("技能设置", "Skills"),
        hint: t("开关技能", "skill toggles"),
        icon: <Wrench className="h-4 w-4" />,
      },
      {
        id: "logs" as const,
        label: t("查看日志", "Logs"),
        hint: t("出问题再看", "when broken"),
        icon: <FileText className="h-4 w-4" />,
      },
      {
        id: "tasks" as const,
        label: t("手动任务", "Manual Tasks"),
        hint: t("高级测试", "advanced"),
        icon: <SquareTerminal className="h-4 w-4" />,
      },
    ],
    [lang],
  );
  const currentPageMeta = pageMeta[currentPage];
  const suggestedNextStep = useMemo(() => {
    if (!isOnline) {
      return {
        title: t("先检查服务是否启动", "Check whether the service is running"),
        desc: t("如果页面显示离线，先确认 clawd 地址是否正确，或者服务是否已经启动。", "If the console looks offline, first confirm the clawd address and whether the service is running."),
        page: "dashboard" as const,
        cta: t("查看首页提示", "Open Home"),
      };
    }
    if ((health?.bound_channel_count ?? 0) === 0) {
      return {
        title: t("绑定你的账号", "Bind your account"),
        desc: t("第一次使用时，先把 Telegram / WhatsApp / 飞书 这些外部账号绑定到当前登录 key。", "For first-time setup, bind Telegram / WhatsApp / Feishu identities to the current login key."),
        page: "channels" as const,
        cta: t("去绑定账号", "Bind account"),
      };
    }
    if (healthyServiceCount === 0) {
      return {
        title: t("启动连接服务", "Start connection services"),
        desc: t("如果一个服务都没运行，就先启动你需要用到的那几个渠道。", "If no connection service is running yet, start the channels you plan to use."),
        page: "services" as const,
        cta: t("去看连接状态", "Check services"),
      };
    }
    return {
      title: t("可以开始试一条消息了", "You can try a message now"),
      desc: t("基础状态已经差不多就绪，可以去对话交互页给 RustClaw 发一条简单消息。", "The basics look ready, so you can send a simple message on the Chat Interaction page."),
      page: "chat" as const,
      cta: t("去对话交互", "Open Chat"),
    };
  }, [healthyServiceCount, health?.bound_channel_count, isOnline, lang]);
  const toggleThemeMode = () => {
    setThemeMode((current) => (current === "dark" ? "light" : "dark"));
  };

  const isDashboardPage = currentPage === "dashboard";

  if (!uiAuthReady) {
    return (
      <div className="theme-shell min-h-screen px-4 py-8">
        <div className="mx-auto grid max-w-5xl gap-6 lg:grid-cols-[1.1fr_0.9fr]">
          <div className="theme-panel p-6 sm:p-8">
            <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">{t("欢迎", "Welcome")}</p>
            <h1 className="mt-4 flex items-center gap-2 text-2xl font-bold sm:text-3xl">
              <span>🦞</span>
              <span>{t("进入 RustClaw 控制台", "Enter RustClaw Console")}</span>
            </h1>
            <p className="mt-4 max-w-xl text-sm leading-7 text-white/70 sm:text-base">
              {t(
                "这是给普通用户准备的可视化面板。你不需要先懂命令行，只要填好服务地址和访问 key，就能查看状态、绑定账号、测试消息。",
                "This is a visual panel designed for everyday users. You do not need the command line first; enter the service address and access key to check status, bind accounts, and test messages.",
              )}
            </p>

            <div className="mt-6 rounded-2xl border border-white/10 bg-black/20 p-4">
              <p className="text-sm font-semibold text-white">{t("登录前你需要什么？", "What do you need before signing in?")}</p>
              <ol className="mt-3 list-decimal space-y-2 pl-5 text-sm text-white/65">
                <li>{t("一个已经启动的 RustClaw 服务地址。", "A running RustClaw service address.")}</li>
                <li>{t("一个有效的 user_key。", "A valid user_key.")}</li>
                <li>{t("如果不知道接下来该做什么，登录后先看首页。", "If you are not sure what to do next, start with Home after signing in.")}</li>
              </ol>
            </div>
          </div>

          <div className="theme-panel p-6">
            <div className="mb-6">
              <h2 className="text-xl font-bold">{t("登录", "Sign in")}</h2>
              <p className="mt-2 text-sm text-white/60">
                {t("先验证 key，验证成功后再进入控制台。", "Verify your key first, then enter the console.")}
              </p>
            </div>

            <div className="space-y-4">
              <label className="block space-y-2">
                <span className="text-xs uppercase tracking-widest text-white/50">
                  {t("RustClaw 服务地址", "RustClaw service URL")}
                </span>
                <input
                  className="theme-input"
                  value={baseUrl}
                  onChange={(e) => setBaseUrl(e.target.value)}
                  placeholder="http://127.0.0.1:8787"
                />
                <p className="text-xs text-white/45">
                  {t("通常就是你打开面板时使用的地址。", "This is usually the same address you use to open the console.")}
                </p>
              </label>

              <label className="block space-y-2">
                <span className="text-xs uppercase tracking-widest text-white/50">{t("访问 Key", "Access Key")}</span>
                <input
                  className="theme-input"
                  value={uiKeyDraft}
                  onChange={(e) => setUiKeyDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void verifyUiKey(uiKeyDraft);
                    }
                  }}
                  placeholder={t("输入已经生成好的 user_key", "Enter an existing user_key")}
                />
                <p className="text-xs text-white/45">
                  {t("如果你不知道这个 key，通常需要找部署 RustClaw 的人帮你生成。", "If you do not know this key, it usually needs to be generated by whoever set up RustClaw.")}
                </p>
              </label>

              {maskedSavedUiKey ? (
                <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-2 text-xs text-white/70">
                  <div>{t("已保存 Key", "Saved key")}: {maskedSavedUiKey}</div>
                  <div className="mt-1 text-white/45">
                    {t("输入新 key 会覆盖已保存的 key。", "Entering a new key will replace the saved key.")}
                  </div>
                </div>
              ) : null}

              {uiAuthError ? (
                <p className="rounded-xl border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                  {uiAuthError}
                </p>
              ) : null}

              <div className="flex flex-wrap items-center gap-3">
                <button
                  onClick={() => void verifyUiKey(uiKeyDraft)}
                  disabled={uiAuthLoading}
                  className="theme-accent-btn"
                >
                  {uiAuthLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                  {t("进入控制台", "Enter Console")}
                </button>
                {uiKey ? (
                  <button
                    onClick={() => void verifyUiKey(uiKey)}
                    disabled={uiAuthLoading}
                    className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-4 py-2 text-sm font-medium text-white transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-60"
                  >
                    {t("使用已保存 Key", "Use saved key")}
                  </button>
                ) : null}
                <button
                  onClick={toggleThemeMode}
                  className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-3 py-2 text-xs hover:bg-white/10 sm:text-sm"
                >
                  {themeMode === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
                  {themeMode === "dark" ? t("日间模式", "Day mode") : t("夜间模式", "Night mode")}
                </button>
                <button
                  onClick={() => setLang((v) => (v === "zh" ? "en" : "zh"))}
                  className="rounded-xl border border-white/15 bg-white/5 px-3 py-2 text-xs hover:bg-white/10"
                >
                  {lang === "zh" ? "中文" : "EN"}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="theme-shell min-h-screen">
      <header className="theme-header sticky top-0 z-40 flex h-16 shrink-0 items-stretch border-b border-white/10 px-3 sm:px-6">
        <div className="mx-auto flex w-full max-w-7xl items-stretch justify-between gap-3">
          <div className="flex min-w-0 items-center py-0">
            <h1 className="flex items-center gap-2 text-lg font-bold tracking-tight sm:text-2xl">
              <span className="text-lg leading-none sm:text-2xl">🦞</span>
              <span className="truncate">RustClaw</span>
            </h1>
          </div>

          <div className="flex h-full items-stretch gap-2">
            {/* 小屏下拉导航，仅在 lg 以下显示 */}
            <div ref={navDropdownRef} className="relative flex h-full items-stretch lg:hidden">
              <button
                type="button"
                onClick={() => setNavDropdownOpen((v) => !v)}
                className="theme-topbar-btn inline-flex h-full min-h-0 items-center gap-1.5 py-1.5"
                aria-expanded={navDropdownOpen}
                aria-haspopup="true"
              >
                <span>{t("导航", "Nav")}</span>
                <ChevronDown className={`h-4 w-4 shrink-0 transition-transform ${navDropdownOpen ? "rotate-180" : ""}`} />
              </button>
              {navDropdownOpen && (
                <div className="absolute right-0 top-full z-50 mt-1 min-w-[200px] rounded-xl border border-white/10 bg-[var(--theme-header-bg)] py-1 shadow-lg backdrop-blur-sm">
                  {navItems.map((item) => {
                    const active = currentPage === item.id;
                    return (
                      <button
                        key={item.id}
                        type="button"
                        onClick={() => {
                          setCurrentPage(item.id);
                          setNavDropdownOpen(false);
                        }}
                        className={`flex w-full items-center gap-2 px-3 py-2.5 text-left text-sm transition ${
                          active ? "theme-nav-active" : "theme-nav-idle"
                        }`}
                      >
                        <span className={active ? "theme-icon-soft" : "text-white/70"}>{item.icon}</span>
                        <span>{item.label}</span>
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
            <button
              onClick={toggleThemeMode}
              className="theme-topbar-btn h-full min-h-0 py-1.5"
              title={themeMode === "dark" ? t("日间模式", "Day mode") : t("夜间模式", "Night mode")}
            >
              {themeMode === "dark" ? <Sun className="h-4 w-4 shrink-0" /> : <Moon className="h-4 w-4 shrink-0" />}
              <span className="hidden sm:inline">
                {themeMode === "dark" ? t("日间模式", "Day mode") : t("夜间模式", "Night mode")}
              </span>
            </button>
            <button
              onClick={() => setLang((v) => (v === "zh" ? "en" : "zh"))}
              className="theme-topbar-btn h-full min-h-0 py-1.5"
            >
              {lang === "zh" ? "中文" : "EN"}
            </button>
            <button
              type="button"
              onClick={logout}
              className="theme-topbar-btn h-full min-h-0 py-1.5"
              title={t("退出登录，需重新输入 key", "Log out; key required to sign in again")}
            >
              {t("退出", "Log out")}
            </button>
          </div>
        </div>
      </header>

      <div className="px-3 py-4 sm:px-6 sm:py-6 lg:pl-[236px]">
        <aside className="fixed left-0 top-16 z-30 hidden h-[calc(100vh-4rem)] w-[220px] overflow-y-auto lg:block">
          <div className="theme-sidebar-shell mx-3 mt-0 sm:mx-4">
            <div className="mb-3 px-1">
              <p className="theme-kicker text-[10px] uppercase tracking-[0.3em]">{t("导航", "Navigation")}</p>
            </div>
            <nav className="flex gap-2 overflow-x-auto pb-1 lg:block lg:space-y-2 lg:overflow-visible">
              {navItems.map((item) => {
                const active = currentPage === item.id;
                return (
                  <button
                    key={item.id}
                    type="button"
                    data-nav-active={active ? "true" : undefined}
                    onClick={(e) => {
                      setCurrentPage(item.id);
                      (e.currentTarget as HTMLButtonElement).blur();
                    }}
                    className={`theme-nav-item min-w-[148px] rounded-2xl border px-3 py-2.5 text-left transition lg:block lg:w-full ${
                      active
                        ? "theme-nav-active"
                        : "theme-nav-idle"
                    }`}
                  >
                    <div className="flex items-center gap-2">
                      <span className={active ? "theme-icon-soft" : "text-white/70"}>{item.icon}</span>
                      <span className="text-sm font-medium leading-5">{item.label}</span>
                    </div>
                  </button>
                );
              })}
            </nav>

            <div className="theme-panel-soft mt-3 p-3.5 text-sm text-white/70">
              <p className="font-medium text-white">{t("当前登录身份", "Current identity")}</p>
              <p className="mt-2 break-all font-mono text-xs text-white/55">{maskedSavedUiKey || "--"}</p>
            </div>
          </div>
        </aside>

        <main className="mx-auto min-w-0 max-w-7xl space-y-4">
          {isDashboardPage ? (
            <section className="theme-panel p-5 sm:p-6">
              <div className="flex flex-col gap-4 xl:flex-row xl:items-end xl:justify-between">
                <div>
                  <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">{t("页面", "Page")}</p>
                  <h2 className="mt-2 text-xl font-semibold tracking-tight sm:text-2xl">{currentPageMeta.title}</h2>
                </div>
                <div className="flex flex-wrap gap-2 text-xs text-white/70 sm:text-sm">
                  <div className="theme-meta-pill">
                    <span className="text-white/45">{t("下一步", "Next")}</span>
                    <span className="ml-2 text-white/80">{suggestedNextStep.title}</span>
                  </div>
                </div>
              </div>
            </section>
          ) : null}

          {isDashboardPage ? (
            <>
              {(queuePressureHigh || runningTooOld || !isOnline) && (
                <section className="rounded-2xl border border-amber-500/30 bg-amber-500/10 p-4">
                  <div className="flex items-start gap-3">
                    <BellRing className="mt-0.5 h-5 w-5 shrink-0 text-amber-300" />
                    <div className="space-y-1 text-sm">
                      <p className="font-semibold text-amber-200">{t("现在有几项需要注意", "A few things need attention")}</p>
                      {!isOnline ? <p className="text-amber-100">- {t("面板现在连不上 RustClaw。先检查服务地址是否正确，或者服务是否已经启动。", "The console cannot reach RustClaw right now. Check the service URL or start the service.")}</p> : null}
                      {queuePressureHigh ? (
                        <p className="text-amber-100">- {t(`排队中的任务有 ${health?.queue_length ?? 0} 个，数量偏多，可能会让回复变慢。`, `There are ${health?.queue_length ?? 0} queued tasks, so replies may be slower than usual.`)}</p>
                      ) : null}
                      {runningTooOld ? (
                        <p className="text-amber-100">
                          - {t(`有任务已经运行了 ${formatDuration(health?.running_oldest_age_seconds)}，时间偏长，建议留意。`, `One task has been running for ${formatDuration(health?.running_oldest_age_seconds)}, which is longer than expected.`)}
                        </p>
                      ) : null}
                    </div>
                  </div>
                </section>
              )}

              <section className="grid gap-4 xl:grid-cols-[minmax(0,1.1fr)_320px]">
                <div className="rounded-2xl border border-white/10 bg-white/5 p-5">
                  <h3 className="text-lg font-semibold sm:text-xl">{t("常用操作", "Common actions")}</h3>
                  <div className="mt-4 space-y-2.5">
                    {[
                      {
                        title: t("先看首页状态", "Check Home first"),
                      },
                      {
                        title: t("绑定通信端账号", "Bind channel account"),
                      },
                      {
                        title: t("试一条最简单的消息", "Send one simple test message"),
                      },
                    ].map((step, index) => (
                      <div key={step.title} className="flex gap-3 rounded-2xl border border-white/10 bg-black/20 px-4 py-3">
                        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-white/10 bg-white/5 text-xs font-semibold text-white/85">
                          {index + 1}
                        </div>
                        <div>
                          <p className="text-sm font-semibold text-white">{step.title}</p>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>

                <div className="rounded-2xl border border-white/10 bg-black/20 p-5">
                  <h3 className="text-xl font-semibold">{suggestedNextStep.title}</h3>
                  <button
                    type="button"
                    onClick={() => setCurrentPage(suggestedNextStep.page)}
                    className="mt-4 w-full theme-accent-btn"
                  >
                    <RefreshCw className="h-4 w-4" />
                    {suggestedNextStep.cta}
                  </button>
                </div>
              </section>

              <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
                <div className="flex items-center gap-2 text-base font-semibold text-white">
                  <Wrench className="theme-icon-accent h-4 w-4" />
                  <span>{t("基础设置", "Basic Settings")}</span>
                </div>
                <div className="mt-4 grid gap-3 md:grid-cols-2 xl:grid-cols-[2fr_1fr_1fr_1fr]">
                  <label className="space-y-2">
                    <span className="text-[10px] uppercase tracking-widest text-white/50">{t("clawd API 地址", "clawd API URL")}</span>
                    <input
                      className="theme-input"
                      value={baseUrl}
                      onChange={(e) => setBaseUrl(e.target.value)}
                      placeholder="http://127.0.0.1:8787"
                    />
                  </label>
                  <label className="space-y-2">
                    <span className="text-[10px] uppercase tracking-widest text-white/50">{t("自动刷新", "Auto Refresh")}</span>
                    <select
                      className="theme-input"
                      value={pollingSeconds}
                      onChange={(e) => setPollingSeconds(Number(e.target.value))}
                    >
                      <option value={3}>{t("3 秒", "3s")}</option>
                      <option value={5}>{t("5 秒", "5s")}</option>
                      <option value={10}>{t("10 秒", "10s")}</option>
                      <option value={0}>{t("关闭", "Off")}</option>
                    </select>
                  </label>
                  <label className="space-y-2">
                    <span className="text-[10px] uppercase tracking-widest text-white/50">{t("队列阈值", "Queue Alert")}</span>
                    <input
                      type="number"
                      min={1}
                      className="theme-input"
                      value={queueWarn}
                      onChange={(e) => setQueueWarn(Math.max(1, Number(e.target.value) || 1))}
                    />
                  </label>
                  <label className="space-y-2">
                    <span className="text-[10px] uppercase tracking-widest text-white/50">{t("运行告警(秒)", "Runtime Alert(s)")}</span>
                    <input
                      type="number"
                      min={10}
                      className="theme-input"
                      value={ageWarnSeconds}
                      onChange={(e) => setAgeWarnSeconds(Math.max(10, Number(e.target.value) || 10))}
                    />
                  </label>
                </div>
                {error ? (
                  <p className="mt-4 rounded-xl border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                    {t("接口错误", "API error")}: {error}
                  </p>
                ) : null}
              </section>

              <section className="grid gap-3 lg:grid-cols-2 xl:grid-cols-4">
                <div className="rounded-2xl border border-white/10 bg-white/5 p-4">
                  <p className="text-[10px] uppercase tracking-widest text-sky-200/70">{t("整体状态", "Overall status")}</p>
                  <p className={`mt-2 text-lg font-semibold ${isOnline ? "text-emerald-300" : "text-red-300"}`}>
                    {isOnline ? t("服务在线", "Service online") : t("当前离线", "Currently offline")}
                  </p>
                  <div className="mt-3 space-y-2 text-sm">
                    <p className="flex items-center justify-between gap-3 rounded-xl border border-white/8 bg-black/15 px-3 py-2">
                      <span className="text-white/45">{t("版本", "Version")}</span>
                      <span className="font-medium text-sky-200">{health?.version || "--"}</span>
                    </p>
                    <p className="flex items-center justify-between gap-3 rounded-xl border border-white/8 bg-black/15 px-3 py-2">
                      <span className="text-white/45">{t("Worker", "Worker")}</span>
                      <span className="font-medium text-violet-200">{health?.worker_state || "--"}</span>
                    </p>
                  </div>
                </div>
                <div className="rounded-2xl border border-white/10 bg-white/5 p-4">
                  <p className="text-[10px] uppercase tracking-widest text-amber-200/70">{t("任务负载", "Task load")}</p>
                  <p className="mt-2 text-lg font-semibold text-amber-100">
                    {t("排队 {{queue}} / 运行 {{running}}", "Queued {{queue}} / Running {{running}}")
                      .replace("{{queue}}", String(health?.queue_length ?? "--"))
                      .replace("{{running}}", String(health?.running_length ?? "--"))}
                  </p>
                  <div className="mt-3 space-y-2 text-sm">
                    <p className="flex items-center justify-between gap-3 rounded-xl border border-white/8 bg-black/15 px-3 py-2">
                      <span className="text-white/45">{t("最久运行", "Oldest task")}</span>
                      <span className="font-medium text-amber-200">{formatDuration(health?.running_oldest_age_seconds)}</span>
                    </p>
                    <p className="flex items-center justify-between gap-3 rounded-xl border border-white/8 bg-black/15 px-3 py-2">
                      <span className="text-white/45">{t("超时阈值", "Timeout")}</span>
                      <span className="font-medium text-orange-200">{formatDuration(health?.task_timeout_seconds)}</span>
                    </p>
                  </div>
                </div>
                <div className="rounded-2xl border border-white/10 bg-white/5 p-4">
                  <p className="text-[10px] uppercase tracking-widest text-emerald-200/70">{t("资源占用", "Resources")}</p>
                  <p className="mt-2 text-lg font-semibold text-emerald-100">{formatBytes(health?.memory_rss_bytes ?? null)}</p>
                  <div className="mt-3 space-y-2 text-sm">
                    <p className="flex items-center justify-between gap-3 rounded-xl border border-white/8 bg-black/15 px-3 py-2">
                      <span className="text-white/45">{t("运行时长", "Uptime")}</span>
                      <span className="font-medium text-emerald-200">{formatDuration(health?.uptime_seconds)}</span>
                    </p>
                    <p className="flex items-center justify-between gap-3 rounded-xl border border-white/8 bg-black/15 px-3 py-2">
                      <span className="text-white/45">{t("刷新频率", "Refresh")}</span>
                      <span className="font-medium text-cyan-200">{pollingSeconds > 0 ? t(`每 ${pollingSeconds} 秒`, `Every ${pollingSeconds}s`) : t("已关闭", "Off")}</span>
                    </p>
                  </div>
                </div>
                <div className="rounded-2xl border border-white/10 bg-black/20 p-4">
                  <p className="text-[10px] uppercase tracking-widest text-white/45">{t("建议处理顺序", "Suggested order")}</p>
                  <ol className="mt-3 list-decimal space-y-2 pl-5 text-sm text-white/65">
                    <li>{t("先看是否在线。", "Check whether the service is online.")}</li>
                    <li>{t("再看有没有积压任务。", "Then check whether tasks are backing up.")}</li>
                    <li>{t("最后再决定要不要进高级页。", "Only then decide whether you need advanced pages.")}</li>
                  </ol>
                </div>
              </section>

              <div className="space-y-4">
                <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
                  <h3 className="mb-2 text-base font-semibold">{t("你现在最可能要做的事", "What you probably want to do next")}</h3>
                  <div className="mt-4 grid gap-3">
                    <QuickActionCard
                      title={t("绑定通信端账号", "Bind channel account")}
                      cta={t("打开绑定账号页", "Open Bind Accounts")}
                      onClick={() => setCurrentPage("channels")}
                      icon={<Database className="h-4 w-4" />}
                    />
                    <QuickActionCard
                      title={t("看看连接是不是正常", "Check whether connections are healthy")}
                      cta={t("打开连接状态页", "Open Connections")}
                      onClick={() => setCurrentPage("services")}
                      icon={<Server className="h-4 w-4" />}
                    />
                    <QuickActionCard
                      title={t("试一条消息", "Try a message")}
                      cta={t("打开对话交互", "Open Chat")}
                      onClick={() => setCurrentPage("chat")}
                      icon={<MessageCircle className="h-4 w-4" />}
                    />
                  </div>
                </section>

                <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
                  <details className="group">
                    <summary className="flex cursor-pointer list-none items-center gap-2 text-base font-semibold text-white">
                      <span>{t("高级调试数据", "Advanced Debug Data")}</span>
                      <span className="ml-auto text-xs font-medium text-white/45">
                        <span className="group-open:hidden">{t("点击展开", "Click to expand")}</span>
                        <span className="hidden group-open:inline">{t("点击收起", "Click to collapse")}</span>
                      </span>
                      <ChevronDown className="h-4 w-4 text-white/55 transition group-open:rotate-180" />
                    </summary>
                    <pre className="mt-4 max-h-72 overflow-auto rounded-xl border border-white/10 bg-[#12151f] p-3 text-xs text-white/80">
                      {JSON.stringify(health, null, 2)}
                    </pre>
                  </details>
                </section>
              </div>
            </>
          ) : null}

          {currentPage === "chat" ? (
            <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
              <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
                <h3 className="text-base font-semibold">{t("和 RustClaw 对话", "Chat with RustClaw")}</h3>
                <div className="flex flex-wrap items-center gap-3 text-sm">
                  <label className="inline-flex items-center gap-2 text-white/80">
                    <input type="checkbox" checked={chatAgentMode} onChange={(e) => setChatAgentMode(e.target.checked)} />
                    agent_mode
                  </label>
                  <button
                    onClick={() =>
                      setChatMessages([
                        {
                          id: `chat-clear-${Date.now()}`,
                          role: "system",
                          text: t("聊天记录已清空。", "Chat history cleared."),
                          ts: Date.now(),
                        },
                      ])
                    }
                    className="rounded-lg border border-white/15 bg-white/5 px-3 py-1.5 text-xs hover:bg-white/10"
                  >
                    {t("清空记录", "Clear")}
                  </button>
                </div>
              </div>

              <div className="h-80 space-y-3 overflow-auto rounded-xl border border-white/10 bg-black/30 p-3">
                {chatMessages.map((msg) => (
                  <div key={msg.id} className="space-y-1">
                    <div className="flex items-center gap-2 text-[11px] text-white/50">
                      <span>{msg.role}</span>
                      <span>{toLocalTime(msg.ts)}</span>
                    </div>
                    <div
                      className={
                        msg.role === "user"
                          ? "theme-user-bubble max-w-[95%] rounded-xl px-3 py-2 text-sm text-white"
                          : msg.role === "assistant"
                            ? "max-w-[95%] rounded-xl bg-emerald-500/15 px-3 py-2 text-sm text-white"
                            : "max-w-[95%] rounded-xl bg-white/10 px-3 py-2 text-sm text-white/80"
                      }
                    >
                      {msg.role === "assistant" ? (
                        <div className="chat-markdown">
                          <ReactMarkdown>{msg.text}</ReactMarkdown>
                        </div>
                      ) : (
                        <pre className="whitespace-pre-wrap break-words font-sans">{msg.text}</pre>
                      )}
                      {msg.images && msg.images.length > 0 ? (
                        <div className="mt-3 flex flex-wrap gap-2">
                          {msg.images.map((image) => (
                            <img
                              key={`${msg.id}-${image.name}`}
                              src={image.dataUrl}
                              alt={image.name}
                              className="max-h-40 rounded-lg border border-white/10 object-contain"
                            />
                          ))}
                        </div>
                      ) : null}
                    </div>
                  </div>
                ))}
              </div>

              <div className="mt-4 grid shrink-0 gap-3 md:grid-cols-[1fr_auto]">
                <div className="min-w-0">
                  {chatImageAttachments.length > 0 ? (
                    <div className="mb-3 flex flex-wrap gap-2 rounded-xl border border-white/10 bg-white/5 p-2">
                      {chatImageAttachments.map((image, index) => (
                        <div key={`${image.name}-${index}`} className="relative">
                          <img
                            src={image.dataUrl}
                            alt={image.name}
                            className="h-20 w-20 rounded-lg border border-white/10 object-cover"
                          />
                          <button
                            type="button"
                            onClick={() => removeChatImageAttachment(index)}
                            className="absolute -right-2 -top-2 rounded-full border border-white/15 bg-black/70 p-1 text-white/80 hover:bg-black/85"
                            title={t("移除图片", "Remove image")}
                          >
                            <X className="h-3 w-3" />
                          </button>
                        </div>
                      ))}
                    </div>
                  ) : null}
                  <div className="mb-3 flex flex-wrap items-center gap-2">
                    <input
                      ref={chatImageInputRef}
                      type="file"
                      accept="image/*"
                      multiple
                      className="hidden"
                      onChange={(e) => void handleChatImageSelection(e.target.files)}
                    />
                    <button
                      type="button"
                      onClick={() => chatImageInputRef.current?.click()}
                      className="rounded-lg border border-white/15 bg-white/5 px-3 py-1.5 text-xs hover:bg-white/10"
                    >
                      {t("选图片", "Choose images")}
                    </button>
                    <span className="text-xs text-white/45">
                      {t("可直接发图，也可以带一句说明。", "You can send images directly or add a short instruction.")}
                    </span>
                  </div>
                  <textarea
                    className="theme-input min-h-24 w-full resize-none"
                    placeholder={t("例如：你好，请告诉我你现在能做什么；或发一张图片让我看看", "For example: Hello, tell me what you can do; or send an image for analysis")}
                    value={chatInput}
                    onChange={(e) => setChatInput(e.target.value)}
                    onKeyDown={handleChatInputKeyDown}
                  />
                </div>
                <button
                  onClick={() => void sendChatMessage()}
                  disabled={chatSending || (!chatInput.trim() && chatImageAttachments.length === 0)}
                  className="theme-accent-btn shrink-0"
                >
                  {chatSending ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                  {t("发送", "Send")}
                </button>
              </div>
              {chatError ? (
                <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                  {t("聊天错误", "Chat error")}: {chatError}
                </p>
              ) : null}
            </section>
          ) : null}

          {currentPage === "services" ? (
            <div className="space-y-5">
              {serviceActionMessage ? (
                <p className="rounded-2xl border border-white/10 bg-white/5 px-4 py-3 text-sm text-white/80">
                  {serviceActionMessage}
                </p>
              ) : null}

              <section className="grid gap-4 xl:grid-cols-[320px_minmax(0,1fr)]">
                <div className="theme-panel-soft p-5">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <p className="text-[10px] uppercase tracking-[0.28em] text-white/45">{t("整体状态", "System")}</p>
                      <h3 className="mt-2 text-xl font-semibold">{isOnline ? t("服务在线", "Service online") : t("当前离线", "Currently offline")}</h3>
                    </div>
                    <span className={isOnline ? "rounded-full border border-emerald-500/25 bg-emerald-500/10 px-3 py-1 text-xs text-emerald-200" : "rounded-full border border-red-500/25 bg-red-500/10 px-3 py-1 text-xs text-red-200"}>
                      {isOnline ? t("可访问", "Reachable") : t("不可访问", "Offline")}
                    </span>
                  </div>

                  <div className="mt-4 rounded-2xl border border-white/10 bg-black/20 px-4 py-3">
                    <p className="text-sm font-medium">{t("RustClaw 主服务", "RustClaw core service")}</p>
                    <p className="mt-1 break-all text-xs text-white/45">{apiBase}</p>
                  </div>

                  <div className="mt-4 grid gap-2 sm:grid-cols-2 xl:grid-cols-1">
                    <div className="theme-service-kpi">
                      {t("已就绪", "Ready")} {serviceGroupCounts.ready}
                    </div>
                    <div className="theme-service-kpi">
                      {t("待处理", "Needs attention")} {serviceGroupCounts.attention}
                    </div>
                    <div className="theme-service-kpi">
                      {t("未运行", "Stopped")} {serviceGroupCounts.stopped}
                    </div>
                    <div className="theme-service-kpi">
                      {t("已绑定渠道", "Bound channels")} {health?.bound_channel_count ?? "--"}
                    </div>
                  </div>

                  <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-1">
                    <div className="rounded-2xl border border-white/10 bg-black/20 px-4 py-3">
                      <p className="text-[10px] uppercase tracking-widest text-white/45">{t("任务负载", "Task load")}</p>
                      <p className="mt-2 text-base font-semibold text-white/88">
                        {t("排队 {{queue}} / 运行 {{running}}", "Queued {{queue}} / Running {{running}}")
                          .replace("{{queue}}", String(health?.queue_length ?? "--"))
                          .replace("{{running}}", String(health?.running_length ?? "--"))}
                      </p>
                    </div>
                    <div className="rounded-2xl border border-white/10 bg-black/20 px-4 py-3">
                      <p className="text-[10px] uppercase tracking-widest text-white/45">{t("异常服务", "Problem services")}</p>
                      <p className="mt-2 text-base font-semibold text-white/88">{unavailableServiceCount}</p>
                    </div>
                  </div>

                  <button onClick={() => void fetchHealth()} disabled={loading} className="theme-accent-soft-btn mt-4">
                    {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                    {t("刷新状态", "Refresh")}
                  </button>
                </div>

                <section className="theme-panel-soft p-5">
                  <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
                    <div className="flex items-center gap-2">
                      <Server className="theme-icon-accent h-4 w-4" />
                      <h3 className="text-base font-semibold">{t("连接服务", "Services")}</h3>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      <span className="theme-service-kpi">{t("已就绪", "Ready")} {serviceGroupCounts.ready}</span>
                      <span className="theme-service-kpi">{t("未运行", "Stopped")} {serviceGroupCounts.stopped}</span>
                    </div>
                  </div>
                  <div className="grid gap-3 xl:grid-cols-2">
                    {serviceStatusRows.map((row) => (
                      <div key={row.key} className="theme-service-card">
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0">
                            <div className="flex items-center gap-2">
                              <Server className="h-4 w-4 shrink-0 text-white/70" />
                              <span className="truncate text-sm font-medium text-white/90">{row.label}</span>
                            </div>
                            <p className="mt-2 text-xs leading-6 text-white/60">{row.detail}</p>
                          </div>
                          <span
                            className={
                              row.category === "ready"
                                ? "inline-flex items-center gap-1.5 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-xs text-emerald-200"
                                : row.category === "attention"
                                  ? "inline-flex items-center gap-1.5 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-xs text-emerald-200"
                                  : row.category === "stopped"
                                    ? "inline-flex items-center gap-1.5 rounded-lg border border-red-500/30 bg-red-500/10 px-2 py-1 text-xs text-red-200"
                                : "inline-flex items-center gap-1.5 rounded-lg border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/55"
                            }
                          >
                            {row.category === "ready" || row.category === "attention" ? (
                              <span className="h-2 w-2 shrink-0 rounded-full bg-emerald-400 animate-pulse" title={row.statusLabel} aria-label={row.statusLabel} />
                            ) : row.category === "stopped" ? (
                              <span className="h-2 w-2 shrink-0 rounded-full bg-red-400" title={row.statusLabel} aria-label={row.statusLabel} />
                            ) : (
                              <span className="h-2 w-2 shrink-0 rounded-full bg-white/50" title={row.statusLabel} aria-label={row.statusLabel} />
                            )}
                          </span>
                        </div>
                        <div className="mt-4 flex flex-wrap gap-2">
                          <span className="theme-service-kpi">{t("进程", "Processes")} {row.processCount == null ? "--" : row.processCount}</span>
                          <span className="theme-service-kpi">RSS {formatBytes(row.memoryRssBytes ?? null)}</span>
                        </div>
                        <div className="mt-4 flex flex-wrap gap-2">
                          {row.key === "whatsapp_web" && waLoginStatus?.connected !== true ? (
                            <button
                              onClick={() => setWaLoginDialogOpen(true)}
                              className="theme-service-action theme-service-action-extra"
                            >
                              {tSlash("扫码登录 / QR Login")}
                            </button>
                          ) : null}
                          {row.key === "whatsapp_web" && waLoginStatus?.connected === true ? (
                            <button
                              onClick={() => void logoutWhatsappWeb()}
                              disabled={waLogoutLoading}
                              className="theme-service-action theme-service-action-stop"
                            >
                              {waLogoutLoading ? tSlash("处理中 / Working") : tSlash("退出登录 / Logout")}
                            </button>
                          ) : null}
                          <button
                            onClick={() => void controlService(row.serviceName, "start")}
                            disabled={Boolean(serviceActionLoading[row.serviceName]) || row.healthy === true}
                            className="theme-service-action theme-service-action-start"
                          >
                            {serviceActionLoading[row.serviceName] ? tSlash("处理中 / Working") : tSlash("启动 / Start")}
                          </button>
                          <button
                            onClick={() => void controlService(row.serviceName, "stop")}
                            disabled={Boolean(serviceActionLoading[row.serviceName]) || row.healthy !== true}
                            className="theme-service-action theme-service-action-stop"
                          >
                            {serviceActionLoading[row.serviceName] ? tSlash("处理中 / Working") : tSlash("停止 / Stop")}
                          </button>
                          <button
                            onClick={() => void controlService(row.serviceName, "restart")}
                            disabled={Boolean(serviceActionLoading[row.serviceName])}
                            className="theme-service-action theme-service-action-restart"
                            title={tSlash("先停止再启动 / Stop then start")}
                          >
                            {serviceActionLoading[row.serviceName] ? tSlash("处理中 / Working") : tSlash("重启 / Restart")}
                          </button>
                        </div>
                      </div>
                    ))}
                  </div>
                </section>
              </section>

              <section className="grid gap-4 xl:grid-cols-[1.1fr_0.9fr]">
                <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
                  <div className="mb-4 flex items-center justify-between gap-3">
                    <h3 className="text-base font-semibold">{tSlash("WhatsApp Web 接入 / WhatsApp Web Access")}</h3>
                    <button
                      onClick={() => setWaLoginDialogOpen((open) => !open)}
                      className="rounded-xl border border-white/15 bg-white/5 px-3 py-2 text-xs hover:bg-white/10"
                    >
                      {waLoginDialogOpen ? t("收起", "Collapse") : t("展开", "Expand")}
                    </button>
                  </div>
                  <div className="rounded-xl border border-white/10 bg-black/20 px-4 py-3">
                    <div className="flex flex-wrap items-center justify-between gap-3">
                      <div>
                        <p className="text-sm text-white/85">{tSlash("连接状态 / Connection")}</p>
                        <p className="mt-1 text-xs text-white/45">
                          {waLoginStatus?.last_update_ts
                            ? `${t("最近更新", "Updated")} ${toLocalTime(waLoginStatus.last_update_ts * 1000)}`
                            : t("尚未获取状态", "No status yet")}
                        </p>
                      </div>
                      <span
                        className={
                          waLoginStatus?.connected
                            ? "rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-xs text-emerald-200"
                            : "rounded-lg border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-xs text-amber-200"
                        }
                      >
                        {waLoginStatus?.connected ? tSlash("已登录 / Connected") : tSlash("未登录 / Not Connected")}
                      </span>
                    </div>
                  </div>

                  {waLoginDialogOpen ? (
                    <div className="mt-4 rounded-xl border border-white/10 bg-black/20 p-4">
                      {!waWebBridgeReachable ? (
                        <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
                          {tSlash("WhatsApp Web 服务未运行，请先启动 wa-web-bridge 后再使用扫码登录。 / WhatsApp Web service not running. Start wa-web-bridge first to use scan login.")}
                        </p>
                      ) : (
                        <>
                          <div className="mb-3 flex items-center justify-between gap-3">
                            <h4 className="text-sm font-semibold">{tSlash("扫码区 / QR Panel")}</h4>
                            <button
                              onClick={() => void fetchWhatsappWebLoginStatus()}
                              disabled={waLoginLoading}
                              className="inline-flex items-center gap-2 rounded-xl bg-white/10 px-3 py-2 text-xs hover:bg-white/20 disabled:opacity-50"
                            >
                              {waLoginLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                              {tSlash("刷新状态 / Refresh")}
                            </button>
                          </div>
                          {waLoginStatus?.connected ? (
                            <p className="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
                              {tSlash("WhatsApp Web 已登录，无需扫码。 / WhatsApp Web already connected.")}
                            </p>
                          ) : waLoginStatus?.qr_data_url ? (
                            <div className="inline-block rounded-xl border border-white/15 bg-white p-3">
                              <img src={waLoginStatus.qr_data_url} alt="WhatsApp QR" className="h-56 w-56" />
                            </div>
                          ) : (
                            <p className="rounded-lg border border-white/10 bg-black/30 px-3 py-2 text-sm text-white/70">
                              {waLoginLoading
                                ? tSlash("正在拉取二维码... / Fetching QR...")
                                : tSlash("暂无可用二维码，请稍候或重启 whatsapp_webd。 / QR not ready yet, please wait or restart whatsapp_webd.")}
                            </p>
                          )}
                          {waLoginStatus?.last_error ? (
                            <p className="mt-3 text-xs text-amber-300">
                              {tSlash("最近错误 / Last error")}: {waLoginStatus.last_error}
                            </p>
                          ) : null}
                          {waLoginError ? (
                            <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                              {waLoginError}
                            </p>
                          ) : null}
                        </>
                      )}
                    </div>
                  ) : null}
                </section>

                <section className="space-y-4">
                  <div className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
                    <div className="mb-4 flex items-center gap-2">
                      <Timer className="theme-icon-accent h-4 w-4" />
                      <h3 className="text-base font-semibold">{tSlash("预留适配器 / Future Adapters")}</h3>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      {(health?.future_adapters_enabled?.length ?? 0) > 0 ? (
                        health?.future_adapters_enabled?.map((name) => (
                          <span key={name} className="rounded-md border border-amber-400/30 bg-amber-500/10 px-2 py-1 text-xs text-amber-200">
                            {name}
                          </span>
                        ))
                      ) : (
                        <span className="text-xs text-white/50">{t("当前没有启用的 future adapters。", "No future adapters enabled right now.")}</span>
                      )}
                    </div>
                  </div>

                  <div className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
                    <div className="mb-4 flex items-center gap-2">
                      <AlertCircle className="theme-icon-accent h-4 w-4" />
                      <h3 className="text-base font-semibold">{tSlash("下一步 / Next Step")}</h3>
                    </div>
                    <button
                      type="button"
                      onClick={() => setCurrentPage("channels")}
                      className="theme-accent-soft-btn"
                    >
                      <Database className="h-4 w-4" />
                      {tSlash("打开渠道 / 诊断 / Open Channels / Diagnostics")}
                    </button>
                  </div>
                </section>
              </section>
            </div>
          ) : null}

          {currentPage === "channels" ? (
            <div className="space-y-4">
              <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
                <h3 className="text-base font-semibold">{t("绑定账号", "Bind Accounts")}</h3>
                <p className="mt-2 text-sm text-white/65">
                  {t("列出后端所有 Key，并生成新 Key。仅 admin 可查看与生成。使用 Key 绑定通信端。", "List all keys from the backend and generate new keys. Admin only. Use a key to bind channel accounts.")}
                </p>
                <div className="mt-4 flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    onClick={() => void fetchAuthKeys()}
                    disabled={authKeysLoading}
                    className="theme-topbar-btn px-3 py-2 text-sm"
                  >
                    {authKeysLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                    {t("刷新列表", "Refresh list")}
                  </button>
                  <button
                    type="button"
                    onClick={() => void createAuthKey("user")}
                    disabled={authKeyCreateLoading || !isAdminIdentity}
                    className="theme-accent-btn px-3 py-2 text-sm"
                  >
                    {authKeyCreateLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                    {t("生成新 Key（user）", "Generate new key (user)")}
                  </button>
                  <button
                    type="button"
                    onClick={() => void createAuthKey("admin")}
                    disabled={authKeyCreateLoading || !isAdminIdentity}
                    className="theme-secondary-btn px-3 py-2 text-sm"
                  >
                    {authKeyCreateLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                    {t("生成新 Key（admin）", "Generate new key (admin)")}
                  </button>
                </div>
                {!isAdminIdentity ? (
                  <p className="mt-3 rounded-lg border border-amber-400/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
                    {t("当前不是 admin 账号：可查看页面，但不能创建/修改/删除账号。", "Current key is not admin: view-only mode for account management.")}
                  </p>
                ) : null}
                {authKeysError ? (
                  <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{authKeysError}</p>
                ) : null}
                {authKeyCreateError ? (
                  <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{authKeyCreateError}</p>
                ) : null}
                {authKeyActionError ? (
                  <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{authKeyActionError}</p>
                ) : null}
                {newlyCreatedKey ? (
                  <div className="mt-4 rounded-xl border border-emerald-500/30 bg-emerald-500/10 p-4">
                    <p className="text-sm font-medium text-emerald-200">{t("新 Key 已生成，请复制保存（只显示一次）", "New key generated. Copy and save it (shown once).")}</p>
                    <p className="mt-2 break-all font-mono text-sm text-white/90">{newlyCreatedKey}</p>
                    <button
                      type="button"
                      onClick={() => setNewlyCreatedKey(null)}
                      className="mt-2 text-xs text-white/70 underline"
                    >
                      {t("关闭", "Dismiss")}
                    </button>
                  </div>
                ) : null}
                <div className="mt-4 rounded-xl border border-white/10 bg-black/20 overflow-hidden">
                  <table className="w-full text-left text-sm">
                    <thead>
                      <tr className="border-b border-white/10 bg-white/5">
                        <th className="px-4 py-3 font-medium text-white/80">{t("Key（脱敏）", "Key (masked)")}</th>
                        <th className="px-4 py-3 font-medium text-white/80">role</th>
                        <th className="px-4 py-3 font-medium text-white/80">{t("启用", "Enabled")}</th>
                        <th className="px-4 py-3 font-medium text-white/80">{t("创建时间", "Created")}</th>
                        <th className="px-4 py-3 font-medium text-white/80">{t("最后使用", "Last used")}</th>
                        <th className="px-4 py-3 font-medium text-white/80">{t("操作", "Actions")}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {authKeysList.length === 0 && !authKeysLoading ? (
                        <tr>
                          <td colSpan={6} className="px-4 py-6 text-center text-white/50">
                            {t("暂无数据，点击「刷新列表」或「生成新 Key」", "No keys yet. Click Refresh list or Generate new key.")}
                          </td>
                        </tr>
                      ) : (
                        authKeysList.map((row) => (
                          <tr key={row.key_id} className="border-b border-white/5">
                            <td className="px-4 py-2 font-mono text-white/85">{row.user_key_masked}</td>
                            <td className="px-4 py-2 text-white/75">{row.role}</td>
                            <td className="px-4 py-2">{row.enabled ? t("是", "Yes") : t("否", "No")}</td>
                            <td className="px-4 py-2 text-white/65">{formatDateTimeHuman(row.created_at)}</td>
                            <td className="px-4 py-2 text-white/65">{formatDateTimeHuman(row.last_used_at)}</td>
                            <td className="px-4 py-2">
                              {isAdminIdentity ? (
                                <div className="flex flex-wrap items-center gap-2">
                                  <button
                                    type="button"
                                    disabled={authKeyActionLoading === row.key_id}
                                    className="theme-topbar-btn px-2 py-1 text-xs"
                                    onClick={() => void updateAuthKey(row.key_id, { enabled: !row.enabled })}
                                  >
                                    {row.enabled ? t("禁用", "Disable") : t("启用", "Enable")}
                                  </button>
                                  <button
                                    type="button"
                                    disabled={authKeyActionLoading === row.key_id}
                                    className="theme-secondary-btn px-2 py-1 text-xs"
                                    onClick={() => void updateAuthKey(row.key_id, { role: row.role === "admin" ? "user" : "admin" })}
                                  >
                                    {row.role === "admin" ? t("设为 user", "Set as user") : t("设为 admin", "Set as admin")}
                                  </button>
                                  <button
                                    type="button"
                                    disabled={authKeyActionLoading === row.key_id}
                                    className="rounded-md border border-red-500/30 bg-red-500/10 px-2 py-1 text-xs text-red-200 transition hover:bg-red-500/20 disabled:opacity-50"
                                    onClick={() => void deleteAuthKey(row)}
                                  >
                                    {t("删除", "Delete")}
                                  </button>
                                </div>
                              ) : (
                                <span className="text-xs text-white/45">--</span>
                              )}
                            </td>
                          </tr>
                        ))
                      )}
                    </tbody>
                  </table>
                </div>
              </section>
              <aside className="rounded-2xl border border-white/10 bg-white/5 p-4">
                <p className="text-[10px] uppercase tracking-widest text-white/45">{t("当前登录身份", "Current identity")}</p>
                <p className="mt-2 text-sm text-white/85">
                  {authIdentity ? authIdentity.role : authMeLoading ? t("读取中...", "Loading...") : t("暂无数据", "No data")}
                </p>
                <p className="mt-1 break-all font-mono text-xs text-white/55">{maskStoredKey(authIdentity?.user_key ?? "", 8) || "--"}</p>
              </aside>
            </div>
          ) : null}

          {currentPage === "models" ? (
            <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
              <div
                className={`mb-5 rounded-2xl border px-4 py-4 sm:px-5 ${
                  llmRestartPending ? "border-amber-500/25 bg-amber-500/10" : "border-emerald-500/25 bg-emerald-500/10"
                }`}
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <p className="text-[10px] uppercase tracking-[0.28em] text-white/50">{t("模型状态", "Model status")}</p>
                    <h3 className="mt-2 text-base font-semibold">
                      {llmRestartPending
                        ? t("配置已经改好，还差重启才能生效", "The config is saved, but a restart is still needed")
                        : t("当前运行中的模型和已保存配置一致", "The running model matches the saved config")}
                    </h3>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <span
                      className={
                        llmRestartPending
                          ? "rounded-full border border-amber-500/30 bg-amber-500/10 px-3 py-1 text-xs font-medium text-amber-200"
                          : "rounded-full border border-emerald-500/30 bg-emerald-500/10 px-3 py-1 text-xs font-medium text-emerald-200"
                      }
                    >
                      {llmRestartPending ? t("待重启生效", "Restart required") : t("已同步", "In sync")}
                    </span>
                    {llmRestartPending ? (
                      <button
                        type="button"
                        onClick={() => void restartSystem()}
                        disabled={systemRestarting}
                        className="theme-accent-btn px-3 py-2 text-xs"
                      >
                        {systemRestarting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                        {t("立即重启", "Restart now")}
                      </button>
                    ) : null}
                  </div>
                </div>

                <div className="mt-4 grid gap-3 lg:grid-cols-2">
                  <div className="rounded-xl border border-white/10 bg-black/20 px-4 py-3">
                    <p className="text-[10px] uppercase tracking-widest text-white/45">{t("当前运行中", "Running now")}</p>
                    <p className="mt-2 text-sm font-semibold text-white/90">{llmRuntimeLabel}</p>
                  </div>
                  <div className="rounded-xl border border-white/10 bg-black/20 px-4 py-3">
                    <p className="text-[10px] uppercase tracking-widest text-white/45">{t("已保存配置", "Saved config")}</p>
                    <p className="mt-2 text-sm font-semibold text-white/90">{llmSavedLabel}</p>
                  </div>
                </div>
                {systemRestartMessage ? (
                  <p className="mt-4 rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-sm text-white/80">
                    {systemRestartMessage}
                  </p>
                ) : null}
              </div>

              <div className="mb-5 rounded-2xl border border-white/10 bg-black/20 p-4">
                <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
                  <h3 className="text-base font-semibold">{t("大模型设置", "LLM Settings")}</h3>
                  <div className="flex items-center gap-2">
                    {hasCustomLlmVendor ? (
                      <button
                        type="button"
                        onClick={() => applyLlmVendorDraft("custom")}
                        disabled={llmConfigLoading}
                        className="theme-secondary-btn px-3 py-2 text-xs"
                      >
                        <Sparkles className="h-3.5 w-3.5" />
                        {t("自定义模型", "Custom model")}
                      </button>
                    ) : null}
                    <button
                      onClick={() => { void fetchLlmConfig(); void fetchMultimodalConfig(); }}
                      disabled={llmConfigLoading || multimodalConfigLoading}
                      className="theme-topbar-btn px-3 py-2 text-xs font-medium disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {(llmConfigLoading || multimodalConfigLoading) ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                      {tSlash("刷新模型配置 / Refresh LLM Config")}
                    </button>
                    <button
                      onClick={() => void saveLlmConfig()}
                      disabled={llmConfigSaving || llmConfigLoading || !hasUnsavedLlmChanges || !llmDraftVendor || !llmDraftModel || !llmDraftBaseUrl.trim()}
                      className="theme-accent-btn px-3 py-2 text-xs"
                    >
                      {llmConfigSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Database className="h-3.5 w-3.5" />}
                      {tSlash("保存模型设置 / Save LLM Settings")}
                    </button>
                  </div>
                </div>

                <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_300px]">
                  <div className="space-y-4">
                    <div className="grid gap-4 md:grid-cols-2">
                      <label className="block space-y-2">
                        <span className="text-xs uppercase tracking-widest text-white/50">{t("模型厂商", "Vendor")}</span>
                        <select
                          className="theme-input"
                          value={llmDraftVendor}
                          onChange={(e) => applyLlmVendorDraft(e.target.value)}
                        >
                          <option value="">{t("请选择厂商", "Select a vendor")}</option>
                          {(llmConfigData?.vendors ?? []).map((vendor) => (
                            <option key={vendor.name} value={vendor.name}>
                              {vendor.name === "custom" ? t("custom（自定义）", "custom (Custom)") : vendor.name}
                            </option>
                          ))}
                        </select>
                      </label>

                      <label className="block space-y-2">
                        <span className="text-xs uppercase tracking-widest text-white/50">{t("具体模型", "Model")}</span>
                        <input
                          className="theme-input"
                          value={llmDraftModel}
                          onChange={(e) => setLlmDraftModel(e.target.value)}
                          list={selectedLlmVendorInfo ? `llm-models-${selectedLlmVendorInfo.name}` : undefined}
                          disabled={!selectedLlmVendorInfo}
                          placeholder={selectedLlmVendorInfo ? t("输入模型名", "Enter model name") : t("先选厂商", "Choose a vendor first")}
                        />
                        {selectedLlmVendorInfo ? (
                          <datalist id={`llm-models-${selectedLlmVendorInfo.name}`}>
                            {(selectedLlmVendorInfo.models ?? []).map((model) => (
                              <option key={model} value={model} />
                            ))}
                          </datalist>
                        ) : null}
                        {selectedLlmVendorInfo?.name === "custom" ? (
                          <p className="text-xs text-white/45">{t("自定义厂商下可直接填写任意模型名。", "With the custom vendor, you can enter any model name directly.")}</p>
                        ) : null}
                      </label>
                    </div>

                    <div className="grid gap-4 md:grid-cols-2">
                      <label className="block space-y-2">
                        <span className="text-xs uppercase tracking-widest text-white/50">Base URL</span>
                        <input
                          className="theme-input"
                          value={llmDraftBaseUrl}
                          onChange={(e) => setLlmDraftBaseUrl(e.target.value)}
                          placeholder="https://api.openai.com/v1"
                          disabled={!selectedLlmVendorInfo}
                        />
                      </label>

                      <label className="block space-y-2">
                        <span className="text-xs uppercase tracking-widest text-white/50">API Key</span>
                        <input
                          type="password"
                          className="theme-input"
                          value={llmDraftApiKey}
                          onChange={(e) => setLlmDraftApiKey(e.target.value)}
                          placeholder="****************"
                          autoComplete="off"
                          disabled={!selectedLlmVendorInfo}
                        />
                      </label>
                    </div>

                    {llmConfigError ? (
                      <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                        {tSlash("模型配置读取/保存失败 / LLM config read/save failed")}: {llmConfigError}
                      </p>
                    ) : null}
                    {llmConfigSaveMessage ? (
                      <p className="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
                        {llmConfigSaveMessage}
                      </p>
                    ) : null}
                    {hasUnsavedLlmChanges ? (
                      <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
                        {t("你有未保存的大模型变更，请点击“保存模型设置”。", "You have unsaved LLM changes. Click \"Save LLM Settings\".")}
                      </p>
                    ) : null}
                  </div>

                  <div className="space-y-3 rounded-xl border border-white/10 bg-[#12151f] p-4 text-sm">
                    <div>
                      <p className="text-[10px] uppercase tracking-widest text-white/45">{t("当前厂商信息", "Current vendor info")}</p>
                      {selectedLlmVendorInfo ? (
                        <div className="mt-2 space-y-2 text-xs text-white/65">
                          <p>
                            <span className="text-white/45">{t("默认模型", "Default model")}</span>
                            <span className="ml-2 text-white/80">{selectedLlmVendorInfo.default_model || "--"}</span>
                          </p>
                          <p className="break-all">
                            <span className="text-white/45">Base URL</span>
                            <span className="ml-2 text-white/80">{selectedLlmVendorInfo.base_url || "--"}</span>
                          </p>
                          <p>
                            <span className="text-white/45">{t("API Key", "API Key")}</span>
                            <span className={`ml-2 ${selectedLlmVendorInfo.api_key_configured ? "text-emerald-200" : "text-amber-200"}`}>
                              {selectedLlmVendorInfo.api_key_configured ? t("已配置", "Configured") : t("未配置", "Missing")}
                            </span>
                          </p>
                          {selectedLlmVendorInfo.api_key_masked ? (
                            <p className="break-all">
                              <span className="text-white/45">{t("当前掩码", "Masked key")}</span>
                              <span className="ml-2 text-white/80">{selectedLlmVendorInfo.api_key_masked}</span>
                            </p>
                          ) : null}
                        </div>
                      ) : (
                        <p className="mt-2 text-xs text-white/50">{t("先选择一个厂商。", "Choose a vendor first.")}</p>
                      )}
                    </div>
                  </div>
                </div>
              </div>

              <div className="mt-6 space-y-6">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <h3 className="text-base font-semibold">{t("图像与语音模块", "Image & Audio Modules")}</h3>
                  <div className="flex items-center gap-2">
                    <button
                      type="button"
                      onClick={() => void fetchMultimodalConfig()}
                      disabled={multimodalConfigLoading}
                      className="theme-topbar-btn px-3 py-2 text-xs font-medium disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {multimodalConfigLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                      {t("刷新", "Refresh")}
                    </button>
                    <button
                      type="button"
                      onClick={() => void saveMultimodalConfig()}
                      disabled={multimodalConfigSaving || multimodalConfigLoading || !hasUnsavedMultimodalChanges}
                      className="theme-accent-btn px-3 py-2 text-xs"
                    >
                      {multimodalConfigSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Database className="h-3.5 w-3.5" />}
                      {t("保存图像/语音配置", "Save Image/Audio Config")}
                    </button>
                  </div>
                </div>
                {multimodalConfigError ? (
                  <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{multimodalConfigError}</p>
                ) : null}
                {multimodalConfigSaveMessage ? (
                  <p className="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">{multimodalConfigSaveMessage}</p>
                ) : null}
                {hasUnsavedMultimodalChanges ? (
                  <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
                    {t("你有未保存的图像/语音配置变更。", "You have unsaved image/audio config changes.")}
                  </p>
                ) : null}

                <div className="rounded-2xl border border-white/10 bg-black/20 p-4">
                  <h4 className="mb-3 text-sm font-medium text-white/90">{t("图像模块", "Image Modules")}</h4>
                  <p className="mb-4 text-xs text-white/50">
                    {t("图像编辑、文生图、图像理解可分别配置厂商、模型及该厂商的 API 地址与密钥（写入 configs/image.toml）。", "Configure vendor, model, base URL and API key per image module. Saved to configs/image.toml.")}
                  </p>
                  <div className="space-y-4">
                    {[
                      { key: "image_edit" as const, label: t("图像编辑", "Image Edit") },
                      { key: "image_generation" as const, label: t("文生图", "Image Generate") },
                      { key: "image_vision" as const, label: t("图像理解", "Image Vision") },
                    ].map(({ key, label }) => (
                      <div key={key} className="space-y-2 rounded-xl border border-white/10 bg-[#12151f] px-4 py-3">
                        <div className="flex flex-wrap items-center gap-3">
                          <span className="w-24 shrink-0 text-xs font-medium text-white/80">{label}</span>
                          <input
                            className="theme-input w-28 shrink-0 text-xs"
                            placeholder={t("厂商", "Vendor")}
                            value={multimodalDraft[key]?.vendor ?? ""}
                            onChange={(e) => setMultimodalDraftKey(key, "vendor", e.target.value)}
                          />
                          <input
                            className="theme-input min-w-[140px] flex-1 text-xs"
                            placeholder={t("模型", "Model")}
                            value={multimodalDraft[key]?.model ?? ""}
                            onChange={(e) => setMultimodalDraftKey(key, "model", e.target.value)}
                          />
                        </div>
                        <div className="flex flex-wrap items-center gap-2 pl-[7.5rem]">
                          <input
                            className="theme-input min-w-[200px] flex-1 text-xs"
                            placeholder={t("API 地址 (base_url)", "API URL (base_url)")}
                            value={multimodalDraft[key]?.base_url ?? ""}
                            onChange={(e) => setMultimodalDraftKey(key, "base_url", e.target.value)}
                          />
                          <input
                            className="theme-input min-w-[160px] flex-1 text-xs"
                            type="password"
                            placeholder={t("API Key", "API Key")}
                            value={multimodalDraft[key]?.api_key ?? ""}
                            onChange={(e) => setMultimodalDraftKey(key, "api_key", e.target.value)}
                          />
                        </div>
                      </div>
                    ))}
                  </div>
                </div>

                <div className="rounded-2xl border border-white/10 bg-black/20 p-4">
                  <h4 className="mb-3 text-sm font-medium text-white/90">{t("声音模块", "Audio Modules")}</h4>
                  <p className="mb-4 text-xs text-white/50">
                    {t("语音合成、语音转写可分别配置厂商、模型及该厂商的 API 地址与密钥（写入 configs/audio.toml）。", "Configure vendor, model, base URL and API key per audio module. Saved to configs/audio.toml.")}
                  </p>
                  <div className="space-y-4">
                    {[
                      { key: "audio_synthesize" as const, label: t("语音合成", "Audio TTS") },
                      { key: "audio_transcribe" as const, label: t("语音转写", "Audio STT") },
                    ].map(({ key, label }) => (
                      <div key={key} className="space-y-2 rounded-xl border border-white/10 bg-[#12151f] px-4 py-3">
                        <div className="flex flex-wrap items-center gap-3">
                          <span className="w-24 shrink-0 text-xs font-medium text-white/80">{label}</span>
                          <input
                            className="theme-input w-28 shrink-0 text-xs"
                            placeholder={t("厂商", "Vendor")}
                            value={multimodalDraft[key]?.vendor ?? ""}
                            onChange={(e) => setMultimodalDraftKey(key, "vendor", e.target.value)}
                          />
                          <input
                            className="theme-input min-w-[140px] flex-1 text-xs"
                            placeholder={t("模型", "Model")}
                            value={multimodalDraft[key]?.model ?? ""}
                            onChange={(e) => setMultimodalDraftKey(key, "model", e.target.value)}
                          />
                        </div>
                        <div className="flex flex-wrap items-center gap-2 pl-[7.5rem]">
                          <input
                            className="theme-input min-w-[200px] flex-1 text-xs"
                            placeholder={t("API 地址 (base_url)", "API URL (base_url)")}
                            value={multimodalDraft[key]?.base_url ?? ""}
                            onChange={(e) => setMultimodalDraftKey(key, "base_url", e.target.value)}
                          />
                          <input
                            className="theme-input min-w-[160px] flex-1 text-xs"
                            type="password"
                            placeholder={t("API Key", "API Key")}
                            value={multimodalDraft[key]?.api_key ?? ""}
                            onChange={(e) => setMultimodalDraftKey(key, "api_key", e.target.value)}
                          />
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            </section>
          ) : null}

          {currentPage === "skills" ? (
            <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
              <div className="mb-5">
                <div className="rounded-2xl border border-sky-500/20 bg-sky-500/10 p-4 sm:p-5">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <p className="text-[10px] uppercase tracking-[0.28em] text-sky-100/70">{t("导入外部技能", "Import External Skills")}</p>
                      <h3 className="mt-2 text-base font-semibold text-white">
                        {t("把别人做好的技能接入进来，扩展 RustClaw 的能力。", "Bring in ready-made skills to extend what RustClaw can do.")}
                      </h3>
                      <p className="mt-2 text-sm text-white/65">
                        {t(
                          "你可以贴一个技能链接，也可以直接上传本地技能文件夹或文件。导入完成后，再决定要不要启用它。",
                          "You can paste a skill link, or directly upload a local skill folder or file. After import, you can decide whether to enable it.",
                        )}
                      </p>
                    </div>
                    <Sparkles className="mt-1 h-4 w-4 shrink-0 text-sky-200" />
                  </div>
                  <div className="mt-4 grid gap-3 md:grid-cols-[minmax(0,1fr)_auto]">
                    <label className="block space-y-2">
                      <span className="text-[10px] uppercase tracking-widest text-sky-100/70">{t("技能链接或文件夹", "Skill link or folder")}</span>
                      <input
                        className="theme-input"
                        value={skillImportSource}
                        onChange={(e) => setSkillImportSource(e.target.value)}
                        placeholder={t(
                          "例如一个技能链接，或一个本地技能文件夹",
                          "For example, a skill link or a local skill folder",
                        )}
                      />
                    </label>
                    <div className="flex items-end">
                      <button
                        type="button"
                        onClick={() => void importExternalSkill()}
                        disabled={skillImportLoading}
                        className="theme-accent-btn px-4 py-2.5 text-sm"
                      >
                        {skillImportLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <Sparkles className="h-4 w-4" />}
                        {t("导入 Skill", "Import Skill")}
                      </button>
                    </div>
                  </div>
                  <div className="mt-3">
                    <div className="relative inline-flex">
                      <button
                        type="button"
                        onClick={() => setLocalImportPickerOpen((prev) => !prev)}
                        disabled={skillImportLoading}
                        className="inline-flex items-center gap-2 rounded-xl border border-white/20 bg-white/5 px-3 py-2 text-xs text-white/85 hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        {t("选择本地技能", "Choose Local Skill")}
                        <ChevronDown className={`h-3.5 w-3.5 transition-transform ${localImportPickerOpen ? "rotate-180" : ""}`} />
                      </button>
                      {localImportPickerOpen ? (
                        <div className="absolute left-0 top-full z-20 mt-2 min-w-[12rem] rounded-xl border border-white/10 bg-[#12151f] p-1.5 shadow-2xl">
                          <button
                            type="button"
                            onClick={() => {
                              setLocalImportPickerOpen(false);
                              folderImportInputRef.current?.click();
                            }}
                            className="flex w-full items-center justify-between rounded-lg px-3 py-2 text-left text-xs text-white/85 hover:bg-white/5"
                          >
                            <span>{t("从文件夹导入", "Import Folder")}</span>
                            <span className="text-[10px] text-white/40">{t("适合整个技能包", "Full bundle")}</span>
                          </button>
                          <button
                            type="button"
                            onClick={() => {
                              setLocalImportPickerOpen(false);
                              fileImportInputRef.current?.click();
                            }}
                            className="flex w-full items-center justify-between rounded-lg px-3 py-2 text-left text-xs text-white/85 hover:bg-white/5"
                          >
                            <span>{t("从文件导入", "Import File")}</span>
                            <span className="text-[10px] text-white/40">{t("适合单个 SKILL.md", "Single file")}</span>
                          </button>
                        </div>
                      ) : null}
                    </div>
                    <input
                      ref={folderImportInputRef}
                      type="file"
                      className="hidden"
                      multiple
                      onChange={(e) => void uploadImportedSkillFiles(e.target.files)}
                      {...({ webkitdirectory: "", directory: "" } as Record<string, string>)}
                    />
                    <input
                      ref={fileImportInputRef}
                      type="file"
                      className="hidden"
                      multiple
                      onChange={(e) => void uploadImportedSkillFiles(e.target.files)}
                    />
                  </div>
                  {skillImportError ? (
                    <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                      {skillImportError}
                    </p>
                  ) : null}
                  {skillImportMessage ? (
                    <p className="mt-3 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
                      {skillImportMessage}
                    </p>
                  ) : null}
                  {systemRestartMessage ? (
                    <p className="mt-3 rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-sm text-white/80">
                      {systemRestartMessage}
                    </p>
                  ) : null}
                  {skillImportPreview ? (
                    <div className="mt-3 rounded-lg border border-white/10 bg-[#12151f] px-3 py-3 text-xs text-white/75">
                      <div className="flex flex-wrap items-start justify-between gap-2">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="rounded-md border border-sky-400/30 bg-sky-500/10 px-2 py-1 text-sky-200">{skillImportPreview.skill_name}</span>
                          <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-white/70">{skillImportPreview.external_kind}</span>
                          {skillImportPreview.runtime ? (
                            <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-white/70">{skillImportPreview.runtime}</span>
                          ) : null}
                        </div>
                        <button
                          type="button"
                          onClick={() => setSkillImportPreview(null)}
                          className="rounded-md border border-white/15 bg-white/5 px-2 py-1 text-[11px] text-white/65 hover:bg-white/10 hover:text-white/85"
                        >
                          {t("收起", "Dismiss")}
                        </button>
                      </div>
                      <p className="mt-2 text-sm text-white/85">{skillImportPreview.description}</p>
                      <p className="mt-2 text-sm text-emerald-200">
                        {t(
                          "下面的技能列表里已经帮你定位到它了。点“设为开启”，再点右上角“保存开关”，确认后系统会自动重启。",
                          "It is now highlighted in the skill list below. Choose Enable, then click Save Switches. The system will restart automatically after you confirm.",
                        )}
                      </p>
                      {skillImportPreview.require_bins.length > 0 ? (
                        <p className="mt-2 text-white/55">{t("需要这些本地工具", "Needs these local tools")}: {skillImportPreview.require_bins.join(", ")}</p>
                      ) : null}
                      {skillImportPreview.require_py_modules.length > 0 ? (
                        <p className="mt-1 text-white/55">{t("还需要这些 Python 依赖", "Also needs these Python packages")}: {skillImportPreview.require_py_modules.join(", ")}</p>
                      ) : null}
                    </div>
                  ) : null}
                </div>
              </div>

              <div className="rounded-xl border border-white/10 bg-black/20 p-4">
                <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
                  <h4 className="text-sm font-semibold">{t("技能开关", "Skill Switches")}</h4>
                  <div className="flex items-center gap-2">
                    <button
                      onClick={() => void fetchSkillsConfig()}
                      disabled={skillsConfigLoading}
                      className="theme-topbar-btn px-3 py-2 text-xs font-medium disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {skillsConfigLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                      {tSlash("刷新配置 / Refresh Config")}
                    </button>
                    <button
                      onClick={() => void saveSkillSwitches()}
                      disabled={skillSwitchSaving || skillsConfigLoading || !hasUnsavedSkillSwitchChanges}
                      className="theme-accent-btn px-3 py-2 text-xs"
                    >
                      {skillSwitchSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Database className="h-3.5 w-3.5" />}
                      {tSlash("保存开关 / Save Switches")}
                    </button>
                  </div>
                </div>
                {hasUnsavedSkillSwitchChanges ? (
                  <p className="mt-3 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
                    {t("你有未保存的技能开关变更，请点击“保存开关”。", "You have unsaved skill switch changes. Click \"Save Switches\".")}
                  </p>
                ) : null}
                {skillsConfigError ? (
                  <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                    {tSlash("配置读取/保存失败 / Config read/save failed")}: {skillsConfigError}
                  </p>
                ) : null}
                {skillSwitchSaveMessage ? (
                  <p className="mt-3 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
                    {skillSwitchSaveMessage}
                  </p>
                ) : null}
                <p className="mt-3 rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-xs text-white/65">
                  {t(
                    "技能按图片、语音、基础能力与其它分组展示。按钮只是先选择；点击“保存开关”后会提示重启，确认后系统会自动帮你重启并生效。",
                    "Skills are grouped by image, audio, core capabilities, and others. Buttons only stage your choice; after Save Switches you will be prompted to restart.",
                  )}
                </p>

                {(() => {
                  const renderSkillGroup = (title: string, filteredList: string[]) => {
                    if (filteredList.length === 0) return null;
                    return (
                      <div key={title} className="space-y-2">
                        <h6 className="text-xs font-semibold uppercase tracking-wider text-white/60">{title}</h6>
                        <div className="grid gap-1.5 sm:grid-cols-2 xl:grid-cols-3">{filteredList.map(renderSkillRow)}</div>
                      </div>
                    );
                  };
                  const renderSkillRow = (name: string) => {
                    const runtimeEnabled = visibleRuntimeSkills.includes(name);
                    const configuredEnabled = configuredEnabledSkills.has(name);
                    const pendingApply = runtimeEnabled !== configuredEnabled;
                    const isRecentImport = recentImportedSkillName === name;
                    const isExternalSkill = externalSkillNamesSet.has(name);
                    const isUninstalling = skillUninstallingName === name;
                    const statusMeta = [
                      baseSkillNamesSet.has(name) ? t("系统基础能力", "Core capability") : null,
                      isExternalSkill ? t("外部导入", "Imported") : null,
                    ].filter(Boolean) as string[];
                    return (
                      <label
                        id={`skill-row-${name}`}
                        key={name}
                        className={
                          isRecentImport
                            ? "flex flex-col gap-2 rounded-lg border border-sky-400/40 bg-sky-500/10 px-2.5 py-2 text-xs shadow-[0_0_0_1px_rgba(56,189,248,0.18)] sm:flex-row sm:items-center sm:justify-between"
                            : "flex flex-col gap-2 rounded-lg border border-white/10 bg-[#12151f] px-2.5 py-2 text-xs sm:flex-row sm:items-center sm:justify-between"
                        }
                      >
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-sm text-white/90">{name}</span>
                          <span className="mt-0.5 block truncate text-[11px] leading-4 text-white/50">{describeSkill(name)}</span>
                          {statusMeta.length > 0 ? (
                            <span className="mt-1 block text-[10px] leading-4 text-white/35">{statusMeta.join(" · ")}</span>
                          ) : null}
                        </span>
                        <span className="mt-1 flex shrink-0 flex-wrap items-center gap-1.5 sm:mt-0">
                          <span
                            className={
                              configuredEnabled
                                ? "inline-flex items-center gap-1 rounded-full border border-emerald-500/35 bg-emerald-500/12 px-2 py-0.5 text-[10px] font-medium text-emerald-200"
                                : "inline-flex items-center gap-1 rounded-full border border-amber-500/35 bg-amber-500/12 px-2 py-0.5 text-[10px] font-medium text-amber-200"
                            }
                          >
                            <span
                              className={
                                configuredEnabled ? "h-1 w-1 rounded-full bg-emerald-300" : "h-1 w-1 rounded-full bg-amber-300"
                              }
                            />
                            {configuredEnabled ? t("已开启", "On") : t("已关闭", "Off")}
                          </span>
                          {pendingApply ? (
                            <span className="text-[10px] text-amber-200/85">
                              {t("保存后生效", "After save")}
                            </span>
                          ) : null}
                          <button
                            type="button"
                            onClick={() => toggleSkillEnabled(name, !configuredEnabled)}
                            className="rounded border border-white/20 bg-white/5 px-1.5 py-0.5 text-[10px] text-white/80 hover:bg-white/10"
                            title={configuredEnabled ? t("先设为关闭，保存后才会真正关闭", "Choose Disable first. It only turns off after you save.") : t("先设为开启，保存后才会真正开启", "Choose Enable first. It only turns on after you save.")}
                          >
                            {configuredEnabled ? t("关", "Off") : isRecentImport ? t("启用", "Enable") : t("开", "On")}
                          </button>
                          {isExternalSkill ? (
                            <button
                              type="button"
                              onClick={() => void uninstallExternalSkill(name)}
                              disabled={isUninstalling}
                              className="inline-flex items-center gap-1 rounded border border-red-500/25 bg-red-500/10 px-1.5 py-0.5 text-[10px] text-red-100 hover:bg-red-500/15 disabled:cursor-not-allowed disabled:opacity-50"
                              title={t("卸载这个外部技能", "Uninstall this imported skill")}
                            >
                              {isUninstalling ? <Loader2 className="h-3 w-3 animate-spin" /> : <Trash2 className="h-3 w-3" />}
                              {t("卸载", "Uninstall")}
                            </button>
                          ) : null}
                        </span>
                      </label>
                    );
                  };

                  return (
                    <div className="mt-4 space-y-4">
                      <div className="rounded-xl border border-white/10 bg-[#12151f] px-4 py-3">
                        <div className="flex items-center justify-between gap-3">
                          <h5 className="text-sm font-semibold text-white">{t("技能分组", "Skills by group")}</h5>
                          <span className="theme-meta-pill !rounded-xl !px-2.5 !py-1 text-[11px]">
                            {filteredManagedSkills.length}/{managedSkills.length}
                          </span>
                        </div>
                        <p className="mt-1 text-xs leading-5 text-white/50">
                          {t(
                            "图片、语音、基础能力与其它。新导入的技能会出现在对应分组。",
                            "Image, audio, core capabilities, and others. Newly imported skills appear in the matching group.",
                          )}
                        </p>
                        <label className="mt-3 block space-y-2">
                          <span className="text-[10px] uppercase tracking-widest text-white/45">
                            {t("按名称查找技能", "Find a skill by name")}
                          </span>
                          <input
                            className="theme-input"
                            value={skillsSearchQuery}
                            onChange={(e) => setSkillsSearchQuery(e.target.value)}
                            placeholder={t("例如 crypto、image、binance", "For example crypto, image, or binance")}
                          />
                        </label>
                      </div>
                      <div className="space-y-4">
                        {renderSkillGroup(t("图片技能", "Image skills"), filteredSkillsImage)}
                        {renderSkillGroup(t("语音技能", "Voice / Audio skills"), filteredSkillsAudio)}
                        {renderSkillGroup(t("不建议关闭的基础技能", "Core skills (not recommended to disable)"), filteredSkillsBase)}
                        {renderSkillGroup(t("其他", "Others"), filteredSkillsOther)}
                      </div>
                      {normalizedSkillsSearchQuery &&
                        filteredSkillsImage.length === 0 &&
                        filteredSkillsAudio.length === 0 &&
                        filteredSkillsBase.length === 0 &&
                        filteredSkillsOther.length === 0 ? (
                        <div className="rounded-xl border border-white/10 bg-white/5 px-4 py-3 text-sm text-white/60">
                          {t("没有找到匹配的技能。可以试试更短的关键词，比如 crypto、image、audio。", "No matching skills found. Try a shorter keyword like crypto, image, or audio.")}
                        </div>
                      ) : null}
                      {managedSkills.length === 0 ? (
                        <span className="text-xs text-white/50">{skillsConfigLoading ? tSlash("加载中... / Loading...") : "--"}</span>
                      ) : null}
                    </div>
                  );
                })()}
              </div>
            </section>
          ) : null}

          {currentPage === "logs" ? (
            <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
              <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
                <h3 className="text-base font-semibold">{t("最新日志", "Latest Logs")}</h3>
                <button
                  onClick={() => void fetchLatestLog()}
                  disabled={logLoading}
                  className="inline-flex items-center justify-center gap-2 rounded-xl bg-white/10 px-3 py-2 text-xs font-medium transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {logLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                  {tSlash("刷新 / Refresh")}
                </button>
              </div>

              <div className="mb-4 grid gap-3 md:grid-cols-4">
                <label className="space-y-2">
                  <span className="text-[10px] uppercase tracking-widest text-white/50">{t("日志文件", "Log File")}</span>
                  <select
                    className="theme-input"
                    value={selectedLogFile}
                    onChange={(e) => setSelectedLogFile(e.target.value)}
                  >
                    <option value="agent_trace.log">agent_trace.log</option>
                    <option value="model_io.log">model_io.log</option>
                    <option value="routing.log">routing.log</option>
                    <option value="act_plan.log">act_plan.log</option>
                    <option value="clawd.log">clawd.log</option>
                    <option value="telegramd.log">telegramd.log</option>
                    <option value="whatsappd.log">whatsappd.log</option>
                    <option value="whatsapp_webd.log">whatsapp_webd.log</option>
                  </select>
                </label>

                <label className="space-y-2">
                  <span className="text-[10px] uppercase tracking-widest text-white/50">{t("尾部行数", "Tail Lines")}</span>
                  <select
                    className="theme-input"
                    value={logTailLines}
                    onChange={(e) => setLogTailLines(Number(e.target.value))}
                  >
                    <option value={100}>100</option>
                    <option value={200}>200</option>
                    <option value={500}>500</option>
                    <option value={1000}>1000</option>
                  </select>
                </label>

                <div className="flex items-end">
                  <label className="inline-flex items-center gap-2 text-sm text-white/80">
                    <input type="checkbox" checked={logFollowTail} onChange={(e) => setLogFollowTail(e.target.checked)} />
                    {t("跟随到底部", "Follow tail")}
                  </label>
                </div>

                <div className="flex items-end text-xs text-white/50">
                  {logLastUpdated ? `${t("更新时间", "Updated")}: ${toLocalTime(logLastUpdated)}` : t("尚未加载", "Not loaded yet")}
                </div>
              </div>

              {logError ? (
                <p className="mb-4 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                  {t("日志读取失败", "Log read failed")}: {logError}
                </p>
              ) : null}

              <pre
                ref={logContainerRef}
                className="h-[70vh] overflow-auto rounded-xl border border-white/10 bg-[#12151f] p-3 text-xs text-white/85"
              >
                {logText || t("日志为空", "Log is empty")}
              </pre>
            </section>
          ) : null}

          {currentPage === "tasks" ? (
            <>
              <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
                <div className="flex flex-wrap gap-3">
                  <button type="button" onClick={() => setCurrentPage("chat")} className="theme-accent-soft-btn">
                    <MessageCircle className="h-4 w-4" />
                    {t("先去对话交互", "Open Chat")}
                  </button>
                  <button type="button" onClick={() => setCurrentPage("channels")} className="theme-accent-soft-btn">
                    <Database className="h-4 w-4" />
                    {t("先去绑定账号", "Open Bind Accounts")}
                  </button>
                </div>
              </section>

              <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
                <h3 className="mb-4 text-lg font-semibold">{t("手动提交一条任务", "Submit a task manually")}</h3>
                <div className="grid gap-4 md:grid-cols-2">
                  <label className="space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">{t("任务类型", "Task type")}</span>
                    <select
                      className="theme-input"
                      value={interactionKind}
                      onChange={(e) => setInteractionKind(e.target.value as "ask" | "run_skill")}
                    >
                      <option value="ask">ask</option>
                      <option value="run_skill">run_skill</option>
                    </select>
                  </label>
                  <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-2 text-sm">
                    <p className="text-white/80">{t("当前本地身份", "Current local identity")}</p>
                    <p className="mt-1 text-xs text-white/50">role={interactionRole}</p>
                    {localContextLoading ? <p className="mt-1 text-xs text-white/50">{tSlash("加载中... / Loading...")}</p> : null}
                    {localContextError ? <p className="mt-1 text-xs text-red-300">{tSlash("上下文错误 / Context error")}: {localContextError}</p> : null}
                  </div>
                </div>
                <div className="mt-4 grid gap-4 md:grid-cols-2">
                  <label className="space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">{t("发送渠道", "Channel")}</span>
                    <select
                      className="theme-input"
                      value={interactionChannel}
                      onChange={(e) => setInteractionChannel(e.target.value as ChannelName)}
                    >
                      <option value="ui">ui</option>
                      <option value="telegram">telegram</option>
                      <option value="whatsapp">whatsapp</option>
                      <option value="feishu">feishu</option>
                      <option value="lark">lark</option>
                    </select>
                  </label>
                  <label className="space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">{t("适配器名（可选）", "Adapter name (optional)")}</span>
                    <input
                      className="theme-input"
                      value={interactionAdapter}
                      onChange={(e) => setInteractionAdapter(e.target.value)}
                      placeholder="telegram_bot / whatsapp_cloud / whatsapp_web / feishu"
                    />
                  </label>
                  <label className="space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">{t("外部用户 ID（可选）", "External user ID (optional)")}</span>
                    <input
                      className="theme-input"
                      value={interactionExternalUserId}
                      onChange={(e) => setInteractionExternalUserId(e.target.value)}
                      placeholder={t("外部用户 ID（跨平台）", "External user id")}
                    />
                  </label>
                  <label className="space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">{t("外部会话 ID（可选）", "External chat ID (optional)")}</span>
                    <input
                      className="theme-input"
                      value={interactionExternalChatId}
                      onChange={(e) => setInteractionExternalChatId(e.target.value)}
                      placeholder={t("外部会话 ID（WhatsApp 建议填写）", "External chat id")}
                    />
                  </label>
                </div>

                {interactionKind === "ask" ? (
                  <div className="mt-4 space-y-4">
                    <label className="block space-y-2">
                      <span className="text-xs uppercase tracking-widest text-white/50">ask.text</span>
                      <textarea
                        className="theme-input min-h-28"
                        value={interactionAskText}
                        onChange={(e) => setInteractionAskText(e.target.value)}
                        placeholder={t("例如：请汇报当前系统状态", "For example: Please summarize the current system status")}
                      />
                    </label>
                    <label className="inline-flex items-center gap-2 text-sm text-white/80">
                      <input type="checkbox" checked={interactionAgentMode} onChange={(e) => setInteractionAgentMode(e.target.checked)} />
                      agent_mode
                    </label>
                  </div>
                ) : (
                  <div className="mt-4 space-y-4">
                    <label className="block space-y-2">
                      <span className="text-xs uppercase tracking-widest text-white/50">run_skill.skill_name</span>
                      <input
                        className="theme-input"
                        value={interactionSkillName}
                        onChange={(e) => setInteractionSkillName(e.target.value)}
                      />
                    </label>
                    <label className="block space-y-2">
                      <span className="text-xs uppercase tracking-widest text-white/50">{tSlash("run_skill.args (JSON 或字符串 / string)")}</span>
                      <textarea
                        className="theme-input min-h-28"
                        value={interactionSkillArgs}
                        onChange={(e) => setInteractionSkillArgs(e.target.value)}
                      />
                    </label>
                  </div>
                )}

                <div className="mt-4 flex flex-wrap items-center gap-3">
                  <button
                    onClick={() => void submitInteractionTask()}
                    disabled={interactionLoading}
                    className="theme-accent-btn"
                  >
                    {interactionLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                    {t("提交任务", "Submit task")}
                  </button>

                  {interactionSubmittedTaskId ? (
                    <span className="text-xs text-emerald-300">
                      {tSlash("已提交 / Submitted")}
                      {trackingTaskId ? ` ${tSlash("（自动跟踪中 / auto tracking）")}` : ""}
                    </span>
                  ) : null}
                </div>

                {interactionError ? (
                  <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                    {tSlash("提交失败 / Submit failed")}: {interactionError}
                  </p>
                ) : null}
              </section>

              <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
                <h3 className="mb-4 text-lg font-semibold">{t("按 task_id 查询结果", "Query a result by task_id")}</h3>
                <div className="grid gap-4 md:grid-cols-[1fr_auto]">
                  <input
                    className="theme-input"
                    placeholder="输入 task_id（UUID）/ Enter task_id"
                    value={taskId}
                    onChange={(e) => setTaskId(e.target.value)}
                  />
                  <button
                    onClick={() => void queryTask()}
                    disabled={taskLoading || !taskId.trim()}
                    className="inline-flex items-center justify-center gap-2 rounded-xl bg-white/10 px-4 py-2 text-sm font-medium transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {taskLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                    {tSlash("查询任务 / Query")}
                  </button>
                </div>

                {taskError ? (
                  <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                    {tSlash("查询失败 / Query failed")}: {taskError}
                  </p>
                ) : null}

                {taskResult ? (
                  <div className="mt-4 rounded-xl border border-white/10 bg-black/30 p-4 text-sm">
                    <p className="mb-1 text-white/60">{tSlash("任务 ID / Task ID")}</p>
                    <p className="font-mono text-white">{taskResult.task_id}</p>
                    <div className="mt-3 grid gap-3 md:grid-cols-2">
                      <div>
                        <p className="mb-1 text-white/60">{tSlash("状态 / Status")}</p>
                        <p className="theme-status-pill inline-block rounded-md px-2 py-1 font-mono">{taskResult.status}</p>
                      </div>
                      <div>
                        <p className="mb-1 text-white/60">{tSlash("错误信息 / Error")}</p>
                        <p className="text-red-200">{taskResult.error_text || "--"}</p>
                      </div>
                    </div>
                    <p className="mb-1 mt-4 text-white/60">{tSlash("结果 JSON / Result")}</p>
                    <pre className="max-h-72 overflow-auto rounded-lg border border-white/10 bg-[#12151f] p-3 text-xs text-white/80">
                      {JSON.stringify(taskResult.result_json ?? null, null, 2)}
                    </pre>
                  </div>
                ) : null}
              </section>
            </>
          ) : null}
        </main>
      </div>
    </div>
  );
}
