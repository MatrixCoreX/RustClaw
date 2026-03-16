import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
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
  channel_gateway_healthy?: boolean | null;
  channel_gateway_process_count?: number | null;
  channel_gateway_memory_rss_bytes?: number | null;
  whatsappd_healthy?: boolean | null;
  whatsappd_process_count?: number | null;
  whatsappd_memory_rss_bytes?: number | null;
  telegram_bot_healthy?: boolean | null;
  telegram_bot_process_count?: number | null;
  telegram_bot_memory_rss_bytes?: number | null;
  telegram_configured_bot_count?: number;
  telegram_configured_bot_names?: string[];
  telegram_bot_statuses?: TelegramBotRuntimeStatus[];
  gateway_instance_statuses?: GatewayInstanceRuntimeStatus[];
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

interface TelegramBotRuntimeStatus {
  name: string;
  healthy: boolean;
  status: string;
  last_heartbeat_ts?: number | null;
  last_error?: string | null;
}

interface GatewayInstanceRuntimeStatus {
  kind: string;
  name: string;
  scope: string;
  healthy: boolean;
  status: string;
  last_heartbeat_ts?: number | null;
  last_error?: string | null;
}

interface TaskQueryResponse {
  task_id: string;
  status: "queued" | "running" | "succeeded" | "failed" | "canceled" | "timeout";
  result_json?: unknown | null;
  error_text?: string | null;
}

interface DebugUsageSnapshot {
  prompt_tokens?: number | null;
  completion_tokens?: number | null;
  total_tokens?: number | null;
  input_tokens?: number | null;
  output_tokens?: number | null;
  reasoning_tokens?: number | null;
  cached_tokens?: number | null;
  cache_creation_input_tokens?: number | null;
  cache_read_input_tokens?: number | null;
}

interface TaskDebugEntry {
  ts?: number | null;
  task_id?: string | null;
  vendor?: string | null;
  provider?: string | null;
  provider_type?: string | null;
  model?: string | null;
  model_kind?: string | null;
  status?: string | null;
  prompt_file?: string | null;
  prompt?: string | null;
  request_payload?: unknown | null;
  response?: string | null;
  raw_response?: string | null;
  clean_response?: string | null;
  sanitized?: boolean | null;
  error?: string | null;
  usage?: DebugUsageSnapshot | null;
}

interface TaskDebugResponse {
  task_id: string;
  entries: TaskDebugEntry[];
}

interface UsageHistoryStats {
  total_requests: number;
  success_requests: number;
  failed_requests: number;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
}

interface UsageHistoryRecord {
  record_id: string;
  task_id: string;
  ts?: number | null;
  channel?: string | null;
  kind?: string | null;
  task_status?: string | null;
  telegram_bot_name?: string | null;
  external_user_id?: string | null;
  external_chat_id?: string | null;
  request_text?: string | null;
  vendor?: string | null;
  provider?: string | null;
  provider_type?: string | null;
  model?: string | null;
  model_kind?: string | null;
  prompt_file?: string | null;
  prompt_tokens?: number | null;
  completion_tokens?: number | null;
  total_tokens?: number | null;
  llm_call_count: number;
  status?: string | null;
  error?: string | null;
}

interface UsageHistoryChainEntry {
  ts?: number | null;
  vendor?: string | null;
  provider?: string | null;
  provider_type?: string | null;
  model?: string | null;
  model_kind?: string | null;
  status?: string | null;
  prompt_file?: string | null;
  prompt?: string | null;
  request_payload?: unknown | null;
  raw_response?: string | null;
  clean_response?: string | null;
  error?: string | null;
  prompt_tokens?: number | null;
  completion_tokens?: number | null;
  total_tokens?: number | null;
}

interface UsageHistoryRecordDetail extends UsageHistoryRecord {
  entries: UsageHistoryChainEntry[];
}

interface UsageHistoryResponse {
  stats: UsageHistoryStats;
  records: UsageHistoryRecord[];
  pagination: UsageHistoryPagination;
}

interface UsageHistoryPagination {
  page: number;
  page_size: number;
  total_records: number;
  total_pages: number;
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

interface TelegramBotConfigEntry {
  channel?: RobotChannelType;
  name: string;
  bot_token: string;
  agent_id?: string;
  admins?: number[];
  allowlist?: number[];
  access_mode?: "public" | "specified";
  allowed_telegram_usernames?: string[];
  is_primary?: boolean;
  role_name?: string;
  description?: string;
  persona_prompt?: string;
  preferred_vendor?: string | null;
  preferred_model?: string | null;
  allowed_skills?: string[];
}

interface AgentConfigEntry {
  id: string;
  name: string;
  description?: string;
  persona_prompt?: string;
  preferred_vendor?: string | null;
  preferred_model?: string | null;
  allowed_skills?: string[];
}

interface AgentLlmOption {
  vendor: string;
  label: string;
  models: string[];
  defaultModel: string;
}

interface TelegramConfigResponse {
  config_path: string;
  bots: TelegramBotConfigEntry[];
  agents: AgentConfigEntry[];
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
}

type BrowserFileWithPath = File & {
  webkitRelativePath?: string;
};

interface AdapterHealthRow {
  key: string;
  label: string;
  serviceName: "channel-gateway" | "telegramd" | "whatsappd" | "whatsapp_webd" | "feishud" | "larkd";
  healthy: boolean | null | undefined;
  processCount: number | null | undefined;
  memoryRssBytes: number | null | undefined;
}

interface ServiceStatusRow extends AdapterHealthRow {
  category: "ready" | "attention" | "stopped" | "unknown";
  statusLabel: string;
  detail: string;
}

type ChannelName = "telegram" | "whatsapp" | "ui" | "feishu" | "lark";
type RobotChannelType = "telegram" | "feishu" | "wechat";
type TelegramAccessMode = "public" | "specified";
type ConsolePage = "dashboard" | "services" | "channels" | "models" | "skills" | "chat" | "usage" | "logs" | "tasks";
type ThemeMode = "dark" | "light";
const CONSOLE_PAGES: ConsolePage[] = ["dashboard", "channels", "models", "skills", "chat", "usage", "logs", "tasks"];

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
  system_basic: { zh: "查看系统信息和基础环境。", en: "Inspect system information and environment basics." },
  write_file: { zh: "写入或修改文件内容。", en: "Write or update file contents." },
  x: { zh: "一个保留的扩展技能入口。", en: "A reserved extension skill entry point." },
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

function toLocalDateTime(ts: number): string {
  return new Date(ts).toLocaleString();
}

function formatInteger(value?: number | null, locale = "zh-CN"): string {
  if (value == null || Number.isNaN(value)) return "--";
  return new Intl.NumberFormat(locale).format(value);
}

function formatCompactInteger(value?: number | null, locale = "zh-CN"): string {
  if (value == null || Number.isNaN(value)) return "--";
  return new Intl.NumberFormat(locale, {
    notation: "compact",
    maximumFractionDigits: value >= 100000 ? 1 : 0,
  }).format(value);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
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

function parseNestedStructuredString(value: string): unknown {
  const trimmed = value.trim();
  if (!trimmed) return value;
  if (!["{", "["].includes(trimmed[0])) return value;
  try {
    return JSON.parse(trimmed);
  } catch {
    return value;
  }
}

function StructuredDebugValue({
  value,
  depth = 0,
}: {
  value: unknown;
  depth?: number;
}) {
  if (typeof value === "string") {
    const parsed = parseNestedStructuredString(value);
    if (parsed !== value) {
      return <StructuredDebugValue value={parsed} depth={depth} />;
    }
    return <div className="whitespace-pre-wrap break-words text-white/80">{value || "--"}</div>;
  }

  if (value == null) {
    return <span className="text-white/40">null</span>;
  }

  if (typeof value === "number" || typeof value === "boolean") {
    return <span className="text-sky-200">{String(value)}</span>;
  }

  if (Array.isArray(value)) {
    if (value.length === 0) {
      return <span className="text-white/40">[]</span>;
    }
    return (
      <div className="space-y-2">
        {value.map((item, index) => (
          <div key={`${depth}-${index}`} className="flex items-start gap-3 rounded-xl border border-white/6 bg-white/[0.03] px-3 py-2">
            <span className="mt-0.5 min-w-5 text-[11px] font-medium text-white/35">{index}</span>
            <div className="min-w-0 flex-1">
              <StructuredDebugValue value={item} depth={depth + 1} />
            </div>
          </div>
        ))}
      </div>
    );
  }

  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>);
    if (entries.length === 0) {
      return <span className="text-white/40">{"{}"}</span>;
    }
    return (
      <div className="space-y-2">
        {entries.map(([key, entryValue]) => (
          <div key={`${depth}-${key}`} className="rounded-xl border border-white/6 bg-white/[0.03] px-3 py-2">
            <div className="text-[11px] uppercase tracking-widest text-white/38">{key}</div>
            <div className="mt-1 min-w-0 text-xs leading-6">
              <StructuredDebugValue value={entryValue} depth={depth + 1} />
            </div>
          </div>
        ))}
      </div>
    );
  }

  return <div className="whitespace-pre-wrap break-words text-white/80">{String(value)}</div>;
}

function DebugPayloadPanel({
  title,
  value,
  defaultOpen = false,
  formatLabel,
  rawLabel,
}: {
  title: string;
  value: unknown;
  defaultOpen?: boolean;
  formatLabel: string;
  rawLabel: string;
}) {
  const [formatted, setFormatted] = useState(false);

  return (
    <details className="rounded-xl border border-white/10 bg-black/20 p-3" open={defaultOpen}>
      <summary className="cursor-pointer text-sm font-medium text-white/85">{title}</summary>
      <div className="mt-3 flex justify-end">
        <button
          type="button"
          onClick={() => setFormatted((current) => !current)}
          className="rounded-full border border-white/10 bg-white/6 px-3 py-1 text-[11px] font-medium text-white/72 transition hover:bg-white/10"
        >
          {formatted ? rawLabel : formatLabel}
        </button>
      </div>
      {formatted ? (
        <div className="theme-scrollbar mt-3 max-h-72 overflow-auto rounded-xl border border-white/8 bg-white/[0.03] p-3 text-xs leading-6">
          <StructuredDebugValue value={value ?? null} />
        </div>
      ) : (
        <pre className="theme-scrollbar mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-words text-xs leading-6 text-white/75">
          {JSON.stringify(value ?? null, null, 2)}
        </pre>
      )}
    </details>
  );
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
  const [telegramConfigLoading, setTelegramConfigLoading] = useState(false);
  const [telegramConfigError, setTelegramConfigError] = useState<string | null>(null);
  const [telegramConfigData, setTelegramConfigData] = useState<TelegramConfigResponse | null>(null);
  const [telegramConfigDrafts, setTelegramConfigDrafts] = useState<TelegramBotConfigEntry[]>([]);
  const [telegramConfigSaving, setTelegramConfigSaving] = useState(false);
  const [telegramConfigSaveMessage, setTelegramConfigSaveMessage] = useState<string | null>(null);
  const [telegramRestartNoticeVisible, setTelegramRestartNoticeVisible] = useState(false);
  const [botEditorOpen, setBotEditorOpen] = useState(false);
  const [botEditorIndex, setBotEditorIndex] = useState<number | null>(null);
  const [botEditorDraft, setBotEditorDraft] = useState<TelegramBotConfigEntry | null>(null);
  const [botEditorUsernameInput, setBotEditorUsernameInput] = useState("");
  const [llmDraftVendor, setLlmDraftVendor] = useState("");
  const [llmDraftModel, setLlmDraftModel] = useState("");
  const [llmConfigSaving, setLlmConfigSaving] = useState(false);
  const [llmConfigSaveMessage, setLlmConfigSaveMessage] = useState<string | null>(null);
  const [llmDraftBaseUrl, setLlmDraftBaseUrl] = useState("");
  const [llmDraftApiKey, setLlmDraftApiKey] = useState("");
  const [systemRestarting, setSystemRestarting] = useState(false);
  const [systemRestartMessage, setSystemRestartMessage] = useState<string | null>(null);

  const [taskResult, setTaskResult] = useState<TaskQueryResponse | null>(null);
  const [trackingTaskId, setTrackingTaskId] = useState<string | null>(null);
  const [debugModeEnabled, setDebugModeEnabled] = useState(false);
  const [taskDebugLoading, setTaskDebugLoading] = useState(false);
  const [taskDebugError, setTaskDebugError] = useState<string | null>(null);
  const [taskDebugData, setTaskDebugData] = useState<TaskDebugResponse | null>(null);
  const [usageRecordsLoading, setUsageRecordsLoading] = useState(false);
  const [usageRecordsError, setUsageRecordsError] = useState<string | null>(null);
  const [usageRecordsData, setUsageRecordsData] = useState<UsageHistoryResponse | null>(null);
  const [usageSearchQuery, setUsageSearchQuery] = useState("");
  const [usageChannelFilter, setUsageChannelFilter] = useState<string>("all");
  const [usageStatusFilter, setUsageStatusFilter] = useState<string>("all");
  const [usagePage, setUsagePage] = useState(1);
  const [selectedUsageRecordId, setSelectedUsageRecordId] = useState<string | null>(null);
  const [selectedUsageRecordDetail, setSelectedUsageRecordDetail] = useState<UsageHistoryRecordDetail | null>(null);
  const [selectedUsageRecordLoading, setSelectedUsageRecordLoading] = useState(false);
  const [selectedUsageRecordError, setSelectedUsageRecordError] = useState<string | null>(null);

  const [interactionKind, setInteractionKind] = useState<"ask" | "run_skill">("ask");
  const [interactionChannel, setInteractionChannel] = useState<ChannelName>("ui");
  const [interactionExternalUserId, setInteractionExternalUserId] = useState("");
  const [interactionExternalChatId, setInteractionExternalChatId] = useState("");
  const [interactionAdapter, setInteractionAdapter] = useState("");
  const [interactionTelegramBotName, setInteractionTelegramBotName] = useState("");
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
  const [chatAgentMode, setChatAgentMode] = useState(true);
  const [chatSending, setChatSending] = useState(false);
  const [chatError, setChatError] = useState<string | null>(null);
  const [serviceActionLoading, setServiceActionLoading] = useState<Record<string, boolean>>({});
  const [serviceActionMessage, setServiceActionMessage] = useState<string | null>(null);
  const [waLoginDialogOpen, setWaLoginDialogOpen] = useState(false);
  const [waLoginLoading, setWaLoginLoading] = useState(false);
  const [waLoginError, setWaLoginError] = useState<string | null>(null);
  const [waLoginStatus, setWaLoginStatus] = useState<WhatsappWebLoginStatus | null>(null);
  const [waLogoutLoading, setWaLogoutLoading] = useState(false);
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
  const logContainerRef = useRef<HTMLPreElement | null>(null);

  const t = (zh: string, en: string) => (lang === "zh" ? zh : en);
  const tSlash = (mixed: string) => {
    const [zh, en] = mixed.split(" / ");
    return lang === "zh" ? zh : en ?? zh;
  };
  const generateAgentId = () => {
    const randomPart =
      typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
        ? crypto.randomUUID().replace(/-/g, "").slice(0, 10)
        : `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
    return `agent-${randomPart}`;
  };
  const deriveStableAgentId = (seed: string, index: number) => {
    const normalizedSeed = seed
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "");
    return normalizedSeed ? `agent-${normalizedSeed}` : `agent-${index + 1}`;
  };
  const formatIdList = (values?: number[]) => (values ?? []).join(", ");
  const parseIdList = (raw: string) =>
    raw
      .split(",")
      .map((part) => part.trim())
      .filter(Boolean)
      .map((part) => Number(part))
      .filter((value) => Number.isInteger(value));
  const formatStringList = (values?: string[]) => (values ?? []).join(", ");
  const parseStringList = (raw: string) =>
    raw
      .split(",")
      .map((part) => part.trim())
      .filter(Boolean);
  const normalizeTelegramUsername = (value: string) => value.trim().replace(/^@+/, "").trim().toLowerCase();
  const normalizeTelegramUsernameList = (values?: string[]) =>
    [...new Set((values ?? []).map(normalizeTelegramUsername).filter(Boolean))];
  const telegramAccessModeLabel = (mode?: TelegramAccessMode) =>
    (mode || "public") === "specified"
      ? t("指定人员", "Specified people")
      : t("公开", "Public");
  const isSystemDefaultAgentName = (name?: string | null) => {
    const normalized = (name || "").trim().toLowerCase();
    return normalized === "" || normalized === "main" || normalized === "default assistant";
  };
  const localizedDefaultAgentName = () => t("默认助手", "Default assistant");
  const mergeTelegramBotsWithAgents = (bots: TelegramBotConfigEntry[], agents: AgentConfigEntry[]) => {
    const normalizedAgents = agents.map((agent) => ({
      ...agent,
      id: (agent.id || "").trim() || generateAgentId(),
      name: (agent.id || "").trim() === "main" && isSystemDefaultAgentName(agent.name) ? "" : agent.name,
    }));
    const agentMap = new Map(normalizedAgents.map((agent) => [agent.id, agent]));
    const seenAgentIds = new Set<string>();
    return bots.map((bot, index) => {
      const isPrimary = index === 0 || bot.is_primary === true;
      let nextAgentId = (bot.agent_id || "").trim();
      if (isPrimary) {
        nextAgentId = "main";
      } else if (!nextAgentId || seenAgentIds.has(nextAgentId)) {
        nextAgentId = deriveStableAgentId(bot.name || "", index);
        while (seenAgentIds.has(nextAgentId)) {
          nextAgentId = generateAgentId();
        }
      }
      seenAgentIds.add(nextAgentId);
      const agent = agentMap.get(nextAgentId);
      return {
        ...bot,
        channel: "telegram" as RobotChannelType,
        name: isPrimary ? "primary" : bot.name,
        agent_id: nextAgentId,
        is_primary: isPrimary,
        access_mode: bot.access_mode || "public",
        allowed_telegram_usernames: normalizeTelegramUsernameList(bot.allowed_telegram_usernames),
        role_name: agent?.name || "",
        description: agent?.description || "",
        persona_prompt: agent?.persona_prompt || "",
        preferred_vendor: agent?.preferred_vendor || "",
        preferred_model: agent?.preferred_model || "",
        allowed_skills: agent?.allowed_skills ?? [],
      };
    });
  };
  const buildAgentPayloadFromBots = (bots: TelegramBotConfigEntry[]) => {
    const normalizedBots = bots.map((bot, index) => {
      const isPrimary = index === 0 || bot.is_primary === true;
      const fallbackAgentId = deriveStableAgentId(bot.name || "", index);
      const agentId = isPrimary ? "main" : ((bot.agent_id || "").trim() || fallbackAgentId);
      return {
        ...bot,
        channel: "telegram" as RobotChannelType,
        name: isPrimary ? "primary" : bot.name.trim(),
        agent_id: agentId,
        is_primary: isPrimary,
      };
    });
    const agents = normalizedBots.map((bot, index) => ({
      id: index === 0 ? "main" : ((bot.agent_id || "").trim() || generateAgentId()),
      name: bot.role_name?.trim() || (index === 0 ? "" : bot.name.trim()),
      description: bot.description?.trim() || "",
      persona_prompt: bot.persona_prompt?.trim() || "",
      preferred_vendor: bot.preferred_vendor?.trim() || undefined,
      preferred_model: bot.preferred_model?.trim() || undefined,
      allowed_skills: bot.allowed_skills ?? [],
    }));
    return {
      bots: normalizedBots.map((bot) => ({
        name: bot.name,
        bot_token: bot.bot_token.trim(),
        agent_id: (bot.agent_id || "main").trim() || "main",
        admins: bot.admins ?? [],
        allowlist: bot.allowlist ?? [],
        access_mode: (bot.access_mode || "public") as TelegramAccessMode,
        allowed_telegram_usernames: normalizeTelegramUsernameList(bot.allowed_telegram_usernames),
        is_primary: bot.is_primary,
      })),
      agents,
    };
  };
  const createEmptyTelegramBotDraft = (): TelegramBotConfigEntry => ({
    channel: "telegram",
    name: "",
    bot_token: "",
    agent_id: generateAgentId(),
    admins: [],
    allowlist: [],
    access_mode: "public",
    allowed_telegram_usernames: [],
    is_primary: false,
    role_name: "",
    description: "",
    persona_prompt: "",
    preferred_vendor: "",
    preferred_model: "",
    allowed_skills: [],
  });
  const robotChannelLabel = (channel?: RobotChannelType) => {
    const labels: Record<RobotChannelType, string> = {
      telegram: "Telegram",
      feishu: "Feishu",
      wechat: t("企业微信", "WeCom"),
    };
    return labels[channel || "telegram"];
  };
  const robotChannelSaveSupported = (channel?: RobotChannelType) => (channel || "telegram") === "telegram";
  const robotDisplayName = (bot?: Pick<TelegramBotConfigEntry, "name" | "role_name"> | null) =>
    (bot?.role_name || "").trim() || (bot?.name || "").trim();
  const serviceDisplayName = (key: AdapterHealthRow["key"]) => {
    const labels: Record<AdapterHealthRow["key"], string> = {
      telegram_bot: t("Telegram 服务", "Telegram Service"),
      whatsapp_web: t("WhatsApp 网页版", "WhatsApp Web"),
      whatsapp_cloud: t("WhatsApp 云接口", "WhatsApp Cloud"),
      feishu_bot: t("飞书", "Feishu"),
      lark_bot: t("Lark", "Lark"),
    };
    return labels[key];
  };
  const telegramBotMonogram = (name: string) =>
    name
      .split(/[^a-zA-Z0-9]+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((part) => part[0]?.toUpperCase() ?? "")
      .join("") || "TG";
  const isOnline = Boolean(health) && !error;
  const queuePressureHigh = (health?.queue_length ?? 0) >= queueWarn;
  const runningTooOld = (health?.running_oldest_age_seconds ?? 0) >= ageWarnSeconds;
  const authHeaders = uiKey ? { "X-RustClaw-Key": uiKey } : {};
  const apiFetch = (path: string, init?: RequestInit) =>
    fetch(`${apiBase.replace(/\/$/, "")}${path}`, {
      ...init,
      headers: {
        ...(init?.headers ?? {}),
        ...authHeaders,
      },
    });

  const applyIdentity = (identity: AuthIdentityResponse) => {
    setInteractionUserId(identity.user_id);
    setInteractionChatId(identity.chat_id);
    setInteractionRole(identity.role);
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
      let identity: AuthIdentityResponse | null = null;
      let verifyError: string | null = null;

      try {
        const res = await fetch(`${apiBase.replace(/\/$/, "")}/v1/auth/ui-key/verify`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ user_key: normalized }),
        });
        const body = (await res.json()) as ApiResponse<AuthIdentityResponse>;
        if (res.ok && body.ok && body.data) {
          identity = body.data;
        } else {
          verifyError = body.error || `key 校验失败 (${res.status})`;
        }
      } catch (err) {
        verifyError = err instanceof Error ? err.message : "未知错误";
      }

      if (!identity) {
        const fallbackRes = await fetch(`${apiBase.replace(/\/$/, "")}/v1/auth/me`, {
          headers: { "X-RustClaw-Key": normalized },
        });
        const fallbackBody = (await fallbackRes.json()) as ApiResponse<AuthIdentityResponse>;
        if (!fallbackRes.ok || !fallbackBody.ok || !fallbackBody.data) {
          throw new Error(fallbackBody.error || verifyError || `key 校验失败 (${fallbackRes.status})`);
        }
        identity = fallbackBody.data;
      }

      setUiKey(normalized);
      setUiKeyDraft(normalized);
      setUiAuthReady(true);
      applyIdentity(identity);
      if (persist) {
        window.localStorage.setItem(STORAGE_KEYS.userKey, normalized);
      }
      return true;
    } catch (err) {
      setUiAuthReady(false);
      setUiKey("");
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
    serviceName: "channel-gateway" | "telegramd" | "whatsappd" | "whatsapp_webd" | "feishud" | "larkd",
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
      if (!silent) {
        setWaLoginError(null);
      }
    } catch (err) {
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
      setLlmDraftApiKey("");
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setLlmConfigError(message);
    } finally {
      setLlmConfigLoading(false);
    }
  };

  const fetchTelegramConfig = async () => {
    setTelegramConfigLoading(true);
    setTelegramConfigError(null);
    try {
      const res = await apiFetch(`/v1/telegram/config`);
      const body = (await res.json()) as ApiResponse<TelegramConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `Telegram 配置获取失败 (${res.status})`);
      }
      setTelegramConfigData(body.data);
      setTelegramConfigDrafts(mergeTelegramBotsWithAgents(body.data.bots ?? [], body.data.agents ?? []));
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setTelegramConfigError(message);
    } finally {
      setTelegramConfigLoading(false);
    }
  };

  const updateTelegramBotDraft = (
    index: number,
    key:
      | "name"
      | "bot_token"
      | "admins"
      | "allowlist"
      | "access_mode"
      | "allowed_telegram_usernames"
      | "role_name"
      | "description"
      | "persona_prompt"
      | "preferred_vendor"
      | "preferred_model"
      | "allowed_skills",
    value: string,
  ) => {
    setTelegramConfigDrafts((prev) =>
      prev.map((bot, currentIndex) => {
        if (currentIndex !== index) return bot;
        if (key === "admins" || key === "allowlist") {
          return { ...bot, [key]: parseIdList(value) };
        }
        if (key === "allowed_telegram_usernames") {
          return { ...bot, allowed_telegram_usernames: normalizeTelegramUsernameList(parseStringList(value)) };
        }
        if (key === "allowed_skills") {
          return { ...bot, allowed_skills: parseStringList(value) };
        }
        if (key === "preferred_vendor") {
          const nextVendor = value.trim();
          const vendorInfo = agentLlmOptions.find((item) => item.vendor === nextVendor);
          return {
            ...bot,
            preferred_vendor: nextVendor,
            preferred_model: nextVendor ? vendorInfo?.defaultModel || vendorInfo?.models?.[0] || "" : "",
          };
        }
        return { ...bot, [key]: value };
      }),
    );
  };

  const updateBotEditorDraft = (
    key:
      | "channel"
      | "name"
      | "bot_token"
      | "admins"
      | "allowlist"
      | "access_mode"
      | "allowed_telegram_usernames"
      | "role_name"
      | "description"
      | "persona_prompt"
      | "preferred_vendor"
      | "preferred_model"
      | "allowed_skills",
    value: string,
  ) => {
    setBotEditorDraft((prev) => {
      if (!prev) return prev;
      if (key === "name") {
        return { ...prev, name: value, role_name: value };
      }
      if (key === "admins" || key === "allowlist") {
        return { ...prev, [key]: parseIdList(value) };
      }
      if (key === "allowed_telegram_usernames") {
        return { ...prev, allowed_telegram_usernames: normalizeTelegramUsernameList(parseStringList(value)) };
      }
      if (key === "allowed_skills") {
        return { ...prev, allowed_skills: parseStringList(value) };
      }
      if (key === "preferred_vendor") {
        const nextVendor = value.trim();
        const vendorInfo = agentLlmOptions.find((item) => item.vendor === nextVendor);
        return {
          ...prev,
          preferred_vendor: nextVendor,
          preferred_model: nextVendor ? vendorInfo?.defaultModel || vendorInfo?.models?.[0] || "" : "",
        };
      }
      return { ...prev, [key]: value };
    });
  };

  const openAddTelegramBotEditor = () => {
    setBotEditorIndex(null);
    setBotEditorDraft(createEmptyTelegramBotDraft());
    setBotEditorUsernameInput("");
    setBotEditorOpen(true);
    setTelegramConfigSaveMessage(null);
  };

  const applyRobotPersonaPreset = (preset: (typeof robotPersonaPresets)[number]) => {
    setBotEditorDraft((prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        description: preset.description,
        persona_prompt: preset.personaPrompt,
      };
    });
  };

  const applyRobotSkillMode = (mode: "inherit" | "common" | "custom") => {
    setBotEditorDraft((prev) => {
      if (!prev) return prev;
      if (mode === "inherit") {
        return { ...prev, allowed_skills: [] };
      }
      if (mode === "common") {
        return { ...prev, allowed_skills: beginnerRobotSkillPreset };
      }
      return { ...prev, allowed_skills: managedSkills };
    });
  };

  const toggleBotEditorSkill = (skill: string, enabled: boolean) => {
    setBotEditorDraft((prev) => {
      if (!prev) return prev;
      const current = new Set(prev.allowed_skills ?? []);
      if (enabled) {
        current.add(skill);
      } else {
        current.delete(skill);
      }
      return {
        ...prev,
        allowed_skills: managedSkills.filter((name) => current.has(name)),
      };
    });
  };

  const openEditTelegramBotEditor = (index: number) => {
    setBotEditorIndex(index);
    setBotEditorDraft({ ...telegramConfigDrafts[index] });
    setBotEditorUsernameInput("");
    setBotEditorOpen(true);
    setTelegramConfigSaveMessage(null);
  };

  const closeBotEditor = () => {
    setBotEditorOpen(false);
    setBotEditorIndex(null);
    setBotEditorDraft(null);
    setBotEditorUsernameInput("");
  };

  const addBotEditorTelegramUsername = (raw: string) => {
    const normalized = normalizeTelegramUsername(raw);
    if (!normalized) return;
    setBotEditorDraft((prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        allowed_telegram_usernames: normalizeTelegramUsernameList([
          ...(prev.allowed_telegram_usernames ?? []),
          normalized,
        ]),
      };
    });
    setBotEditorUsernameInput("");
  };

  const removeBotEditorTelegramUsername = (username: string) => {
    const normalized = normalizeTelegramUsername(username);
    if (!normalized) return;
    setBotEditorDraft((prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        allowed_telegram_usernames: (prev.allowed_telegram_usernames ?? []).filter((item) => item !== normalized),
      };
    });
  };

  const saveBotEditorDraft = async () => {
    if (!botEditorDraft) return;
    if (!robotChannelSaveSupported(botEditorDraft.channel)) {
      setTelegramConfigError(
        t(
          "飞书和企业微信机器人的后端配置接口还在接，这个版本先只能直接保存 Telegram 机器人。",
          "Feishu and WeCom backend config APIs are still being wired in. This version can only save Telegram robots directly.",
        ),
      );
      return;
    }
    const nextDrafts =
      botEditorIndex == null
        ? [...telegramConfigDrafts, botEditorDraft]
        : telegramConfigDrafts.map((bot, index) => (index === botEditorIndex ? { ...botEditorDraft, is_primary: bot.is_primary } : bot));
    setTelegramConfigDrafts(nextDrafts);
    closeBotEditor();
    await saveTelegramConfig(nextDrafts);
  };

  const removeTelegramBotDraft = async (index: number) => {
    const nextDrafts = telegramConfigDrafts.filter((_, currentIndex) => currentIndex !== index);
    setTelegramConfigDrafts(nextDrafts);
    setTelegramConfigSaveMessage(null);
    await saveTelegramConfig(nextDrafts);
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
          vendor_api_key: llmDraftApiKey.trim() || undefined,
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

  const saveTelegramConfig = async (draftsOverride?: TelegramBotConfigEntry[]) => {
    setTelegramConfigSaving(true);
    setTelegramConfigSaveMessage(null);
    setTelegramConfigError(null);
    setSystemRestartMessage(null);
    try {
      const { bots, agents } = buildAgentPayloadFromBots(draftsOverride ?? telegramConfigDrafts);
      const res = await apiFetch(`/v1/telegram/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ bots, agents }),
      });
      const body = (await res.json()) as ApiResponse<TelegramConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `Telegram 配置保存失败 (${res.status})`);
      }
      setTelegramConfigData(body.data);
      setTelegramConfigDrafts(mergeTelegramBotsWithAgents(body.data.bots ?? [], body.data.agents ?? []));
      const savedMessage = t(
        "Telegram 机器人名册已保存到 configs/channels/telegram.toml。",
        "The Telegram robot roster was saved to configs/channels/telegram.toml.",
      );
      if (body.data.restart_required) {
        setTelegramRestartNoticeVisible(true);
        setTelegramConfigSaveMessage(
          t(
            "机器人设置已保存。等你把其他机器人也调整完，再统一重启 RustClaw 生效。",
            "Robot settings were saved. Finish the other robot changes first, then restart RustClaw once to apply them all.",
          ),
        );
        await fetchHealth();
        return;
      }
      setTelegramRestartNoticeVisible(false);
      setTelegramConfigSaveMessage(savedMessage);
      await fetchHealth();
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setTelegramConfigError(message);
    } finally {
      setTelegramConfigSaving(false);
    }
  };

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
        await Promise.allSettled([fetchLlmConfig(), fetchSkillsConfig(), fetchSkills(), fetchTelegramConfig()]);
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

  const fetchTaskDebugById = async (id: string): Promise<TaskDebugResponse> => {
    const normalizedId = id.trim();
    try {
      const res = await apiFetch(`/v1/debug/tasks/${normalizedId}`);
      const body = (await res.json()) as ApiResponse<TaskDebugResponse>;
      if (res.ok && body.ok && body.data) {
        return body.data;
      }
    } catch {
      // Fallback to parsing model_io.log for older backend builds.
    }
    const params = new URLSearchParams({
      file: "model_io.log",
      lines: "2000",
    });
    const res = await apiFetch(`/v1/logs/latest?${params.toString()}`);
    const body = (await res.json()) as ApiResponse<LogLatestResponse>;
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(body.error || `调试信息读取失败 (${res.status})`);
    }
    const entries = (body.data.text || "")
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean)
      .flatMap((line) => {
        try {
          return [JSON.parse(line) as TaskDebugEntry];
        } catch {
          return [];
        }
      })
      .filter((entry) => (entry.task_id || "").trim() === normalizedId)
      .sort((a, b) => (a.ts ?? 0) - (b.ts ?? 0));
    return {
      task_id: normalizedId,
      entries,
    };
  };

  const fetchUsageRecords = async () => {
    setUsageRecordsLoading(true);
    setUsageRecordsError(null);
    try {
      const params = new URLSearchParams({
        page: String(usagePage),
        page_size: "20",
      });
      if (usageSearchQuery.trim()) {
        params.set("search", usageSearchQuery.trim());
      }
      if (usageChannelFilter !== "all") {
        params.set("channel", usageChannelFilter);
      }
      if (usageStatusFilter !== "all") {
        params.set("status", usageStatusFilter);
      }
      const res = await apiFetch(`/v1/debug/usage-records?${params.toString()}`);
      const raw = await res.text();
      if (res.status === 404) {
        throw new Error(
          t(
            "使用记录页需要升级后的后端版本。",
            "The usage history page needs the newer backend build.",
          ),
        );
      }
      const body = raw ? (JSON.parse(raw) as ApiResponse<UsageHistoryResponse>) : null;
      if (!res.ok || !body?.ok || !body.data) {
        throw new Error(body?.error || `使用记录读取失败 (${res.status})`);
      }
      setUsageRecordsData(body.data);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setUsageRecordsError(message);
      setUsageRecordsData(null);
    } finally {
      setUsageRecordsLoading(false);
    }
  };

  const submitInteractionTask = async () => {
    setInteractionLoading(true);
    setInteractionError(null);
    setInteractionSubmittedTaskId(null);
    try {
      const telegramBotName = interactionTelegramBotName.trim();
      if (interactionChannel === "telegram" && !telegramBotName) {
        throw new Error(t("严格模式下必须先选择 Telegram 机器人。", "Strict mode requires selecting a Telegram robot first."));
      }
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
      if (interactionChannel === "telegram" && telegramBotName) {
        payload.telegram_bot_name = telegramBotName;
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
      setTrackingTaskId(resp.data.task_id);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setInteractionError(message);
    } finally {
      setInteractionLoading(false);
    }
  };

  const sendChatMessage = async () => {
    const text = chatInput.trim();
    if (!text || chatSending) return;
    setChatSending(true);
    setChatError(null);
    const userMsg: ChatMessage = {
      id: `u-${Date.now()}`,
      role: "user",
      text,
      ts: Date.now(),
    };
    setChatMessages((prev) => [...prev, userMsg]);
    setChatInput("");

    try {
      const chatPayload: Record<string, unknown> = {
        text,
        agent_mode: chatAgentMode,
      };
      const telegramBotName = interactionTelegramBotName.trim();
      if (interactionChannel === "telegram" && !telegramBotName) {
        throw new Error(t("严格模式下必须先选择 Telegram 机器人。", "Strict mode requires selecting a Telegram robot first."));
      }
      const adapterName = interactionAdapter.trim();
      if (adapterName) {
        chatPayload.adapter = adapterName;
      }
      if (interactionChannel === "telegram" && telegramBotName) {
        chatPayload.telegram_bot_name = telegramBotName;
      }
      const submitBody = {
        user_key: uiKey,
        channel: interactionChannel,
        ...(interactionExternalUserId.trim() ? { external_user_id: interactionExternalUserId.trim() } : {}),
        ...(interactionExternalChatId.trim() ? { external_chat_id: interactionExternalChatId.trim() } : {}),
        kind: "ask" as const,
        payload: chatPayload,
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
        text: extractTaskText(finalResult),
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
      setInteractionUserId(null);
      setInteractionChatId(null);
      setInteractionRole("-");
      return;
    }
    void verifyUiKey(uiKey, false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase]);

  useEffect(() => {
    if (!debugModeEnabled) {
      setTaskDebugLoading(false);
      setTaskDebugError(null);
      setTaskDebugData(null);
      return;
    }
    const currentTaskId = taskResult?.task_id?.trim();
    if (!uiAuthReady || !currentTaskId) {
      setTaskDebugLoading(false);
      setTaskDebugError(null);
      setTaskDebugData(null);
      return;
    }
    let cancelled = false;
    setTaskDebugLoading(true);
    setTaskDebugError(null);
    void fetchTaskDebugById(currentTaskId)
      .then((data) => {
        if (cancelled) return;
        setTaskDebugData(data);
      })
      .catch((err) => {
        if (cancelled) return;
        const message = err instanceof Error ? err.message : "未知错误";
        setTaskDebugError(message);
        setTaskDebugData(null);
      })
      .finally(() => {
        if (!cancelled) {
          setTaskDebugLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [apiBase, debugModeEnabled, taskResult?.task_id, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady || currentPage !== "usage") return;
    void fetchUsageRecords();
  }, [apiBase, currentPage, uiAuthReady, usagePage, usageSearchQuery, usageChannelFilter, usageStatusFilter]);

  useEffect(() => {
    const safePage = usageRecordsData?.pagination?.page;
    if (!safePage || safePage === usagePage) return;
    setUsagePage(safePage);
  }, [usagePage, usageRecordsData?.pagination?.page]);

  useEffect(() => {
    if (!selectedUsageRecordId) return;
    const allUsageRecords = usageRecordsData?.records ?? [];
    if (allUsageRecords.some((record) => record.record_id === selectedUsageRecordId)) return;
    setSelectedUsageRecordDetail(null);
    setSelectedUsageRecordError(null);
    setSelectedUsageRecordLoading(false);
    setSelectedUsageRecordId(null);
  }, [selectedUsageRecordId, usageRecordsData]);

  useEffect(() => {
    if (!selectedUsageRecordId) {
      setSelectedUsageRecordDetail(null);
      setSelectedUsageRecordError(null);
      setSelectedUsageRecordLoading(false);
      return;
    }
    let cancelled = false;
    const loadDetail = async () => {
      setSelectedUsageRecordLoading(true);
      setSelectedUsageRecordError(null);
      try {
        const res = await apiFetch(`/v1/debug/usage-records/${encodeURIComponent(selectedUsageRecordId)}`);
        const raw = await res.text();
        const body = raw ? (JSON.parse(raw) as ApiResponse<UsageHistoryRecordDetail>) : null;
        if (!res.ok || !body?.ok || !body.data) {
          throw new Error(body?.error || `使用记录详情读取失败 (${res.status})`);
        }
        if (!cancelled) {
          setSelectedUsageRecordDetail(body.data);
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : "未知错误";
        if (!cancelled) {
          setSelectedUsageRecordDetail(null);
          setSelectedUsageRecordError(message);
        }
      } finally {
        if (!cancelled) {
          setSelectedUsageRecordLoading(false);
        }
      }
    };
    void loadDetail();
    return () => {
      cancelled = true;
    };
  }, [selectedUsageRecordId]);

  useEffect(() => {
    if (!selectedUsageRecordId) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setSelectedUsageRecordId(null);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [selectedUsageRecordId]);

  useEffect(() => {
    if (!selectedUsageRecordId) return;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, [selectedUsageRecordId]);

  useEffect(() => {
    if (!uiAuthReady || pollingSeconds <= 0) return;
    void fetchHealth();
    void fetchSkills();
    void fetchSkillsConfig();
    void fetchLlmConfig();
    void fetchTelegramConfig();
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

  useEffect(() => {
    if (!uiAuthReady) return;
    void fetchSkills();
    void fetchSkillsConfig();
    void fetchLlmConfig();
    void fetchTelegramConfig();
    void fetchLocalInteractionContext();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!trackingTaskId) return;
    const interval = window.setInterval(async () => {
      try {
        const result = await fetchTaskById(trackingTaskId);
        if (["succeeded", "failed", "canceled", "timeout"].includes(result.status)) {
          setTrackingTaskId(null);
        }
      } catch {
        // Keep polling quietly; transient failures are expected during restarts.
      }
    }, 2000);
    return () => window.clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [trackingTaskId, apiBase, uiAuthReady]);

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
        healthy: health?.telegram_bot_healthy ?? health?.telegramd_healthy ?? health?.channel_gateway_healthy,
        processCount:
          health?.telegram_bot_process_count ?? health?.telegramd_process_count ?? health?.channel_gateway_process_count,
        memoryRssBytes:
          health?.telegram_bot_memory_rss_bytes ?? health?.telegramd_memory_rss_bytes ?? health?.channel_gateway_memory_rss_bytes,
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
          detail:
            row.key === "telegram_bot" && (health?.telegram_configured_bot_count ?? 0) > 1
              ? t(
                  `Telegram 接入进程正在承载 ${health?.telegram_configured_bot_count ?? 0} 个机器人。`,
                  `The Telegram service is currently carrying ${health?.telegram_configured_bot_count ?? 0} robots.`,
                )
              : t("至少从健康探针看，进程已经起来了。", "The health probe indicates the daemon process is up."),
        };
      }
      if (row.healthy === false) {
        return {
          ...row,
          category: "stopped",
          statusLabel: t("进程未运行", "Daemon stopped"),
          detail:
            row.key === "telegram_bot" && (health?.telegram_configured_bot_count ?? 0) > 1
              ? t(
                  `已经配置 ${health?.telegram_configured_bot_count ?? 0} 个 Telegram 机器人，但 Telegram 接入进程还没运行。`,
                  `${health?.telegram_configured_bot_count ?? 0} Telegram robots are configured, but the Telegram service is not running yet.`,
                )
              : t("当前没有检测到对应进程。", "The corresponding daemon process was not detected."),
        };
      }
      return {
        ...row,
        category: "unknown",
        statusLabel: t("状态未知", "Unknown"),
        detail: t("当前还拿不到足够的进程状态。", "There is not enough process state information yet."),
      };
    });
  }, [adapterHealthRows, health, lang, waLoginStatus]);
  const telegramBotCards = useMemo(() => {
    const statusMap = new Map((health?.telegram_bot_statuses ?? []).map((status) => [status.name, status]));
    const names = telegramConfigDrafts.map((bot, index) => (index === 0 ? "primary" : bot.name.trim())).filter(Boolean);
    const configMap = new Map(telegramConfigDrafts.map((bot, index) => [index === 0 ? "primary" : bot.name.trim(), bot]));
    return names.map((name, index) => {
      const isPrimary = index === 0 && name === "primary";
      const runtimeStatus = statusMap.get(name);
      const configBot = configMap.get(name);
      const displayName = robotDisplayName(configBot) || (isPrimary ? localizedDefaultAgentName() : name);
      const healthy = runtimeStatus?.healthy ?? false;
      const statusLabel =
        runtimeStatus?.status === "starting"
          ? t("启动中", "Starting")
          : runtimeStatus?.status === "stale"
            ? t("心跳过期", "Heartbeat stale")
            : runtimeStatus?.status === "stopped"
              ? t("已停止", "Stopped")
              : runtimeStatus?.status === "missing"
                ? t("未报到", "No heartbeat yet")
                : healthy === true
          ? t("工作中", "On shift")
          : t("待启动", "Waiting to start");
      const statusTone =
        healthy === true
          ? "emerald"
          : runtimeStatus?.status === "starting"
            ? "white"
            : runtimeStatus?.status === "stale"
              ? "amber"
              : "amber";
      return {
        name,
        displayName,
        monogram: telegramBotMonogram(displayName),
        role: configBot?.role_name?.trim()
          ? configBot.role_name
          : isPrimary
            ? localizedDefaultAgentName()
            : t("未命名角色", "Unnamed role"),
        description: configBot?.description?.trim() || "",
        access_mode: configBot?.access_mode || "public",
        allowed_telegram_usernames: configBot?.allowed_telegram_usernames ?? [],
        statusLabel,
        statusTone,
        lastError: runtimeStatus?.last_error,
        heartbeatTs: runtimeStatus?.last_heartbeat_ts ?? null,
      };
    });
  }, [health, lang, telegramConfigDrafts]);
  const telegramBotsOnShiftCount = useMemo(
    () => telegramBotCards.filter((bot) => bot.statusTone === "emerald").length,
    [telegramBotCards],
  );
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
  const channelGatewayRow = useMemo(
    () => serviceStatusRows.find((row) => row.key === "telegram_bot"),
    [serviceStatusRows],
  );
  const telegramRobotControl = useCallback(
    (bot: { statusTone: string }) => {
      if (!channelGatewayRow) return null;
      const serviceName = channelGatewayRow.serviceName || "telegramd";
      const affectsAllTelegramBots = telegramBotCards.length > 1;
      const botOnline = bot.statusTone === "emerald";
      if (botOnline) {
        return {
          action: "stop" as const,
          className: "theme-service-action theme-service-action-stop",
          label: t("暂停机器人", "Pause robot"),
          title: affectsAllTelegramBots
            ? t(
                "现在暂停的是 Telegram 接入进程，这个渠道下的其他机器人也会一起暂停。",
                "This pauses the Telegram service, so the other robots on this channel will pause too.",
              )
            : t("暂停后，这个机器人会先停止接收新消息。", "Pausing stops this robot from receiving new messages."),
          serviceName,
        };
      }
      if (channelGatewayRow.healthy === true) {
        return {
          action: "restart" as const,
          className: "theme-service-action theme-service-action-restart",
          label: t("重启机器人", "Restart robot"),
          title: affectsAllTelegramBots
            ? t(
                "现在重启的是 Telegram 接入进程，这个渠道下的其他机器人也会一起重启。",
                "This restarts the Telegram service, so the other robots on this channel will restart too.",
              )
            : t("重启后，这个机器人会重新接入。", "Restarting reconnects this robot."),
          serviceName,
        };
      }
      return {
        action: "start" as const,
        className: "theme-service-action theme-service-action-start",
        label: t("启动机器人", "Start robot"),
        title: affectsAllTelegramBots
          ? t(
              "现在启动的是 Telegram 接入进程，这个渠道下的其他机器人也会一起接入。",
              "This starts the Telegram service, so the other robots on this channel will come online too.",
            )
          : t("启动后，这个机器人就能开始接收消息。", "Starting brings this robot online to receive messages."),
        serviceName,
      };
    },
    [channelGatewayRow, telegramBotCards.length, t],
  );
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
      llmDraftApiKey.trim() !== ""
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
  const agentLlmOptions = useMemo<AgentLlmOption[]>(
    () =>
      (llmConfigData?.vendors ?? []).map((vendor) => ({
        vendor: vendor.name,
        label: vendor.name,
        models: vendor.models ?? [],
        defaultModel: vendor.default_model || vendor.models?.[0] || "",
      })),
    [llmConfigData],
  );
  const robotPersonaPresets = useMemo(
    () => [
      {
        id: "assistant",
        label: t("私人秘书", "Personal assistant"),
        description: t("帮我安排日程、提醒事项和日常沟通。", "Handles planning, reminders, and daily coordination."),
        personaPrompt: t(
          "你是一位贴心、可靠的私人秘书。优先帮用户安排日程、整理待办、提醒重要事项，并用温和、清晰、安心的语气回复。回答尽量简洁，先给结论，再补充必要细节。",
          "You are a thoughtful and reliable personal assistant. Help with plans, to-dos, reminders, and daily coordination. Respond in a calm, clear, reassuring tone. Keep answers concise: lead with the answer, then add only the needed detail.",
        ),
      },
      {
        id: "support",
        label: t("客服接待", "Customer support"),
        description: t("负责答疑、接待和常见问题处理。", "Handles support questions, onboarding, and routine help."),
        personaPrompt: t(
          "你是一位耐心、专业的客服助手。优先准确理解问题，先直接回答，再给下一步建议。语气友好、不推诿、不使用太多技术词。如果信息不足，先问 1 个最关键的问题。",
          "You are a patient and professional support assistant. Understand the issue first, answer directly, then suggest the next step. Be friendly and avoid technical jargon. If information is missing, ask for the single most important detail first.",
        ),
      },
      {
        id: "sales",
        label: t("销售助手", "Sales assistant"),
        description: t("负责咨询转化、报价说明和下一步推进。", "Handles discovery, pricing explanations, and next-step conversion."),
        personaPrompt: t(
          "你是一位清晰、主动的销售助手。回答时先抓住用户需求，再用简明语言介绍方案价值、适用场景和下一步行动。不要夸张承诺，语气自然、有推进感。",
          "You are a clear and proactive sales assistant. Start from the user's need, explain the value and fit in simple language, and guide toward the next step. Avoid exaggerated promises; keep the tone natural and forward-moving.",
        ),
      },
      {
        id: "content",
        label: t("内容助手", "Content assistant"),
        description: t("负责写文案、整理信息和润色表达。", "Helps write copy, organize information, and polish wording."),
        personaPrompt: t(
          "你是一位擅长整理和表达的内容助手。优先把信息讲清楚、讲顺，输出结构化、好读、可直接使用的内容。默认避免长篇空话，尽量让结果拿来就能发。",
          "You are a content assistant who excels at organizing and expressing information. Make outputs clear, structured, and ready to use. Avoid fluffy long-form wording; the result should be practical and publishable as-is.",
        ),
      },
    ],
    [t],
  );
  const beginnerRobotSkillPreset = useMemo(() => {
    const preferred = ["health_check", "http_basic", "rss_fetch", "image_vision", "audio_transcribe", "audio_synthesize"];
    return preferred.filter((name) => configuredEnabledSkills.has(name) || managedSkills.includes(name));
  }, [configuredEnabledSkills, managedSkills]);
  const botEditorSkillMode = useMemo(() => {
    if (!botEditorDraft) return "inherit";
    const current = [...new Set(botEditorDraft.allowed_skills ?? [])].sort();
    const beginner = [...new Set(beginnerRobotSkillPreset)].sort();
    if (current.length === 0) return "inherit";
    if (current.length === beginner.length && current.every((value, index) => value === beginner[index])) {
      return "common";
    }
    return "custom";
  }, [beginnerRobotSkillPreset, botEditorDraft]);
  const serializeTelegramConfigDrafts = (bots: TelegramBotConfigEntry[]) =>
    JSON.stringify(
      bots.map((bot, index) => ({
        name: index === 0 ? "primary" : bot.name.trim(),
        bot_token: bot.bot_token.trim(),
        agent_id: (bot.agent_id || "main").trim() || "main",
        admins: bot.admins ?? [],
        allowlist: bot.allowlist ?? [],
        is_primary: index === 0,
        role_name: bot.role_name?.trim() || "",
        description: bot.description?.trim() || "",
        persona_prompt: bot.persona_prompt?.trim() || "",
        preferred_vendor: bot.preferred_vendor?.trim() || "",
        preferred_model: bot.preferred_model?.trim() || "",
        allowed_skills: bot.allowed_skills ?? [],
      })),
    );
  const normalizedSkillsSearchQuery = useMemo(() => skillsSearchQuery.trim().toLowerCase(), [skillsSearchQuery]);
  const filteredManagedSkills = useMemo(
    () => managedSkills.filter((name) => !normalizedSkillsSearchQuery || name.toLowerCase().includes(normalizedSkillsSearchQuery)),
    [managedSkills, normalizedSkillsSearchQuery],
  );
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
      : t(
          "这是一个额外接入的技能。先在这里设定开关，保存后才会真正生效。",
          "This is an additional integrated skill. Choose its switch here and save to apply it.",
        );
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
    setLlmDraftApiKey("");
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
      channels: {
        title: t("机器人设置", "Robot Settings"),
        desc: t("这里管理机器人。", "Manage robots here."),
      },
      models: {
        title: t("模型设置", "Model Settings"),
        desc: t("这里设置 RustClaw 主要用哪家模型、模型名，以及该厂商的接口地址和密钥。", "Choose RustClaw's main LLM vendor, model, endpoint, and API key here."),
      },
      skills: {
        title: t("技能设置", "Skill Settings"),
        desc: t("这里单独管理技能开关和运行中的技能列表。", "Manage skill switches and the current runtime skill list here."),
      },
      chat: {
        title: t("对话测试", "Chat Test"),
        desc: t("用最简单的方式给 RustClaw 发一条消息，确认它能正常回应。", "Send a simple message to RustClaw and confirm it can respond."),
      },
      usage: {
        title: t("使用记录", "Usage History"),
        desc: t("这里能看到每一次真实请求。点开一条，就能看当时发了什么、模型回了什么。", "See each real request here. Open any record to inspect the input and the model response."),
      },
      logs: {
        title: t("故障日志", "Logs"),
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
        id: "channels" as const,
        label: t("机器人设置", "Robot Settings"),
        hint: t("机器人", "robots"),
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
        id: "chat" as const,
        label: t("对话测试", "Chat Test"),
        hint: t("试消息", "send a message"),
        icon: <MessageCircle className="h-4 w-4" />,
      },
      {
        id: "usage" as const,
        label: t("使用记录", "Usage History"),
        hint: t("看发了什么", "request history"),
        icon: <Timer className="h-4 w-4" />,
      },
      {
        id: "logs" as const,
        label: t("故障日志", "Logs"),
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
  const usageChannelLabel = useCallback(
    (channel?: string | null) => {
      if (channel === "telegram") return "Telegram";
      if (channel === "whatsapp") return "WhatsApp";
      if (channel === "feishu") return "Feishu";
      if (channel === "lark") return "Lark";
      if (channel === "ui") return t("控制台", "Console");
      return channel || "--";
    },
    [t],
  );
  const usageStatusLabel = useCallback(
    (status?: string | null) => {
      if (status === "ok") return t("成功", "Success");
      if (!status || status === "error") return t("失败", "Failed");
      return status;
    },
    [t],
  );
  const usageRecords = usageRecordsData?.records ?? [];
  const usagePagination = usageRecordsData?.pagination ?? null;
  const usageChannelOptions = useMemo(() => {
    const defaults = ["all", "telegram", "whatsapp", "feishu", "lark", "ui"];
    const discovered = usageRecords
      .map((record) => (record.channel || "").trim())
      .filter(Boolean)
      .filter((value, index, arr) => arr.indexOf(value) === index);
    return defaults.filter((value, index, arr) => arr.indexOf(value) === index).concat(discovered.filter((value) => !defaults.includes(value)));
  }, [usageRecords]);
  const usageStats = usageRecordsData?.stats ?? {
    total_requests: 0,
    success_requests: 0,
    failed_requests: 0,
    prompt_tokens: 0,
    completion_tokens: 0,
    total_tokens: 0,
  };
  const selectedUsageRecordSummary = useMemo(
    () => usageRecords.find((record) => record.record_id === selectedUsageRecordId) ?? null,
    [selectedUsageRecordId, usageRecords],
  );
  const selectedUsageRecord = selectedUsageRecordDetail ?? selectedUsageRecordSummary;
  const suggestedNextStep = useMemo(() => {
    if (!isOnline) {
      return {
        title: t("先检查服务是否启动", "Check whether the service is running"),
        desc: t("如果页面显示离线，先确认 clawd 地址是否正确，或者服务是否已经启动。", "If the console looks offline, first confirm the clawd address and whether the service is running."),
        page: "dashboard" as const,
        cta: t("查看首页提示", "Open Home"),
      };
    }
    if ((health?.telegram_configured_bot_count ?? 0) === 0) {
      return {
        title: t("去机器人设置完成配置", "Open Robot Settings to finish setup"),
        desc: t("第一次使用时，先新增一个机器人并保存。", "For first-time setup, create and save your first robot."),
        page: "channels" as const,
        cta: t("打开机器人设置", "Open Robot Settings"),
      };
    }
    if (healthyServiceCount === 0) {
      return {
        title: t("去机器人设置启动连接服务", "Open Robot Settings to start services"),
        desc: t("如果一个服务都没运行，就先到机器人设置里把你需要用到的连接服务启动起来。", "If no connection service is running yet, start the services you need from Robot Settings."),
        page: "channels" as const,
        cta: t("打开机器人设置", "Open Robot Settings"),
      };
    }
    return {
      title: t("可以开始试一条消息了", "You can try a message now"),
      desc: t("基础状态已经差不多就绪，可以去对话测试页给 RustClaw 发一条简单消息。", "The basics look ready, so you can send a simple message in the chat test page."),
      page: "chat" as const,
      cta: t("去试一条消息", "Try a message"),
    };
  }, [healthyServiceCount, health?.telegram_configured_bot_count, isOnline, lang]);
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

  const debugUsageRows = (usage?: DebugUsageSnapshot | null) => {
    if (!usage) return [];
    return [
      { key: "prompt_tokens", label: t("输入 tokens", "Prompt tokens"), value: usage.prompt_tokens ?? usage.input_tokens ?? null },
      { key: "completion_tokens", label: t("输出 tokens", "Completion tokens"), value: usage.completion_tokens ?? usage.output_tokens ?? null },
      { key: "total_tokens", label: t("总计", "Total"), value: usage.total_tokens ?? null },
      { key: "reasoning_tokens", label: t("推理 tokens", "Reasoning"), value: usage.reasoning_tokens ?? null },
      { key: "cached_tokens", label: t("缓存命中", "Cached"), value: usage.cached_tokens ?? usage.cache_read_input_tokens ?? null },
    ].filter((row) => row.value != null);
  };

  const renderTaskDebugPanel = () => {
    if (!debugModeEnabled) return null;
    const activeTaskId = taskResult?.task_id?.trim();
    return (
      <section className="mt-4 rounded-2xl border border-sky-400/15 bg-sky-500/5 p-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <p className="text-[11px] uppercase tracking-[0.24em] text-sky-200/70">{t("调试模式", "Debug mode")}</p>
            <h4 className="mt-2 text-base font-semibold text-white">{t("模型请求与返回", "Model requests and responses")}</h4>
            <p className="mt-1 text-sm text-white/55">
              {t("这里展示最终 prompt、发给模型的 request JSON、原始返回和 token 使用情况。", "This shows the final prompt, request JSON, raw model output, and token usage.")}
            </p>
          </div>
          {activeTaskId ? <span className="rounded-full border border-white/10 bg-black/20 px-3 py-1 text-xs text-white/60">{activeTaskId}</span> : null}
        </div>

        {!activeTaskId ? (
          <p className="mt-4 rounded-xl border border-white/10 bg-black/20 px-4 py-3 text-sm text-white/65">
            {t("先跑一条任务，完成后这里就会自动显示调试数据。", "Run a task first. The debug data will appear here automatically once it finishes.")}
          </p>
        ) : null}

        {taskDebugError ? (
          <p className="mt-4 rounded-xl border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
            {t("调试信息读取失败", "Debug data failed to load")}: {taskDebugError}
          </p>
        ) : null}

        {taskDebugLoading ? (
          <div className="mt-4 flex items-center gap-2 text-sm text-white/65">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("正在加载调试数据…", "Loading debug data...")}
          </div>
        ) : null}

        {!taskDebugLoading && activeTaskId && !taskDebugError && (taskDebugData?.entries.length ?? 0) === 0 ? (
          <p className="mt-4 rounded-xl border border-white/10 bg-black/20 px-4 py-3 text-sm text-white/65">
            {t("这条任务暂时没有模型调用记录。可能它还没走到 LLM，或者这是纯技能/纯系统动作。", "This task has no model call records yet. It may not have reached the LLM, or it may be a pure skill/system action.")}
          </p>
        ) : null}

        {(taskDebugData?.entries.length ?? 0) > 0 ? (
          <div className="mt-4 space-y-4">
            {taskDebugData!.entries.map((entry, index) => {
              const usageRows = debugUsageRows(entry.usage);
              return (
                <article key={`${entry.ts ?? "debug"}-${index}`} className="rounded-2xl border border-white/10 bg-black/20 p-4">
                  <div className="flex flex-wrap items-center gap-2 text-xs text-white/60">
                    <span className="rounded-full border border-white/10 bg-white/5 px-2.5 py-1">{entry.vendor || "--"}</span>
                    <span className="rounded-full border border-white/10 bg-white/5 px-2.5 py-1">{entry.model || "--"}</span>
                    <span className={entry.status === "ok" ? "rounded-full border border-emerald-400/20 bg-emerald-400/10 px-2.5 py-1 text-emerald-200" : "rounded-full border border-amber-400/20 bg-amber-400/10 px-2.5 py-1 text-amber-200"}>
                      {entry.status || "--"}
                    </span>
                    {entry.prompt_file ? <span>{entry.prompt_file}</span> : null}
                    {entry.ts ? <span>{toLocalTime(entry.ts * 1000)}</span> : null}
                  </div>

                  {usageRows.length > 0 ? (
                    <div className="mt-3 grid gap-2 sm:grid-cols-2 xl:grid-cols-5">
                      {usageRows.map((row) => (
                        <div key={row.key} className="rounded-xl border border-white/10 bg-white/5 px-3 py-2">
                          <p className="text-[11px] uppercase tracking-widest text-white/45">{row.label}</p>
                          <p className="mt-1 text-sm font-medium text-white">{row.value}</p>
                        </div>
                      ))}
                    </div>
                  ) : null}

                  {entry.error ? (
                    <p className="mt-3 rounded-xl border border-red-500/25 bg-red-500/10 px-3 py-2 text-sm text-red-200">{entry.error}</p>
                  ) : null}

                  <div className="mt-4 space-y-3">
                    <details className="rounded-xl border border-white/10 bg-[#12151f] p-3" open={index === taskDebugData!.entries.length - 1}>
                      <summary className="cursor-pointer text-sm font-medium text-white/85">{t("最终 Prompt", "Final prompt")}</summary>
                      <pre className="theme-scrollbar mt-3 max-h-80 overflow-auto whitespace-pre-wrap break-words text-xs text-white/75">{entry.prompt || "--"}</pre>
                    </details>

                    <DebugPayloadPanel
                      title={t("Request JSON", "Request JSON")}
                      value={entry.request_payload}
                      formatLabel={t("格式化查看", "Formatted view")}
                      rawLabel={t("查看原始 JSON", "View raw JSON")}
                    />

                    <details className="rounded-xl border border-white/10 bg-[#12151f] p-3">
                      <summary className="cursor-pointer text-sm font-medium text-white/85">{t("原始返回", "Raw response")}</summary>
                      <pre className="theme-scrollbar mt-3 max-h-80 overflow-auto whitespace-pre-wrap break-words text-xs text-white/75">{entry.raw_response || "--"}</pre>
                    </details>

                    <details className="rounded-xl border border-white/10 bg-[#12151f] p-3">
                      <summary className="cursor-pointer text-sm font-medium text-white/85">{t("清洗后的返回", "Clean response")}</summary>
                      <pre className="theme-scrollbar mt-3 max-h-80 overflow-auto whitespace-pre-wrap break-words text-xs text-white/75">{entry.clean_response || entry.response || "--"}</pre>
                    </details>
                  </div>
                </article>
              );
            })}
          </div>
        ) : null}
      </section>
    );
  };

  const renderUsageRecordModal = () => {
    if (!selectedUsageRecord) return null;

    const detailMeta = [
      { label: t("渠道", "Channel"), value: usageChannelLabel(selectedUsageRecord.channel) },
      { label: t("模型", "Model"), value: selectedUsageRecord.model || "--" },
      { label: t("提供方", "Provider"), value: selectedUsageRecord.provider || selectedUsageRecord.vendor || "--" },
      { label: t("状态", "Status"), value: usageStatusLabel(selectedUsageRecord.status) },
      { label: "task_id", value: selectedUsageRecord.task_id },
      { label: t("时间", "Time"), value: selectedUsageRecord.ts ? toLocalDateTime(selectedUsageRecord.ts * 1000) : "--" },
    ];
    const locale = lang === "zh" ? "zh-CN" : "en-US";
    const detailReady = Boolean(selectedUsageRecordDetail);
    const chainEntries = selectedUsageRecordDetail?.entries ?? [];

    return (
      <div className="fixed inset-0 z-50 flex items-center justify-center bg-[rgba(8,10,16,0.72)] px-4 py-6 backdrop-blur-sm" onClick={() => setSelectedUsageRecordId(null)}>
        <div
          className="theme-panel theme-scrollbar flex max-h-[92vh] w-full max-w-5xl flex-col overflow-auto border-white/12 bg-[linear-gradient(180deg,rgba(38,41,51,0.98),rgba(23,26,35,0.98))] p-5 shadow-[0_32px_90px_rgba(0,0,0,0.42)] sm:p-6"
          onClick={(event) => event.stopPropagation()}
        >
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div className="max-w-3xl">
              <p className="text-[11px] uppercase tracking-[0.24em] text-[#ffcfb5]/75">{t("记录详情", "Record detail")}</p>
              <h3 className="mt-2 text-xl font-semibold text-white sm:text-2xl">{selectedUsageRecord.request_text || t("这次请求没有记录到用户原文。", "This request has no captured user text.")}</h3>
              <p className="mt-2 text-sm leading-7 text-white/60">
                {t(
                  "这里按时间顺序展开这次请求的完整链路，方便你确认路由判断、最终提示词和每一步返回了什么。",
                  "This expands the full chain for this request in time order, so you can inspect the routing step, final prompts, and each response.",
                )}
              </p>
            </div>
            <button
              type="button"
              onClick={() => setSelectedUsageRecordId(null)}
              className="inline-flex h-12 w-12 items-center justify-center rounded-2xl border border-white/12 bg-white/6 text-white/72 transition hover:bg-white/12"
              aria-label={t("关闭详情", "Close detail")}
            >
              <X className="h-5 w-5" />
            </button>
          </div>

          <div className="mt-5 grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {detailMeta.map((item) => (
              <div key={item.label} className="rounded-2xl border border-white/10 bg-black/18 px-4 py-3">
                <p className="text-[11px] uppercase tracking-widest text-white/40">{item.label}</p>
                <p className="mt-1 break-all text-sm leading-6 text-white/82">{item.value}</p>
              </div>
            ))}
          </div>

          <div className="mt-4 grid gap-3 sm:grid-cols-3">
            <div className="rounded-2xl border border-white/10 bg-black/18 px-4 py-3">
              <p className="text-[11px] uppercase tracking-widest text-white/40">{t("输入 Tokens", "Prompt tokens")}</p>
              <p className="mt-1 text-lg font-semibold text-white">{formatInteger(selectedUsageRecord.prompt_tokens, locale)}</p>
            </div>
            <div className="rounded-2xl border border-white/10 bg-black/18 px-4 py-3">
              <p className="text-[11px] uppercase tracking-widest text-white/40">{t("输出 Tokens", "Completion tokens")}</p>
              <p className="mt-1 text-lg font-semibold text-white">{formatInteger(selectedUsageRecord.completion_tokens, locale)}</p>
            </div>
            <div className="rounded-2xl border border-white/10 bg-black/18 px-4 py-3">
              <p className="text-[11px] uppercase tracking-widest text-white/40">{t("总 Tokens", "Total tokens")}</p>
              <p className="mt-1 text-lg font-semibold text-white">{formatInteger(selectedUsageRecord.total_tokens, locale)}</p>
            </div>
          </div>

          <div className="mt-4 rounded-2xl border border-white/10 bg-black/18 px-4 py-3">
            <p className="text-[11px] uppercase tracking-widest text-white/40">{t("链路节点数", "Chain steps")}</p>
            <p className="mt-1 text-lg font-semibold text-white">{formatInteger(selectedUsageRecord.llm_call_count, locale)}</p>
          </div>

          {selectedUsageRecord.error ? (
            <p className="mt-4 rounded-2xl border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
              {selectedUsageRecord.error}
            </p>
          ) : null}

          {selectedUsageRecordLoading ? (
            <div className="mt-4 flex items-center gap-2 rounded-2xl border border-white/10 bg-black/18 px-4 py-3 text-sm text-white/65">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("正在加载完整参数和返回内容…", "Loading the full request and response...")}
            </div>
          ) : null}

          {selectedUsageRecordError ? (
            <p className="mt-4 rounded-2xl border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
              {t("详情读取失败", "Detail load failed")}: {selectedUsageRecordError}
            </p>
          ) : null}

          {detailReady ? (
            <div className="mt-4 space-y-4">
              {chainEntries.map((entry, index) => (
                <article key={`${entry.prompt_file || "step"}-${entry.ts || index}-${index}`} className="rounded-2xl border border-white/10 bg-[#12151f] p-4">
                  <div className="flex flex-wrap items-center gap-2 text-xs text-white/60">
                    <span className="rounded-full border border-white/10 bg-white/5 px-2.5 py-1">{t("第", "Step")} {index + 1}</span>
                    <span className="rounded-full border border-white/10 bg-white/5 px-2.5 py-1">{entry.prompt_file || "--"}</span>
                    <span className={entry.status === "ok" ? "rounded-full border border-emerald-400/20 bg-emerald-400/10 px-2.5 py-1 text-emerald-200" : "rounded-full border border-rose-400/20 bg-rose-400/10 px-2.5 py-1 text-rose-200"}>
                      {usageStatusLabel(entry.status)}
                    </span>
                    <span>{entry.ts ? toLocalDateTime(entry.ts * 1000) : "--"}</span>
                    <span>{entry.model || "--"}</span>
                  </div>

                  <div className="mt-3 grid gap-3 sm:grid-cols-3">
                    <div className="rounded-xl border border-white/10 bg-white/5 px-3 py-2">
                      <p className="text-[11px] uppercase tracking-widest text-white/40">{t("输入", "Prompt")}</p>
                      <p className="mt-1 text-sm text-white">{formatInteger(entry.prompt_tokens, locale)}</p>
                    </div>
                    <div className="rounded-xl border border-white/10 bg-white/5 px-3 py-2">
                      <p className="text-[11px] uppercase tracking-widest text-white/40">{t("输出", "Completion")}</p>
                      <p className="mt-1 text-sm text-white">{formatInteger(entry.completion_tokens, locale)}</p>
                    </div>
                    <div className="rounded-xl border border-white/10 bg-white/5 px-3 py-2">
                      <p className="text-[11px] uppercase tracking-widest text-white/40">{t("总计", "Total")}</p>
                      <p className="mt-1 text-sm text-white">{formatInteger(entry.total_tokens, locale)}</p>
                    </div>
                  </div>

                  {entry.error ? (
                    <p className="mt-3 rounded-xl border border-red-500/25 bg-red-500/10 px-3 py-2 text-sm text-red-200">{entry.error}</p>
                  ) : null}

                  <div className="mt-4 space-y-3">
                    <DebugPayloadPanel
                      title={t("请求参数", "Request payload")}
                      value={entry.request_payload}
                      defaultOpen
                      formatLabel={t("格式化查看", "Formatted view")}
                      rawLabel={t("查看原始 JSON", "View raw JSON")}
                    />
                    <details className="rounded-xl border border-white/10 bg-black/20 p-3">
                      <summary className="cursor-pointer text-sm font-medium text-white/85">{t("最终提示词", "Final prompt")}</summary>
                      <pre className="theme-scrollbar mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-words text-xs leading-6 text-white/75">{entry.prompt || "--"}</pre>
                    </details>
                    <details className="rounded-xl border border-white/10 bg-black/20 p-3" open>
                      <summary className="cursor-pointer text-sm font-medium text-white/85">{t("返回内容", "Response")}</summary>
                      <pre className="theme-scrollbar mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-words text-xs leading-6 text-white/75">{entry.clean_response || "--"}</pre>
                    </details>
                    <details className="rounded-xl border border-white/10 bg-black/20 p-3">
                      <summary className="cursor-pointer text-sm font-medium text-white/85">{t("原始返回", "Raw response")}</summary>
                      <pre className="theme-scrollbar mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-words text-xs leading-6 text-white/75">{entry.raw_response || "--"}</pre>
                    </details>
                  </div>
                </article>
              ))}
            </div>
          ) : (
            <div className="mt-4 space-y-3">
              <div className="rounded-2xl border border-white/10 bg-[#12151f] p-4 text-sm text-white/60">
                {t("详情加载后会按时间顺序显示整条请求链路。", "The full request chain will appear in time order after the detail finishes loading.")}
              </div>
            </div>
          )}
        </div>
      </div>
    );
  };

  return (
    <div className="theme-shell min-h-screen">
      <header className="theme-header sticky top-0 z-40 border-b border-white/10 px-3 py-3 sm:px-6 sm:py-4">
        <div className="mx-auto flex max-w-7xl flex-wrap items-center justify-between gap-3">
          <div className="min-w-0">
            <h1 className="flex items-center gap-2 text-lg font-bold tracking-tight sm:text-2xl">
              <span className="text-lg leading-none sm:text-2xl">🦞</span>
              <span className="truncate">RustClaw</span>
            </h1>
          </div>

          <div className="flex items-center gap-2">
            <button
              onClick={toggleThemeMode}
              className="theme-topbar-btn"
            >
              {themeMode === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
              {themeMode === "dark" ? t("日间模式", "Day mode") : t("夜间模式", "Night mode")}
            </button>
            <button
              onClick={() => setLang((v) => (v === "zh" ? "en" : "zh"))}
              className="theme-topbar-btn"
            >
              {lang === "zh" ? "中文" : "EN"}
            </button>
            <button
              type="button"
              onClick={logout}
              className="theme-topbar-btn"
              title={t("退出登录，需重新输入 key", "Log out; key required to sign in again")}
            >
              {t("退出", "Log out")}
            </button>
          </div>
        </div>
      </header>

      <div className="mx-auto grid max-w-7xl gap-4 px-3 py-4 sm:px-6 sm:py-6 lg:grid-cols-[220px_minmax(0,1fr)]">
        <aside className="lg:sticky lg:top-24 lg:self-start">
          <div className="theme-sidebar-shell">
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
                    onClick={() => setCurrentPage(item.id)}
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

        <main className="space-y-4">
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

              <section className="theme-panel p-4 sm:p-5">
                <div className="grid gap-4 xl:grid-cols-[minmax(0,1.2fr)_minmax(280px,0.8fr)]">
                  <div className="rounded-[22px] border border-white/10 bg-black/20 p-4 sm:p-5">
                    <div className="flex flex-wrap items-center justify-between gap-3">
                      <div>
                        <p className="theme-kicker text-[10px] uppercase tracking-[0.3em]">{t("Telegram 服务", "Telegram service")}</p>
                        <h3 className="mt-2 text-xl font-semibold text-white">{channelGatewayRow?.statusLabel || t("状态未知", "Unknown state")}</h3>
                      </div>
                      <span
                        className={
                          channelGatewayRow?.category === "ready"
                            ? "rounded-full border border-emerald-500/25 bg-emerald-500/10 px-3 py-1 text-xs text-emerald-200"
                            : channelGatewayRow?.category === "attention"
                              ? "rounded-full border border-amber-500/25 bg-amber-500/10 px-3 py-1 text-xs text-amber-200"
                              : "rounded-full border border-red-500/25 bg-red-500/10 px-3 py-1 text-xs text-red-200"
                        }
                      >
                        {channelGatewayRow?.healthy === true ? t("服务在线", "Service online") : t("未就绪", "Not ready")}
                      </span>
                    </div>
                    <p className="mt-3 text-sm text-white/62">
                      {channelGatewayRow?.detail || t("当前还没有拿到 Telegram 服务状态。", "The Telegram service state is not available yet.")}
                    </p>
                    <div className="mt-4 flex flex-wrap gap-2">
                      <span className="theme-service-kpi">{t("已配置机器人", "Configured robots")} {health?.telegram_configured_bot_count ?? 0}</span>
                      <span className="theme-service-kpi">{t("在线机器人", "Robots online")} {telegramBotsOnShiftCount}</span>
                    </div>
                    <div className="mt-4 flex flex-wrap gap-2">
                      <button
                        type="button"
                        onClick={() => void controlService(channelGatewayRow?.serviceName || "telegramd", channelGatewayRow?.healthy === true ? "restart" : "start")}
                        disabled={Boolean(serviceActionLoading[channelGatewayRow?.serviceName || "telegramd"])}
                        className="theme-accent-btn"
                      >
                        {serviceActionLoading[channelGatewayRow?.serviceName || "telegramd"] ? <Loader2 className="h-4 w-4 animate-spin" /> : <Server className="h-4 w-4" />}
                        {channelGatewayRow?.healthy === true ? t("重启", "Restart") : t("启动", "Start")}
                      </button>
                      <button type="button" onClick={() => setCurrentPage("channels")} className="theme-secondary-btn">
                        <Database className="h-4 w-4" />
                        {t("打开机器人设置", "Open Robot Settings")}
                      </button>
                    </div>
                  </div>

                  <div className="rounded-[22px] border border-white/10 bg-white/5 p-4 sm:p-5">
                    <p className="text-[10px] uppercase tracking-[0.28em] text-white/45">{t("下一步", "Next")}</p>
                    <h3 className="mt-2 text-xl font-semibold text-white">{suggestedNextStep.title}</h3>
                    <p className="mt-3 text-sm text-white/62">{suggestedNextStep.desc}</p>
                    <div className="mt-4 flex flex-wrap gap-2">
                      <button
                        type="button"
                        onClick={() => setCurrentPage(suggestedNextStep.page)}
                        className="theme-accent-btn"
                      >
                        <RefreshCw className="h-4 w-4" />
                        {suggestedNextStep.cta}
                      </button>
                      <button type="button" onClick={() => setCurrentPage("chat")} className="theme-secondary-btn">
                        <MessageCircle className="h-4 w-4" />
                        {t("试消息", "Try Chat")}
                      </button>
                    </div>
                  </div>
                </div>
              </section>

              <details className="group rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
                <summary className="flex cursor-pointer list-none items-center gap-2 text-base font-semibold text-white">
                  <Wrench className="theme-icon-accent h-4 w-4" />
                  <span>{t("基础设置", "Basic Settings")}</span>
                  <span className="ml-auto text-xs font-medium text-white/45">
                    <span className="group-open:hidden">{t("点击展开", "Click to expand")}</span>
                    <span className="hidden group-open:inline">{t("点击收起", "Click to collapse")}</span>
                  </span>
                  <ChevronDown className="h-4 w-4 text-white/55 transition group-open:rotate-180" />
                </summary>
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
              </details>

              <section className="grid gap-3 lg:grid-cols-2 xl:grid-cols-3">
                <div className="rounded-2xl border border-white/10 bg-white/5 p-4">
                  <p className="text-[10px] uppercase tracking-widest text-white/45">{t("整体状态", "Overall status")}</p>
                  <p className="mt-2 text-lg font-semibold text-white">
                    {isOnline ? t("服务在线", "Service online") : t("当前离线", "Currently offline")}
                  </p>
                  <div className="mt-3 space-y-1.5 text-sm text-white/60">
                    <p>{t("版本", "Version")}: {health?.version || "--"}</p>
                    <p>{t("Worker", "Worker")}: {health?.worker_state || "--"}</p>
                  </div>
                </div>
                <div className="rounded-2xl border border-white/10 bg-white/5 p-4">
                  <p className="text-[10px] uppercase tracking-widest text-white/45">{t("任务负载", "Task load")}</p>
                  <p className="mt-2 text-lg font-semibold text-white">
                    {t("排队 {{queue}} / 运行 {{running}}", "Queued {{queue}} / Running {{running}}")
                      .replace("{{queue}}", String(health?.queue_length ?? "--"))
                      .replace("{{running}}", String(health?.running_length ?? "--"))}
                  </p>
                  <div className="mt-3 space-y-1.5 text-sm text-white/60">
                    <p>{t("最久运行", "Oldest task")}: {formatDuration(health?.running_oldest_age_seconds)}</p>
                    <p>{t("超时阈值", "Timeout")}: {formatDuration(health?.task_timeout_seconds)}</p>
                  </div>
                </div>
                <div className="rounded-2xl border border-white/10 bg-white/5 p-4">
                  <p className="text-[10px] uppercase tracking-widest text-white/45">{t("资源占用", "Resources")}</p>
                  <p className="mt-2 text-lg font-semibold text-white">{formatBytes(health?.memory_rss_bytes ?? null)}</p>
                  <div className="mt-3 space-y-1.5 text-sm text-white/60">
                    <p>{t("运行时长", "Uptime")}: {formatDuration(health?.uptime_seconds)}</p>
                    <p>{t("刷新频率", "Refresh")}: {pollingSeconds > 0 ? t(`每 ${pollingSeconds} 秒`, `Every ${pollingSeconds}s`) : t("已关闭", "Off")}</p>
                  </div>
                </div>
              </section>

              <div className="space-y-4">
                <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
                  <h3 className="mb-2 text-base font-semibold">{t("你现在最可能要做的事", "What you probably want to do next")}</h3>
                  <div className="mt-4 grid gap-3">
                    <QuickActionCard
                      title={t("绑定外部账号", "Bind an external account")}
                      cta={t("打开机器人设置", "Open Robot Settings")}
                      onClick={() => setCurrentPage("channels")}
                      icon={<Database className="h-4 w-4" />}
                    />
                    <QuickActionCard
                      title={t("管理渠道和机器人", "Manage channels and robots")}
                      cta={t("打开机器人设置", "Open Robot Settings")}
                      onClick={() => setCurrentPage("channels")}
                      icon={<Server className="h-4 w-4" />}
                    />
                    <QuickActionCard
                      title={t("试一条消息", "Try a message")}
                      cta={t("打开对话测试页", "Open Chat Test")}
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
                                ? "rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-xs text-emerald-200"
                                : row.category === "attention"
                                  ? "rounded-lg border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-xs text-amber-200"
                                  : row.category === "stopped"
                                    ? "rounded-lg border border-red-500/30 bg-red-500/10 px-2 py-1 text-xs text-red-200"
                                : "rounded-lg border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/55"
                            }
                          >
                            {row.statusLabel}
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
                      {tSlash("打开机器人设置 / Open Robot Settings")}
                    </button>
                  </div>
                </section>
              </section>
            </div>
          ) : null}

          {currentPage === "channels" ? (
            <div className="space-y-5">

              {serviceActionMessage ? (
                <p className="rounded-2xl border border-white/10 bg-white/5 px-4 py-3 text-sm text-white/80">
                  {serviceActionMessage}
                </p>
              ) : null}

              {waLoginDialogOpen ? (
                <section className="theme-channel-section">
                  <div className="flex flex-wrap items-center justify-between gap-3">
                    <div>
                      <h3 className="text-lg font-semibold text-white">{t("WhatsApp Web 登录", "WhatsApp Web login")}</h3>
                      <p className="mt-1 text-sm text-white/55">
                        {waLoginStatus?.connected
                          ? t("当前已经登录，可以继续使用。", "WhatsApp Web is already connected.")
                          : t("如果二维码已准备好，直接扫码即可。", "If the QR code is ready, scan it now.")}
                      </p>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      <button
                        type="button"
                        onClick={() => void fetchWhatsappWebLoginStatus()}
                        disabled={waLoginLoading}
                        className="theme-secondary-btn"
                      >
                        {waLoginLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                        {t("刷新状态", "Refresh")}
                      </button>
                      <button
                        type="button"
                        onClick={() => setWaLoginDialogOpen(false)}
                        className="theme-service-action theme-service-action-stop"
                      >
                        {t("收起", "Hide")}
                      </button>
                    </div>
                  </div>

                  <div className="mt-5 rounded-xl border border-white/10 bg-black/20 px-4 py-3">
                    <div className="flex flex-wrap items-center justify-between gap-3">
                      <div>
                        <p className="text-sm text-white/85">{t("连接状态", "Connection")}</p>
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
                        {waLoginStatus?.connected ? t("已登录", "Connected") : t("未登录", "Not connected")}
                      </span>
                    </div>
                  </div>

                  <div className="mt-4 rounded-xl border border-white/10 bg-black/20 p-4">
                    {waLoginStatus?.connected ? (
                      <div className="flex flex-wrap items-center justify-between gap-3">
                        <p className="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
                          {t("WhatsApp Web 已登录，无需扫码。", "WhatsApp Web is already connected.")}
                        </p>
                        <button
                          type="button"
                          onClick={() => void logoutWhatsappWeb()}
                          disabled={waLogoutLoading}
                          className="theme-service-action theme-service-action-stop"
                        >
                          {waLogoutLoading ? t("处理中", "Working") : t("退出登录", "Logout")}
                        </button>
                      </div>
                    ) : waLoginStatus?.qr_data_url ? (
                      <div className="inline-block rounded-xl border border-white/15 bg-white p-3">
                        <img src={waLoginStatus.qr_data_url} alt="WhatsApp QR" className="h-56 w-56" />
                      </div>
                    ) : (
                      <p className="rounded-lg border border-white/10 bg-black/30 px-3 py-2 text-sm text-white/70">
                        {waLoginLoading
                          ? t("正在拉取二维码...", "Fetching QR...")
                          : t("暂无可用二维码，请稍候或重启 WhatsApp Web。", "QR is not ready yet. Please wait or restart WhatsApp Web.")}
                      </p>
                    )}
                    {waLoginStatus?.last_error ? (
                      <p className="mt-3 text-xs text-amber-300">
                        {t("最近错误", "Last error")}: {waLoginStatus.last_error}
                      </p>
                    ) : null}
                    {waLoginError ? (
                      <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                        {waLoginError}
                      </p>
                    ) : null}
                  </div>
                </section>
              ) : null}

              <section id="robot-entry-list" className="theme-channel-section">
                <div className="flex flex-wrap items-start gap-3">
                  <Database className="theme-icon-accent mt-0.5 h-4 w-4" />
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-3">
                      <h3 className="text-lg font-semibold text-white">{t("机器人列表", "Robot list")}</h3>
                      <span className="theme-service-kpi">{t("在线", "Online")} {telegramBotsOnShiftCount}</span>
                    </div>
                  </div>
                  <button
                    type="button"
                    onClick={openAddTelegramBotEditor}
                    className="theme-secondary-btn"
                  >
                    <Sparkles className="h-4 w-4" />
                    {t("新增机器人", "Add robot")}
                  </button>
                </div>

                {telegramConfigError ? (
                  <p className="mt-4 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                    {t("机器人配置读取/保存失败", "Robot config read/save failed")}: {telegramConfigError}
                  </p>
                ) : null}
                {telegramConfigSaveMessage ? (
                  <p className="mt-4 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
                    {telegramConfigSaveMessage}
                  </p>
                ) : null}
                {telegramRestartNoticeVisible ? (
                  <div className="mt-4 rounded-2xl border border-amber-500/25 bg-amber-500/10 px-4 py-4">
                    <div className="flex flex-wrap items-center justify-between gap-3">
                      <div>
                        <p className="text-sm font-medium text-amber-100">{t("这些机器人设置等待重启后生效", "These robot settings will apply after restart")}</p>
                        <p className="mt-1 text-xs text-amber-100/80">
                          {t("你可以继续修改多个机器人，全部弄好后再统一重启一次。", "You can keep editing multiple robots and restart once after everything is ready.")}
                        </p>
                      </div>
                        <button
                          type="button"
                          onClick={async () => {
                            const restarted = await restartSystem();
                            if (restarted) {
                              setTelegramRestartNoticeVisible(false);
                              setTelegramConfigSaveMessage(
                                t(
                                  "RustClaw 已重启完成，机器人设置已经生效。",
                                  "RustClaw restarted successfully. Robot settings are now active.",
                                ),
                              );
                            }
                          }}
                          disabled={systemRestarting}
                          className="theme-secondary-btn px-3 py-2 text-xs"
                        >
                          {systemRestarting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                          {t("现在重启", "Restart now")}
                      </button>
                    </div>
                  </div>
                ) : null}

                {telegramBotCards.length > 0 ? (
                  <div className="mt-6 grid gap-4 lg:grid-cols-2">
                    {telegramBotCards.map((bot, index) => {
                      const control = telegramRobotControl(bot);
                      return (
                      <article key={bot.name} className="theme-channel-bot-card">
                        <div className="flex items-start gap-4">
                          <div className="theme-channel-bot-avatar shrink-0">{bot.monogram}</div>
                          <div className="min-w-0 flex-1">
                            <div className="flex flex-wrap items-center gap-3">
                              <h4 className="text-xl font-semibold tracking-tight text-white">{bot.displayName}</h4>
                              <span className="rounded-full border border-white/12 bg-white/6 px-3 py-1 text-[11px] text-white/65">
                                {robotChannelLabel("telegram")}
                              </span>
                              <span className="rounded-full border border-white/12 bg-white/6 px-3 py-1 text-[11px] text-white/65">
                                {t("访问", "Access")}: {telegramAccessModeLabel(bot.access_mode)}
                              </span>
                              <span
                                className={
                                  bot.statusTone === "emerald"
                                    ? "rounded-full border border-emerald-400/25 bg-emerald-400/10 px-3 py-1 text-xs text-emerald-200"
                                    : bot.statusTone === "amber"
                                      ? "rounded-full border border-amber-400/25 bg-amber-400/10 px-3 py-1 text-xs text-amber-200"
                                      : "rounded-full border border-white/15 bg-white/8 px-3 py-1 text-xs text-white/70"
                                }
                              >
                                {bot.statusLabel}
                              </span>
                            </div>
                            {bot.description.trim() ? <p className="mt-2 text-sm text-white/65">{bot.description}</p> : null}
                          </div>
                        </div>

                        <div className="mt-4 flex flex-wrap gap-2">
                          {control ? (
                            <button
                              type="button"
                              onClick={() => void controlService(control.serviceName, control.action)}
                              disabled={Boolean(serviceActionLoading[control.serviceName])}
                              className={control.className}
                              title={control.title}
                            >
                              {Boolean(serviceActionLoading[control.serviceName]) ? (
                                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                              ) : null}
                              {control.label}
                            </button>
                          ) : null}
                          <button
                            type="button"
                            onClick={() => openEditTelegramBotEditor(index)}
                            className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-3 py-2 text-xs text-white/80 hover:bg-white/10"
                          >
                            <Wrench className="h-3.5 w-3.5" />
                            {t("编辑配置", "Edit settings")}
                          </button>
                          {telegramBotCards.length > 0 ? (
                            <button
                              type="button"
                              onClick={() => removeTelegramBotDraft(index)}
                              className="inline-flex items-center gap-2 rounded-xl border border-red-400/25 bg-red-500/10 px-3 py-2 text-xs text-red-200 hover:bg-red-500/15"
                            >
                              <Trash2 className="h-3.5 w-3.5" />
                              {t("删除机器人", "Remove robot")}
                            </button>
                          ) : null}
                        </div>

                        <div className="mt-5 grid gap-3 border-t border-white/10 pt-4 text-sm">
                          {(bot.access_mode || "public") === "specified" ? (
                            <div className="grid grid-cols-[92px_minmax(0,1fr)] gap-3">
                              <p className="text-white/45">{t("允许账号", "Allowed accounts")}</p>
                              <p className="text-white/72">
                                {(bot.allowed_telegram_usernames ?? []).length > 0
                                  ? bot.allowed_telegram_usernames!.map((name) => `@${name}`).join(", ")
                                  : t("还没有填写", "Not filled yet")}
                              </p>
                            </div>
                          ) : null}
                          <div className="grid grid-cols-[92px_minmax(0,1fr)] gap-3">
                            <p className="text-white/45">{t("最近工作时间", "Last activity")}</p>
                            <p className="text-white/72">
                              {bot.heartbeatTs ? new Date(bot.heartbeatTs * 1000).toLocaleString() : t("还没有", "Not yet")}
                            </p>
                          </div>
                          {bot.lastError ? (
                            <div className="grid grid-cols-[92px_minmax(0,1fr)] gap-3">
                              <p className="text-white/45">{t("最近错误", "Last error")}</p>
                              <p className="text-amber-200/90">{bot.lastError}</p>
                            </div>
                          ) : null}
                        </div>
                      </article>
                      );
                    })}
                  </div>
                ) : (
                  <div className="mt-6 rounded-[24px] border border-dashed border-white/12 bg-black/20 px-5 py-6 text-sm text-white/60">
                    {t(
                      "当前还没有配置任何机器人。先点上面的“新增机器人”，再把第一个 Token 填进去。",
                      "No robots are configured yet. Use Add robot above and fill in the first token.",
                    )}
                  </div>
                )}
              </section>

              {botEditorOpen && botEditorDraft ? (
                <div className="fixed inset-0 z-50 overflow-y-auto bg-slate-950/72 px-4 py-6 backdrop-blur-sm">
                  <div className="flex min-h-full items-start justify-center sm:items-center">
                    <div className="flex w-full max-w-3xl max-h-[calc(100dvh-3rem)] flex-col overflow-hidden rounded-[28px] border border-white/10 bg-[#121722] p-5 shadow-2xl shadow-black/40 sm:p-6">
                    <div className="flex items-start justify-between gap-4">
                      <div>
                        <p className="text-[11px] uppercase tracking-[0.28em] text-white/45">
                          {botEditorIndex == null ? t("新增机器人", "Add robot") : t("编辑机器人", "Edit robot")}
                        </p>
                        <h3 className="mt-2 text-2xl font-semibold text-white">
                          {botEditorIndex == null
                            ? t("填写这个机器人的配置", "Set up this robot entry")
                            : (robotDisplayName(botEditorDraft) || t("未命名机器人", "Unnamed robot"))}
                        </h3>
                        <p className="mt-2 text-sm text-white/60">
                          {t("保存后会直接写入配置；如果需要重启，回到上一层再统一处理。", "Saving writes directly to config. If a restart is needed, handle it from the previous screen later.")}
                        </p>
                      </div>
                      <button
                        type="button"
                        onClick={closeBotEditor}
                        className="inline-flex h-10 w-10 items-center justify-center rounded-2xl border border-white/10 bg-white/5 text-white/70 hover:bg-white/10"
                        aria-label={t("关闭", "Close")}
                      >
                        <X className="h-4 w-4" />
                      </button>
                    </div>

                    <div className="theme-scrollbar mt-6 min-h-0 overflow-y-auto pr-2">
                      <div className="grid gap-4 md:grid-cols-2">
                      <label className="space-y-2">
                        <span className="text-xs uppercase tracking-widest text-white/50">{t("渠道", "Channel")}</span>
                        <select
                          className="theme-input"
                          value={botEditorDraft.channel || "telegram"}
                          onChange={(e) => updateBotEditorDraft("channel", e.target.value)}
                        >
                          <option value="telegram">Telegram</option>
                          <option value="feishu">{`Feishu · ${t("准备中", "Coming soon")}`}</option>
                          <option value="wechat">{`${t("企业微信", "WeCom")} · ${t("准备中", "Coming soon")}`}</option>
                        </select>
                      </label>
                      <label className="space-y-2">
                        <span className="text-xs uppercase tracking-widest text-white/50">{t("机器人名称", "Robot name")}</span>
                        <input
                          className="theme-input"
                          autoComplete="off"
                          autoCorrect="off"
                          autoCapitalize="none"
                          spellCheck={false}
                          name="robot-display-name"
                          value={robotDisplayName(botEditorDraft)}
                          onChange={(e) => updateBotEditorDraft("name", e.target.value)}
                          placeholder={t("例如 销售助手", "For example, Sales assistant")}
                        />
                      </label>
                      <label className="space-y-2">
                        <span className="text-xs uppercase tracking-widest text-white/50">{t("机器人 Token", "Robot token")}</span>
                        <input
                          className="theme-input"
                          type="text"
                          autoComplete="new-password"
                          autoCorrect="off"
                          autoCapitalize="none"
                          spellCheck={false}
                          data-lpignore="true"
                          data-1p-ignore="true"
                          data-form-type="other"
                          name="robot-channel-token"
                          inputMode="text"
                          value={botEditorDraft.bot_token || ""}
                          onChange={(e) => updateBotEditorDraft("bot_token", e.target.value)}
                          placeholder="123456:ABCDEF"
                        />
                      </label>
                      {(botEditorDraft.channel || "telegram") !== "telegram" ? (
                        <div className="rounded-2xl border border-amber-400/20 bg-amber-400/10 px-4 py-3 text-sm text-amber-100 md:col-span-2">
                          {t(
                            "这个版本已经把新增机器人的渠道选择放进来了，但真正可直接保存的还只有 Telegram。飞书和企业微信等后端配置接好后，这里会直接继续沿用。",
                            "This version already includes channel selection when adding a robot, but only Telegram can be saved directly right now. Feishu and WeCom will use the same flow once their backend config endpoints are ready.",
                          )}
                        </div>
                      ) : null}
                      <label className="space-y-2">
                        <span className="text-xs uppercase tracking-widest text-white/50">{t("使用哪个大模型", "Which model vendor")}</span>
                        <select
                          className="theme-input"
                          value={botEditorDraft.preferred_vendor || ""}
                          onChange={(e) => updateBotEditorDraft("preferred_vendor", e.target.value)}
                        >
                          <option value="">{t("跟随全局默认", "Follow global default")}</option>
                          {agentLlmOptions.map((option) => (
                            <option key={option.vendor} value={option.vendor}>
                              {option.label}
                            </option>
                          ))}
                        </select>
                      </label>
                      <label className="space-y-2">
                        <span className="text-xs uppercase tracking-widest text-white/50">{t("模型", "Model")}</span>
                        <select
                          className="theme-input"
                          value={botEditorDraft.preferred_model || ""}
                          onChange={(e) => updateBotEditorDraft("preferred_model", e.target.value)}
                          disabled={!(botEditorDraft.preferred_vendor || "").trim()}
                        >
                          <option value="">
                            {(botEditorDraft.preferred_vendor || "").trim() ? t("请选择模型", "Choose a model") : t("跟随全局默认", "Follow global default")}
                          </option>
                          {(agentLlmOptions.find((option) => option.vendor === (botEditorDraft.preferred_vendor || "").trim())?.models ?? []).map((model) => (
                            <option key={model} value={model}>
                              {model}
                            </option>
                          ))}
                        </select>
                      </label>
                      <label className="space-y-2 md:col-span-2">
                        <span className="text-xs uppercase tracking-widest text-white/50">{t("机器人描述", "Robot description")}</span>
                        <input
                          className="theme-input"
                          autoComplete="off"
                          autoCorrect="off"
                          autoCapitalize="sentences"
                          value={botEditorDraft.description || ""}
                          onChange={(e) => updateBotEditorDraft("description", e.target.value)}
                          placeholder={t("例如 负责客服答疑和日常接待", "For example, handles support questions and daily replies")}
                        />
                      </label>
                      <div className="space-y-3 md:col-span-2 rounded-2xl border border-white/10 bg-black/20 p-4">
                        <div className="flex flex-wrap items-start justify-between gap-3">
                          <div>
                            <p className="text-xs uppercase tracking-widest text-white/50">{t("允许访问类型", "Access type")}</p>
                            <p className="mt-1 text-[11px] text-white/45">
                              {t("公开表示任何 Telegram 用户都能直接问；指定人员表示只有你填在下面的账号才会收到机器人回复。", "Public means any Telegram user can ask directly. Specified means only the accounts listed below can get replies.")}
                            </p>
                          </div>
                          <label className="min-w-[180px] max-w-[220px] space-y-2">
                            <span className="text-[11px] uppercase tracking-widest text-white/40">{t("访问方式", "Mode")}</span>
                            <select
                              className="theme-input"
                              value={botEditorDraft.access_mode || "public"}
                              onChange={(e) => updateBotEditorDraft("access_mode", e.target.value)}
                            >
                              <option value="public">{t("公开", "Public")}</option>
                              <option value="specified">{t("指定人员", "Specified people")}</option>
                            </select>
                          </label>
                        </div>
                        {(botEditorDraft.access_mode || "public") === "specified" ? (
                          <label className="space-y-2">
                            <span className="text-xs uppercase tracking-widest text-white/50">{t("Telegram 账号", "Telegram accounts")}</span>
                            <div className="rounded-2xl border border-white/10 bg-[#0f131c] px-3 py-3">
                              <div className="flex flex-wrap gap-2">
                                {(botEditorDraft.allowed_telegram_usernames ?? []).map((username) => (
                                  <span
                                    key={username}
                                    className="inline-flex items-center gap-2 rounded-full border border-emerald-400/20 bg-emerald-400/10 px-3 py-1.5 text-xs text-emerald-100"
                                  >
                                    <span>@{username}</span>
                                    <button
                                      type="button"
                                      onClick={() => removeBotEditorTelegramUsername(username)}
                                      className="text-emerald-100/70 transition hover:text-emerald-50"
                                      aria-label={`${t("删除账号", "Remove account")} @${username}`}
                                    >
                                      <X className="h-3.5 w-3.5" />
                                    </button>
                                  </span>
                                ))}
                              </div>
                              <input
                                className="mt-3 w-full border-0 bg-transparent px-0 py-0 text-sm text-white outline-none placeholder:text-white/30"
                                autoComplete="off"
                                autoCorrect="off"
                                autoCapitalize="none"
                                spellCheck={false}
                                value={botEditorUsernameInput}
                                onChange={(e) => setBotEditorUsernameInput(e.target.value)}
                                onKeyDown={(e) => {
                                  if (e.key === "Enter" || e.key === "," || e.key === " ") {
                                    e.preventDefault();
                                    addBotEditorTelegramUsername(botEditorUsernameInput);
                                  } else if (e.key === "Backspace" && !botEditorUsernameInput.trim()) {
                                    const last = (botEditorDraft.allowed_telegram_usernames ?? []).at(-1);
                                    if (last) {
                                      removeBotEditorTelegramUsername(last);
                                    }
                                  }
                                }}
                                onBlur={() => addBotEditorTelegramUsername(botEditorUsernameInput)}
                                placeholder={t("输入 @alice 后按回车", "Type @alice and press Enter")}
                              />
                            </div>
                            <p className="text-[11px] leading-5 text-white/45">
                              {t("输入后按回车、逗号或空格就会加入标签。系统会自动去掉 @ 并忽略大小写。管理员和旧 allowlist 里的 ID 也会继续放行。", "Press Enter, comma, or space after typing to add a tag. The system will automatically remove @ and ignore case. Admins and legacy allowlist IDs will also continue to pass.")}
                            </p>
                          </label>
                        ) : null}
                      </div>
                      <label className="space-y-2 md:col-span-2">
                        <div className="flex flex-wrap items-center justify-between gap-2">
                          <span className="text-xs uppercase tracking-widest text-white/50">{t("人设 / 系统提示词", "Persona / system prompt")}</span>
                          <span className="text-[11px] text-white/45">{t("可先套用一个常用预设，再按你的口吻微调。", "Start from a preset, then adjust the tone if needed.")}</span>
                        </div>
                        <div className="flex flex-wrap gap-2">
                          {robotPersonaPresets.map((preset) => (
                            <button
                              key={preset.id}
                              type="button"
                              onClick={() => applyRobotPersonaPreset(preset)}
                              className="rounded-full border border-white/12 bg-white/5 px-3 py-1.5 text-xs text-white/75 transition hover:bg-white/10"
                            >
                              {preset.label}
                            </button>
                          ))}
                        </div>
                        <textarea
                          className="theme-input min-h-[120px]"
                          autoComplete="off"
                          autoCorrect="off"
                          autoCapitalize="sentences"
                          spellCheck={false}
                          name="robot-persona-prompt"
                          value={botEditorDraft.persona_prompt || ""}
                          onChange={(e) => updateBotEditorDraft("persona_prompt", e.target.value)}
                          placeholder={t("告诉 RustClaw 这个 bot 背后的角色是谁、回答风格是什么。", "Describe who this bot is and how it should respond.")}
                        />
                      </label>
                      <div className="space-y-2 md:col-span-2">
                        <div className="flex flex-wrap items-center justify-between gap-2">
                          <span className="text-xs uppercase tracking-widest text-white/50">{t("技能范围", "Skill scope")}</span>
                          <span className="text-[11px] text-white/45">
                            {t("默认建议跟随系统设置，小白用户一般不用自己手填技能名。", "Following the system default is recommended. Most users do not need to type skill names manually.")}
                          </span>
                        </div>
                        <div className="flex flex-wrap gap-2">
                          <button
                            type="button"
                            onClick={() => applyRobotSkillMode("inherit")}
                            className={
                              botEditorSkillMode === "inherit"
                                ? "rounded-full border border-emerald-400/25 bg-emerald-400/10 px-3 py-1.5 text-xs text-emerald-200"
                                : "rounded-full border border-white/12 bg-white/5 px-3 py-1.5 text-xs text-white/75 transition hover:bg-white/10"
                            }
                          >
                            {t("跟随系统默认", "Follow system default")}
                          </button>
                          <button
                            type="button"
                            onClick={() => applyRobotSkillMode("common")}
                            className={
                              botEditorSkillMode === "common"
                                ? "rounded-full border border-emerald-400/25 bg-emerald-400/10 px-3 py-1.5 text-xs text-emerald-200"
                                : "rounded-full border border-white/12 bg-white/5 px-3 py-1.5 text-xs text-white/75 transition hover:bg-white/10"
                            }
                          >
                            {t("常用能力", "Common abilities")}
                          </button>
                          <button
                            type="button"
                            onClick={() => applyRobotSkillMode("custom")}
                            className={
                              botEditorSkillMode === "custom"
                                ? "rounded-full border border-emerald-400/25 bg-emerald-400/10 px-3 py-1.5 text-xs text-emerald-200"
                                : "rounded-full border border-white/12 bg-white/5 px-3 py-1.5 text-xs text-white/75 transition hover:bg-white/10"
                            }
                          >
                            {t("自定义", "Custom")}
                          </button>
                        </div>
                        {botEditorSkillMode === "inherit" ? (
                          <p className="rounded-xl border border-white/10 bg-black/20 px-4 py-3 text-sm text-white/70">
                            {t(
                              "当前会跟随系统里已经启用的技能。留空不是没效果，而是沿用整套系统默认能力。",
                              "This robot will follow the skills already enabled in the system. Leaving it blank does not disable skills; it inherits the system default set.",
                            )}
                          </p>
                        ) : null}
                        {botEditorSkillMode === "common" ? (
                          <div className="rounded-xl border border-white/10 bg-black/20 px-4 py-3">
                            <p className="text-sm text-white/70">
                              {t("当前只保留一组更适合普通对话的常用能力。", "This keeps a smaller set of abilities that fits everyday conversations better.")}
                            </p>
                            <div className="mt-3 flex flex-wrap gap-2">
                              {beginnerRobotSkillPreset.map((skill) => (
                                <span key={skill} className="rounded-full border border-white/10 bg-white/5 px-2.5 py-1 text-[11px] text-white/70">
                                  {skill}
                                </span>
                              ))}
                            </div>
                          </div>
                        ) : null}
                        {botEditorSkillMode === "custom" ? (
                          <div className="rounded-xl border border-white/10 bg-black/20 p-3">
                            <p className="text-sm text-white/70">
                              {t("默认已经全选。把你不想给这个机器人的能力取消勾选就行。", "Everything is selected by default. Simply uncheck the abilities you do not want this robot to use.")}
                            </p>
                            <div className="theme-scrollbar mt-3 max-h-72 overflow-y-auto pr-2">
                              <div className="grid gap-2 sm:grid-cols-2">
                                {managedSkills.map((skill) => {
                                  const enabled = (botEditorDraft.allowed_skills ?? []).includes(skill);
                                  return (
                                    <label
                                      key={skill}
                                      className="flex items-start gap-3 rounded-xl border border-white/10 bg-white/5 px-3 py-3 text-sm text-white/85"
                                    >
                                      <input
                                        type="checkbox"
                                        className="mt-1 h-4 w-4 rounded border-white/20 bg-transparent"
                                        checked={enabled}
                                        onChange={(e) => toggleBotEditorSkill(skill, e.target.checked)}
                                      />
                                      <span className="min-w-0">
                                        <span className="block break-words text-sm font-medium text-white/90">{skill}</span>
                                        <span className="mt-1 block text-[11px] leading-5 text-white/50">{describeSkill(skill)}</span>
                                      </span>
                                    </label>
                                  );
                                })}
                              </div>
                            </div>
                          </div>
                        ) : null}
                      </div>
                    </div>
                    </div>

                    <div className="mt-6 flex flex-wrap items-center justify-between gap-3 border-t border-white/10 pt-5">
                      <p className="text-xs text-white/45">
                        {t("这里不会立刻重启服务。你可以继续改别的机器人，最后再统一重启。", "This will not restart services immediately. You can keep editing other robots and restart once at the end.")}
                      </p>
                      <div className="flex flex-wrap items-center gap-2">
                        <button type="button" onClick={closeBotEditor} className="theme-secondary-btn px-4 py-2 text-xs">
                          {t("取消", "Cancel")}
                        </button>
                        <button
                          type="button"
                          onClick={() => void saveBotEditorDraft()}
                          disabled={!robotChannelSaveSupported(botEditorDraft.channel) || telegramConfigSaving}
                          className="theme-accent-btn px-4 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                        >
                          {telegramConfigSaving ? t("保存中", "Saving") : t("保存到列表", "Save to list")}
                        </button>
                      </div>
                    </div>
                  </div>
                </div>
                </div>
              ) : null}

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
                      onClick={() => void fetchLlmConfig()}
                      disabled={llmConfigLoading}
                      className="theme-topbar-btn px-3 py-2 text-xs font-medium disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {llmConfigLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
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
                    "这里按名称统一列出所有技能，不再强行分类。按钮只是先选择；点击“保存开关”后会提示重启，确认后系统会自动帮你重启并生效。",
                    "All skills are listed together by name here. Buttons only stage your choice first; when you click Save Switches, you will be asked to restart and the system will restart automatically after you confirm.",
                  )}
                </p>

                {(() => {
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
                            ? "flex flex-col gap-3 rounded-lg border border-sky-400/40 bg-sky-500/10 px-3 py-3 text-xs shadow-[0_0_0_1px_rgba(56,189,248,0.18)] sm:flex-row sm:items-start sm:justify-between"
                            : "flex flex-col gap-3 rounded-lg border border-white/10 bg-[#12151f] px-3 py-3 text-xs sm:flex-row sm:items-start sm:justify-between"
                        }
                      >
                        <span className="min-w-0 flex min-h-[8.5rem] flex-1 flex-col self-stretch">
                          <span className="block break-words text-sm text-white/90">{name}</span>
                          <span className="mt-1 block text-[11px] leading-5 text-white/50">{describeSkill(name)}</span>
                          {statusMeta.length > 0 ? (
                            <span className="mt-3 block text-[11px] leading-5 text-white/35">{statusMeta.join(" · ")}</span>
                          ) : null}
                          <span className="mt-auto pt-4">
                            <span
                              className={
                                configuredEnabled
                                  ? "inline-flex items-center gap-2 rounded-full border border-emerald-500/35 bg-emerald-500/12 px-2.5 py-1 text-[11px] font-medium text-emerald-200"
                                  : "inline-flex items-center gap-2 rounded-full border border-amber-500/35 bg-amber-500/12 px-2.5 py-1 text-[11px] font-medium text-amber-200"
                              }
                            >
                              <span
                                className={
                                  configuredEnabled
                                    ? "h-1.5 w-1.5 rounded-full bg-emerald-300"
                                    : "h-1.5 w-1.5 rounded-full bg-amber-300"
                                }
                              />
                              {configuredEnabled ? t("当前已开启", "Currently enabled") : t("当前已关闭", "Currently disabled")}
                            </span>
                            {pendingApply ? (
                              <span className="mt-2 block text-[11px] leading-5 text-amber-200/85">
                                {t("保存后会自动重启生效", "Will apply after save and restart")}
                              </span>
                            ) : null}
                          </span>
                        </span>
                        <span className="flex flex-wrap items-center gap-2 sm:max-w-[55%] sm:justify-end">
                          <button
                            type="button"
                            onClick={() => toggleSkillEnabled(name, !configuredEnabled)}
                            className="rounded border border-white/20 bg-white/5 px-2 py-1 text-[10px] text-white/80 hover:bg-white/10"
                            title={configuredEnabled ? t("先设为关闭，保存后才会真正关闭", "Choose Disable first. It only turns off after you save.") : t("先设为开启，保存后才会真正开启", "Choose Enable first. It only turns on after you save.")}
                          >
                            {configuredEnabled ? t("关闭", "Disable") : isRecentImport ? t("启用这个技能", "Enable this skill") : t("启用", "Enable")}
                          </button>
                          {isExternalSkill ? (
                            <button
                              type="button"
                              onClick={() => void uninstallExternalSkill(name)}
                              disabled={isUninstalling}
                              className="inline-flex items-center gap-1 rounded border border-red-500/25 bg-red-500/10 px-2 py-1 text-[10px] text-red-100 hover:bg-red-500/15 disabled:cursor-not-allowed disabled:opacity-50"
                              title={t("卸载这个外部技能，并删除它导入的文件", "Uninstall this imported skill and delete its files")}
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
                          <h5 className="text-sm font-semibold text-white">{t("全部技能", "All skills")}</h5>
                          <span className="theme-meta-pill !rounded-xl !px-2.5 !py-1 text-[11px]">
                            {filteredManagedSkills.length}/{managedSkills.length}
                          </span>
                        </div>
                        <p className="mt-1 text-xs leading-5 text-white/50">
                          {t(
                            "按名称统一管理。后面导入的新技能也会直接出现在这里。",
                            "All skills are managed together by name. Newly imported skills will also appear here.",
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
                      {filteredManagedSkills.length > 0 ? (
                        <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">{filteredManagedSkills.map(renderSkillRow)}</div>
                      ) : null}
                      {normalizedSkillsSearchQuery && filteredManagedSkills.length === 0 ? (
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

          {currentPage === "chat" ? (
            <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
              <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
                <h3 className="text-base font-semibold">{t("试着和 RustClaw 说一句话", "Try saying one sentence to RustClaw")}</h3>
                <div className="flex flex-wrap items-center gap-3 text-sm">
                  <label className="inline-flex items-center gap-2 text-white/80">
                    <input type="checkbox" checked={chatAgentMode} onChange={(e) => setChatAgentMode(e.target.checked)} />
                    agent_mode
                  </label>
                  <label className="inline-flex items-center gap-2 text-white/80">
                    <input type="checkbox" checked={debugModeEnabled} onChange={(e) => setDebugModeEnabled(e.target.checked)} />
                    {t("调试模式", "Debug mode")}
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

              <div className="h-80 overflow-auto rounded-xl border border-white/10 bg-black/30 p-3 space-y-3">
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
                    </div>
                  </div>
                ))}
              </div>

              <div className="mt-4 grid gap-3 md:grid-cols-[1fr_auto]">
                <textarea
                  className="theme-input min-h-24"
                  placeholder={t("例如：你好，请告诉我你现在能做什么", "For example: Hello, please tell me what you can do right now")}
                  value={chatInput}
                  onChange={(e) => setChatInput(e.target.value)}
                  onKeyDown={handleChatInputKeyDown}
                />
                <button
                  onClick={() => void sendChatMessage()}
                  disabled={chatSending || !chatInput.trim()}
                  className="theme-accent-btn"
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
              {renderTaskDebugPanel()}
            </section>
          ) : null}

          {currentPage === "usage" ? (
            <>
              <section className="theme-panel p-5">
                <div className="flex flex-wrap items-start justify-between gap-4">
                  <div className="max-w-3xl">
                    <p className="theme-kicker text-[11px] uppercase tracking-[0.24em]">{t("使用记录", "Usage history")}</p>
                    <h3 className="mt-2 text-lg font-semibold text-white">{t("每一次真实请求，都能在这里回看", "Review every real request here")}</h3>
                    <p className="mt-2 text-sm leading-6 text-white/60">
                      {t(
                        "这里只保留今天的真实模型请求。列表先看摘要，点开某一条再用弹窗看完整参数和返回。",
                        "This page keeps today's real model requests only. Scan the summary list first, then open any row in a dialog for full parameters and responses.",
                      )}
                    </p>
                  </div>
                  <div className="flex flex-wrap gap-3">
                    <button type="button" onClick={() => void fetchUsageRecords()} disabled={usageRecordsLoading} className="theme-accent-btn">
                      {usageRecordsLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                      {t("刷新记录", "Refresh")}
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setUsageSearchQuery("");
                        setUsageChannelFilter("all");
                        setUsageStatusFilter("all");
                        setUsagePage(1);
                      }}
                      className="theme-secondary-btn px-4 py-2 text-sm"
                    >
                      {t("清空筛选", "Reset filters")}
                    </button>
                  </div>
                </div>

                <div className="mt-5 grid gap-4 xl:grid-cols-4 md:grid-cols-2">
                  <div className="rounded-[24px] border border-white/10 bg-white/6 px-5 py-4">
                    <div className="flex items-center gap-3">
                      <div className="rounded-2xl bg-sky-500/12 p-3 text-sky-200">
                        <FileText className="h-5 w-5" />
                      </div>
                      <div>
                        <p className="text-xs uppercase tracking-widest text-white/40">{t("请求数", "Requests")}</p>
                        <p className="mt-1 text-2xl font-semibold text-white">{formatInteger(usageStats.total_requests, lang === "zh" ? "zh-CN" : "en-US")}</p>
                        <p className="mt-1 text-xs text-white/45">{t("今天符合筛选条件", "Today's matching records")}</p>
                      </div>
                    </div>
                  </div>
                  <div className="rounded-[24px] border border-white/10 bg-white/6 px-5 py-4">
                    <div className="flex items-center gap-3">
                      <div className="rounded-2xl bg-amber-500/12 p-3 text-amber-200">
                        <Sparkles className="h-5 w-5" />
                      </div>
                      <div>
                        <p className="text-xs uppercase tracking-widest text-white/40">{t("总 Tokens", "Total tokens")}</p>
                        <p className="mt-1 text-2xl font-semibold text-white">{formatCompactInteger(usageStats.total_tokens, lang === "zh" ? "zh-CN" : "en-US")}</p>
                        <p className="mt-1 text-xs text-white/45">
                          {t("输入", "Prompt")} {formatCompactInteger(usageStats.prompt_tokens, lang === "zh" ? "zh-CN" : "en-US")} · {t("输出", "Completion")} {formatCompactInteger(usageStats.completion_tokens, lang === "zh" ? "zh-CN" : "en-US")}
                        </p>
                      </div>
                    </div>
                  </div>
                  <div className="rounded-[24px] border border-white/10 bg-white/6 px-5 py-4">
                    <div className="flex items-center gap-3">
                      <div className="rounded-2xl bg-emerald-500/12 p-3 text-emerald-200">
                        <MessageCircle className="h-5 w-5" />
                      </div>
                      <div>
                        <p className="text-xs uppercase tracking-widest text-white/40">{t("成功请求", "Successful requests")}</p>
                        <p className="mt-1 text-2xl font-semibold text-white">{formatInteger(usageStats.success_requests, lang === "zh" ? "zh-CN" : "en-US")}</p>
                        <p className="mt-1 text-xs text-white/45">{t("模型返回成功", "Model returned successfully")}</p>
                      </div>
                    </div>
                  </div>
                  <div className="rounded-[24px] border border-white/10 bg-white/6 px-5 py-4">
                    <div className="flex items-center gap-3">
                      <div className="rounded-2xl bg-rose-500/12 p-3 text-rose-200">
                        <AlertCircle className="h-5 w-5" />
                      </div>
                      <div>
                        <p className="text-xs uppercase tracking-widest text-white/40">{t("失败请求", "Failed requests")}</p>
                        <p className="mt-1 text-2xl font-semibold text-white">{formatInteger(usageStats.failed_requests, lang === "zh" ? "zh-CN" : "en-US")}</p>
                        <p className="mt-1 text-xs text-white/45">{t("包含报错或异常返回", "Includes errors or abnormal responses")}</p>
                      </div>
                    </div>
                  </div>
                </div>

                <div className="mt-5 grid gap-3 xl:grid-cols-[minmax(0,1.4fr)_minmax(240px,0.8fr)_minmax(200px,0.7fr)_auto]">
                  <label className="space-y-2">
                    <span className="text-[10px] uppercase tracking-widest text-white/45">{t("按内容查找", "Search")}</span>
                    <input
                      className="theme-input"
                      value={usageSearchQuery}
                      onChange={(e) => {
                        setUsageSearchQuery(e.target.value);
                        setUsagePage(1);
                      }}
                      placeholder={t("搜 task_id、消息内容、模型名、机器人名", "Search task_id, message, model, or robot name")}
                    />
                  </label>
                  <label className="space-y-2">
                    <span className="text-[10px] uppercase tracking-widest text-white/45">{t("渠道", "Channel")}</span>
                    <select className="theme-input" value={usageChannelFilter} onChange={(e) => {
                      setUsageChannelFilter(e.target.value);
                      setUsagePage(1);
                    }}>
                      {usageChannelOptions.map((value) => (
                        <option key={value} value={value}>
                          {value === "all" ? t("全部渠道", "All channels") : usageChannelLabel(value)}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="space-y-2">
                    <span className="text-[10px] uppercase tracking-widest text-white/45">{t("结果", "Result")}</span>
                    <select className="theme-input" value={usageStatusFilter} onChange={(e) => {
                      setUsageStatusFilter(e.target.value);
                      setUsagePage(1);
                    }}>
                      <option value="all">{t("全部结果", "All results")}</option>
                      <option value="success">{t("只看成功", "Success only")}</option>
                      <option value="failed">{t("只看失败", "Failed only")}</option>
                    </select>
                  </label>
                  <div className="flex items-end text-xs text-white/45">
                    {usageRecordsData
                      ? t(
                          `这里只显示今天的记录，每页 ${usagePagination?.page_size ?? 20} 条。`,
                          `Only today's records are shown here, ${usagePagination?.page_size ?? 20} per page.`,
                        )
                      : t("刷新后会显示今天的请求记录。", "Refresh to load today's request history.")}
                  </div>
                </div>
              </section>

              {usageRecordsError ? (
                <section className="rounded-2xl border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
                  {usageRecordsError}
                </section>
              ) : null}

              <section className="theme-panel overflow-hidden">
                  <div className="border-b border-white/10 px-5 py-4">
                    <h3 className="text-base font-semibold text-white">{t("请求列表", "Request list")}</h3>
                    <p className="mt-1 text-sm text-white/55">
                      {t(
                        "一行就是一条完整请求。先看消息内容、最终状态和总消耗，点开后再看整条模型调用链路。",
                        "Each row is one full request. Start with the message, final status, and total usage, then open it to inspect the full model-call chain.",
                      )}
                    </p>
                  </div>

                  {!usageRecordsLoading && usageRecords.length === 0 ? (
                    <div className="px-5 py-10 text-center text-sm leading-7 text-white/60">
                      {usageRecordsData
                        ? t("当前筛选条件下还没有记录。你可以放宽筛选条件，或者先给机器人发一条消息。", "No records match the current filters. Try broader filters or send a message to your robot first.")
                        : t("还没有加载到请求记录。点上面的刷新按钮即可读取。", "No request history has been loaded yet. Use the refresh button above to fetch it.")}
                    </div>
                  ) : null}

                  {usageRecords.length > 0 ? (
                    <>
                    <div className="theme-scrollbar max-h-[900px] overflow-auto">
                      <div className="hidden grid-cols-[170px_minmax(0,1.8fr)_160px_132px_150px] gap-3 border-b border-white/10 bg-black/15 px-5 py-3 text-[11px] uppercase tracking-[0.22em] text-white/40 lg:grid">
                        <span>{t("时间 / 渠道", "Time / Channel")}</span>
                        <span>{t("请求内容", "Request")}</span>
                        <span>{t("模型", "Model")}</span>
                        <span>{t("Tokens", "Tokens")}</span>
                        <span>{t("状态 / 操作", "Status / Action")}</span>
                      </div>

                      {usageRecords.map((record) => {
                        return (
                          <button
                            key={record.record_id}
                            type="button"
                            onClick={() => setSelectedUsageRecordId(record.record_id)}
                            className="grid w-full gap-3 border-b border-white/6 px-5 py-4 text-left transition hover:bg-white/6 lg:grid-cols-[170px_minmax(0,1.8fr)_160px_132px_150px]"
                          >
                            <div className="space-y-1">
                              <p className="text-sm text-white/82">{record.ts ? toLocalDateTime(record.ts * 1000) : "--"}</p>
                              <div className="flex flex-wrap items-center gap-2 text-xs text-white/50">
                                <span className="rounded-full border border-white/10 bg-white/5 px-2 py-1">{usageChannelLabel(record.channel)}</span>
                                {record.telegram_bot_name ? <span>{record.telegram_bot_name}</span> : null}
                              </div>
                            </div>
                            <div className="min-w-0">
                              <p className="line-clamp-2 text-sm font-medium leading-6 text-white">{record.request_text || t("这条请求没有记录到用户原文。", "This request has no captured user text.")}</p>
                              <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-white/45">
                                <span>task_id: <span className="font-mono text-white/65">{record.task_id}</span></span>
                                {record.external_chat_id ? <span>{t("会话", "Chat")}: {record.external_chat_id}</span> : null}
                                {record.external_user_id ? <span>{t("用户", "User")}: {record.external_user_id}</span> : null}
                                <span>{t("链路", "Chain")}: {record.llm_call_count}</span>
                              </div>
                            </div>
                            <div className="space-y-1">
                              <p className="text-sm text-white/82">{record.model || "--"}</p>
                              <p className="text-xs text-white/45">{record.provider || record.vendor || "--"}</p>
                            </div>
                            <div className="space-y-1">
                              <p className="text-sm text-white/82">{formatCompactInteger(record.total_tokens, lang === "zh" ? "zh-CN" : "en-US")}</p>
                              <p className="text-xs text-white/45">
                                {t("入", "In")} {formatCompactInteger(record.prompt_tokens, lang === "zh" ? "zh-CN" : "en-US")} · {t("出", "Out")} {formatCompactInteger(record.completion_tokens, lang === "zh" ? "zh-CN" : "en-US")}
                              </p>
                            </div>
                            <div className="space-y-2">
                              <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${record.status === "ok" ? "border-emerald-400/20 bg-emerald-400/10 text-emerald-100" : "border-rose-400/20 bg-rose-400/10 text-rose-100"}`}>
                                {usageStatusLabel(record.status)}
                              </span>
                              <p className="line-clamp-2 text-xs text-white/45">{record.error || record.prompt_file || "--"}</p>
                              <p className="text-xs font-medium text-[#ffb08a]">{t("点击查看完整链路", "Click to open full chain")}</p>
                            </div>
                          </button>
                        );
                      })}
                    </div>
                    <div className="flex flex-wrap items-center justify-between gap-3 border-t border-white/10 px-5 py-4">
                      <p className="text-sm text-white/55">
                        {usagePagination && usagePagination.total_records > 0
                          ? t(
                              `今天共 ${usagePagination.total_records} 条，当前第 ${usagePagination.page} / ${Math.max(usagePagination.total_pages, 1)} 页。`,
                              `${usagePagination.total_records} records today, page ${usagePagination.page} of ${Math.max(usagePagination.total_pages, 1)}.`,
                            )
                          : t("今天还没有可展示的记录。", "There are no records to show today.")}
                      </p>
                      <div className="flex items-center gap-2">
                        <button
                          type="button"
                          onClick={() => setUsagePage((value) => Math.max(1, value - 1))}
                          disabled={usageRecordsLoading || (usagePagination?.page ?? 1) <= 1}
                          className="theme-secondary-btn px-4 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-50"
                        >
                          {t("上一页", "Previous")}
                        </button>
                        <span className="rounded-xl border border-white/10 bg-white/5 px-3 py-2 text-sm text-white/70">
                          {(usagePagination?.page ?? 1)} / {Math.max(usagePagination?.total_pages ?? 1, 1)}
                        </span>
                        <button
                          type="button"
                          onClick={() => setUsagePage((value) => value + 1)}
                          disabled={usageRecordsLoading || (usagePagination?.page ?? 1) >= (usagePagination?.total_pages ?? 1)}
                          className="theme-secondary-btn px-4 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-50"
                        >
                          {t("下一页", "Next")}
                        </button>
                      </div>
                    </div>
                    </>
                  ) : null}
              </section>
              {renderUsageRecordModal()}
            </>
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
                    <option value="channel-gateway.log">channel-gateway.log</option>
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
                    {t("先去对话测试", "Open Chat Test")}
                  </button>
                  <button type="button" onClick={() => setCurrentPage("channels")} className="theme-accent-soft-btn">
                    <Database className="h-4 w-4" />
                    {t("先去机器人设置", "Open Robot Settings")}
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
                  {interactionChannel === "telegram" ? (
                    <label className="space-y-2">
                      <span className="text-xs uppercase tracking-widest text-white/50">{t("Telegram 机器人（必选）", "Telegram robot (required)")}</span>
                      <select
                        className="theme-input"
                        value={interactionTelegramBotName}
                        onChange={(e) => setInteractionTelegramBotName(e.target.value)}
                      >
                        <option value="">{t("请选择要发回的机器人", "Choose the robot that must reply")}</option>
                        {(health?.telegram_configured_bot_names ?? []).map((name) => (
                          <option key={name} value={name}>
                            {name}
                          </option>
                        ))}
                      </select>
                    </label>
                  ) : null}
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

            </>
          ) : null}
        </main>
      </div>
    </div>
  );
}
