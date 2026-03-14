import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertCircle,
  BellRing,
  CheckCircle2,
  Clock3,
  Database,
  FileText,
  Loader2,
  MessageCircle,
  RefreshCw,
  Server,
  Settings,
  Timer,
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
  effective_enabled_skills_preview: string[];
  runtime_enabled_skills: string[];
  restart_required: boolean;
}

interface ModelConfigItem {
  vendor: string;
  model: string;
}

interface ModelConfigResponse {
  llm: ModelConfigItem;
  image_edit: ModelConfigItem;
  image_generation: ModelConfigItem;
  image_vision: ModelConfigItem;
  audio_transcribe: ModelConfigItem;
  audio_synthesize: ModelConfigItem;
  restart_required?: boolean;
}

/** 主模型 config.toml [llm.*]；图像 image.toml [image_*].providers.*；声音 audio.toml [audio_*].providers.* */
interface ProviderKeysResponse {
  llm?: Record<string, string>;
  image?: Record<string, Record<string, string>>;
  audio?: Record<string, Record<string, string>>;
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

interface Snapshot {
  ts: number;
  queue: number;
  running: number;
  memory: number | null;
}

interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  text: string;
  ts: number;
}

interface AdapterHealthRow {
  key: string;
  label: string;
  serviceName: "telegramd" | "whatsappd" | "whatsapp_webd" | "feishud" | "larkd";
  healthy: boolean | null | undefined;
  processCount: number | null | undefined;
  memoryRssBytes: number | null | undefined;
}

const UI_HIDDEN_SKILLS = new Set<string>(["chat"]);
const IMAGE_SKILLS = new Set<string>(["image_vision", "image_generate", "image_edit"]);
const AUDIO_SKILLS = new Set<string>(["audio_transcribe", "audio_synthesize"]);
/** 基本技能（与后端 base_skill_names 一致，由 tool 转换），API 未返回时用此兜底 */
const FALLBACK_BASE_SKILL_NAMES = ["run_cmd", "read_file", "write_file", "list_dir", "make_dir", "remove_file"];

const STORAGE_KEYS = {
  baseUrl: "rustclaw.monitor.baseUrl",
  userKey: "rustclaw.monitor.userKey",
  polling: "rustclaw.monitor.pollingSeconds",
  queueWarn: "rustclaw.monitor.queueWarn",
  ageWarn: "rustclaw.monitor.ageWarnSeconds",
  lang: "rustclaw.monitor.lang",
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

function StatCard({
  title,
  value,
  hint,
}: {
  title: string;
  value: string | number;
  hint?: string;
}) {
  return (
    <div className="rounded-xl border border-white/10 bg-white/5 p-3 sm:rounded-2xl sm:p-5">
      <p className="text-[10px] uppercase tracking-widest text-white/50 sm:text-xs">{title}</p>
      <p className="mt-1 text-lg font-bold text-white sm:mt-2 sm:text-2xl">{value}</p>
      {hint ? <p className="mt-0.5 text-[10px] text-white/50 sm:mt-1 sm:text-xs">{hint}</p> : null}
    </div>
  );
}

export default function App() {
  const [lang, setLang] = useState<"zh" | "en">(() => {
    const saved = window.localStorage.getItem(STORAGE_KEYS.lang);
    return saved === "en" ? "en" : "zh";
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
  const [lastUpdated, setLastUpdated] = useState<number | null>(null);
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
  const [skillsLoading, setSkillsLoading] = useState(false);
  const [skillsError, setSkillsError] = useState<string | null>(null);
  const [skillsData, setSkillsData] = useState<SkillsResponse | null>(null);
  const [skillsConfigLoading, setSkillsConfigLoading] = useState(false);
  const [skillsConfigError, setSkillsConfigError] = useState<string | null>(null);
  const [skillsConfigData, setSkillsConfigData] = useState<SkillsConfigResponse | null>(null);
  const [skillSwitchDraft, setSkillSwitchDraft] = useState<Record<string, boolean>>({});
  const [skillSwitchSaving, setSkillSwitchSaving] = useState(false);
  const [skillSwitchSaveMessage, setSkillSwitchSaveMessage] = useState<string | null>(null);

  const [modelConfigLoading, setModelConfigLoading] = useState(false);
  const [modelConfigError, setModelConfigError] = useState<string | null>(null);
  const [modelConfigData, setModelConfigData] = useState<ModelConfigResponse | null>(null);
  const [modelConfigDraft, setModelConfigDraft] = useState<ModelConfigResponse | null>(null);
  const [modelConfigSaving, setModelConfigSaving] = useState(false);
  const [modelConfigSaveMessage, setModelConfigSaveMessage] = useState<string | null>(null);

  const [providerKeysLoading, setProviderKeysLoading] = useState(false);
  const [providerKeysError, setProviderKeysError] = useState<string | null>(null);
  const [providerKeysData, setProviderKeysData] = useState<ProviderKeysResponse>({});
  const [providerKeysDraft, setProviderKeysDraft] = useState<ProviderKeysResponse>({});
  const [providerKeysSaving, setProviderKeysSaving] = useState<"llm" | "image" | "audio" | null>(null);
  const [providerKeysSaveMessage, setProviderKeysSaveMessage] = useState<string | null>(null);

  const [restartClawdLoading, setRestartClawdLoading] = useState(false);
  const [restartClawdMessage, setRestartClawdMessage] = useState<string | null>(null);

  const [taskId, setTaskId] = useState("");
  const [taskLoading, setTaskLoading] = useState(false);
  const [taskResult, setTaskResult] = useState<TaskQueryResponse | null>(null);
  const [taskError, setTaskError] = useState<string | null>(null);
  const [trackingTaskId, setTrackingTaskId] = useState<string | null>(null);

  const [interactionKind, setInteractionKind] = useState<"ask" | "run_skill">("ask");
  const [interactionChannel, setInteractionChannel] = useState<"ui" | "telegram" | "whatsapp" | "feishu">("ui");
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
  const [chatAgentMode, setChatAgentMode] = useState(true);
  const [chatSending, setChatSending] = useState(false);
  const [chatError, setChatError] = useState<string | null>(null);
  const [chatDialogOpen, setChatDialogOpen] = useState(false);
  const [serviceActionLoading, setServiceActionLoading] = useState<Record<string, boolean>>({});
  const [serviceActionMessage, setServiceActionMessage] = useState<string | null>(null);
  const [waLoginDialogOpen, setWaLoginDialogOpen] = useState(false);
  const [waLoginLoading, setWaLoginLoading] = useState(false);
  const [waLoginError, setWaLoginError] = useState<string | null>(null);
  const [waLoginStatus, setWaLoginStatus] = useState<WhatsappWebLoginStatus | null>(null);
  const [waLogoutLoading, setWaLogoutLoading] = useState(false);
  const [viewMode, setViewMode] = useState<"dashboard" | "logs" | "config">("dashboard");
  const [selectedLogFile, setSelectedLogFile] = useState("clawd.log");
  const [logTailLines, setLogTailLines] = useState(200);
  const [logLoading, setLogLoading] = useState(false);
  const [logError, setLogError] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  const [logLastUpdated, setLogLastUpdated] = useState<number | null>(null);
  const [logFollowTail, setLogFollowTail] = useState(true);
  const logContainerRef = useRef<HTMLPreElement | null>(null);

  const openChatPanel = () => {
    setViewMode("dashboard");
    setChatDialogOpen(true);
  };

  const t = (zh: string, en: string) => (lang === "zh" ? zh : en);
  const tSlash = (mixed: string) => {
    const [zh, en] = mixed.split(" / ");
    return lang === "zh" ? zh : en ?? zh;
  };

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
      const res = await fetch(`${apiBase.replace(/\/$/, "")}/v1/auth/ui-key/verify`, {
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
      applyIdentity(body.data);
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
      setLastUpdated(Date.now());
      setSnapshots((prev) => {
        const next: Snapshot[] = [
          ...prev,
          {
            ts: Date.now(),
            queue: body.data.queue_length,
            running: body.data.running_length,
            memory: body.data.memory_rss_bytes ?? null,
          },
        ];
        return next.slice(-24);
      });
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
      setSkillSwitchSaveMessage(
        t(
          "技能开关已保存到 config.toml（需重启 clawd 生效）",
          "Skill switches saved to config.toml (restart clawd to apply)",
        ),
      );
      await fetchSkillsConfig();
      await fetchSkills();
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setSkillsConfigError(message);
    } finally {
      setSkillSwitchSaving(false);
    }
  };

  const fetchModelConfig = async () => {
    setModelConfigLoading(true);
    setModelConfigError(null);
    try {
      const res = await apiFetch(`/v1/admin/model-config`);
      const body = (await res.json()) as ApiResponse<ModelConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `模型配置获取失败 (${res.status})`);
      }
      setModelConfigData(body.data);
      setModelConfigDraft(body.data);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setModelConfigError(message);
    } finally {
      setModelConfigLoading(false);
    }
  };

  const saveModelConfig = async () => {
    if (!modelConfigDraft) return;
    setModelConfigSaving(true);
    setModelConfigSaveMessage(null);
    setModelConfigError(null);
    try {
      const res = await apiFetch(`/v1/admin/model-config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          llm: modelConfigDraft.llm,
          image_edit: modelConfigDraft.image_edit,
          image_generation: modelConfigDraft.image_generation,
          image_vision: modelConfigDraft.image_vision,
          audio_transcribe: modelConfigDraft.audio_transcribe,
          audio_synthesize: modelConfigDraft.audio_synthesize,
        }),
      });
      const body = (await res.json()) as ApiResponse<ModelConfigResponse>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `模型配置保存失败 (${res.status})`);
      }
      setModelConfigData(body.data ?? modelConfigDraft);
      setModelConfigDraft(body.data ?? modelConfigDraft);
      setModelConfigSaveMessage(
        body.data?.restart_required
          ? t("保存成功，需重启相关服务后生效。", "Saved. Restart required to take effect.")
          : t("保存成功", "Saved successfully"),
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setModelConfigError(message);
    } finally {
      setModelConfigSaving(false);
    }
  };

  const parseJsonOrThrow = async (res: Response, context: string): Promise<ApiResponse<ProviderKeysResponse>> => {
    const text = await res.text();
    const trimmed = text.trim().toLowerCase();
    if (trimmed.startsWith("<!doctype") || trimmed.startsWith("<html")) {
      throw new Error(
        t(
          "接口返回了 HTML 而非 JSON，请确认「clawd API 地址」指向已启动的 clawd（例如 http://主机:8787），若通过 nginx 访问需将 /v1 代理到 clawd。",
          "API returned HTML instead of JSON. Ensure «clawd API URL» points to running clawd (e.g. http://host:8787), or proxy /v1 to clawd if using nginx.",
        ),
      );
    }
    try {
      return JSON.parse(text) as ApiResponse<ProviderKeysResponse>;
    } catch {
      throw new Error(`${context}: ${text.slice(0, 80)}${text.length > 80 ? "…" : ""}`);
    }
  };

  const fetchProviderKeys = async () => {
    setProviderKeysLoading(true);
    setProviderKeysError(null);
    try {
      const res = await apiFetch(`/v1/admin/provider-keys`);
      const body = await parseJsonOrThrow(res, "API Key 列表");
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `API Key 列表获取失败 (${res.status})`);
      }
      const data = {
        llm: body.data.llm ?? {},
        image: body.data.image ?? {},
        audio: body.data.audio ?? {},
      };
      setProviderKeysData(data);
      setProviderKeysDraft(data);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setProviderKeysError(message);
    } finally {
      setProviderKeysLoading(false);
    }
  };

  const restartClawd = async () => {
    setRestartClawdLoading(true);
    setRestartClawdMessage(null);
    try {
      const res = await apiFetch(`/v1/admin/restart-clawd`, { method: "POST" });
      const body = (await res.json()) as ApiResponse<{ message?: string; restart_triggered?: boolean }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `重启失败 (${res.status})`);
      }
      setRestartClawdMessage(
        t("已触发重启，clawd 将在数秒后重启，页面可能断开。", "Restart triggered; clawd will restart in a few seconds; page may disconnect."),
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setRestartClawdMessage(`${t("重启失败", "Restart failed")}: ${message}`);
    } finally {
      setRestartClawdLoading(false);
    }
  };

  const saveProviderKeysGroup = async (group: "llm" | "image" | "audio") => {
    setProviderKeysSaving(group);
    setProviderKeysSaveMessage(null);
    setProviderKeysError(null);
    try {
      const toSend: ProviderKeysResponse = {};
      if (group === "llm") {
        const dl = providerKeysDraft.llm ?? {};
        const da = providerKeysData.llm ?? {};
        const out: Record<string, string> = {};
        for (const [vendor, value] of Object.entries(dl)) {
          if (value.trim() === "") continue;
          if (value !== (da[vendor] ?? "")) out[vendor] = value;
        }
        toSend.llm = out;
      } else if (group === "image") {
        const il = providerKeysDraft.image ?? {};
        const ia = providerKeysData.image ?? {};
        const out: Record<string, Record<string, string>> = {};
        for (const [section, vendors] of Object.entries(il)) {
          const changed: Record<string, string> = {};
          for (const [vendor, value] of Object.entries(vendors)) {
            if (value.trim() === "") continue;
            if (value !== ((ia[section] ?? {})[vendor] ?? "")) changed[vendor] = value;
          }
          if (Object.keys(changed).length > 0) out[section] = changed;
        }
        toSend.image = out;
      } else {
        const al = providerKeysDraft.audio ?? {};
        const aa = providerKeysData.audio ?? {};
        const out: Record<string, Record<string, string>> = {};
        for (const [section, vendors] of Object.entries(al)) {
          const changed: Record<string, string> = {};
          for (const [vendor, value] of Object.entries(vendors)) {
            if (value.trim() === "") continue;
            if (value !== ((aa[section] ?? {})[vendor] ?? "")) changed[vendor] = value;
          }
          if (Object.keys(changed).length > 0) out[section] = changed;
        }
        toSend.audio = out;
      }
      const res = await apiFetch(`/v1/admin/provider-keys`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(toSend),
      });
      const body = await parseJsonOrThrow(res, "API Key 保存");
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `API Key 保存失败 (${res.status})`);
      }
      setProviderKeysData((prev) => ({
        ...prev,
        llm: body.data?.llm ?? prev.llm ?? {},
        image: body.data?.image ?? prev.image ?? {},
        audio: body.data?.audio ?? prev.audio ?? {},
      }));
      setProviderKeysDraft((prev) => ({
        ...prev,
        llm: body.data?.llm ?? prev.llm ?? {},
        image: body.data?.image ?? prev.image ?? {},
        audio: body.data?.audio ?? prev.audio ?? {},
      }));
      const msg =
        group === "llm"
          ? t("主模型 API Key 已写入 config.toml，需重启后生效。", "LLM API keys saved to config.toml; restart to apply.")
          : group === "image"
            ? t("图像 API Key 已写入 image.toml，需重启后生效。", "Image API keys saved to image.toml; restart to apply.")
            : t("声音 API Key 已写入 audio.toml，需重启后生效。", "Audio API keys saved to audio.toml; restart to apply.");
      setProviderKeysSaveMessage(msg);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setProviderKeysError(message);
    } finally {
      setProviderKeysSaving(null);
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
      const adapterName = interactionAdapter.trim();
      if (adapterName) {
        chatPayload.adapter = adapterName;
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
    if (!uiAuthReady || pollingSeconds <= 0) return;
    void fetchHealth();
    void fetchSkills();
    void fetchSkillsConfig();
    void fetchModelConfig();
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
    if (!uiAuthReady) return;
    void fetchSkills();
    void fetchSkillsConfig();
    void fetchModelConfig();
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
    if (viewMode === "config") {
      void fetchModelConfig();
      void fetchSkillsConfig();
      void fetchProviderKeys();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewMode, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (viewMode !== "logs") return;
    void fetchLatestLog();
    const timer = window.setInterval(() => {
      void fetchLatestLog();
    }, Math.max(2, pollingSeconds) * 1000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewMode, apiBase, selectedLogFile, logTailLines, pollingSeconds, uiAuthReady]);

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

  const timeline = useMemo(() => snapshots.slice().reverse(), [snapshots]);
  const maskedSavedUiKey = useMemo(() => maskStoredKey(uiKey), [uiKey]);
  const memoryMax = useMemo(() => {
    const values = snapshots.map((s) => s.memory ?? 0);
    return Math.max(1, ...values);
  }, [snapshots]);
  const adapterHealthRows = useMemo<AdapterHealthRow[]>(() => {
    const rows: AdapterHealthRow[] = [
      {
        key: "telegram_bot",
        label: "telegram_bot",
        serviceName: "telegramd",
        healthy: health?.telegram_bot_healthy ?? health?.telegramd_healthy,
        processCount: health?.telegram_bot_process_count ?? health?.telegramd_process_count,
        memoryRssBytes: health?.telegram_bot_memory_rss_bytes ?? health?.telegramd_memory_rss_bytes,
      },
      {
        key: "whatsapp_web",
        label: "whatsapp_web",
        serviceName: "whatsapp_webd",
        healthy: health?.whatsapp_web_healthy,
        processCount: health?.whatsapp_web_process_count,
        memoryRssBytes: health?.whatsapp_web_memory_rss_bytes,
      },
      {
        key: "whatsapp_cloud",
        label: "whatsapp_cloud",
        serviceName: "whatsappd",
        healthy: health?.whatsapp_cloud_healthy ?? health?.whatsappd_healthy,
        processCount: health?.whatsapp_cloud_process_count ?? health?.whatsappd_process_count,
        memoryRssBytes: health?.whatsapp_cloud_memory_rss_bytes ?? health?.whatsappd_memory_rss_bytes,
      },
      {
        key: "feishu_bot",
        label: "feishu_bot",
        serviceName: "feishud",
        healthy: health?.feishud_healthy,
        processCount: health?.feishud_process_count,
        memoryRssBytes: health?.feishud_memory_rss_bytes,
      },
      {
        key: "lark_bot",
        label: "lark_bot",
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
  }, [health]);
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
  const baseSkillsList = useMemo(
    () => managedSkills.filter((n) => baseSkillNamesSet.has(n)),
    [managedSkills, baseSkillNamesSet],
  );
  const imageSkillsList = useMemo(() => managedSkills.filter((n) => IMAGE_SKILLS.has(n)), [managedSkills]);
  const audioSkillsList = useMemo(() => managedSkills.filter((n) => AUDIO_SKILLS.has(n)), [managedSkills]);
  const otherSkillsList = useMemo(
    () =>
      managedSkills.filter(
        (n) => !baseSkillNamesSet.has(n) && !IMAGE_SKILLS.has(n) && !AUDIO_SKILLS.has(n),
      ),
    [managedSkills, baseSkillNamesSet],
  );
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
  const visibleRuntimeSkills = useMemo(
    () => (skillsData?.skills ?? []).filter((name) => !UI_HIDDEN_SKILLS.has(name)),
    [skillsData],
  );

  if (!uiAuthReady) {
    return (
      <div className="min-h-screen bg-[#0f1116] px-4 py-8 text-white">
        <div className="mx-auto max-w-xl rounded-2xl border border-white/10 bg-white/5 p-6 shadow-2xl">
          <div className="mb-6">
            <h1 className="flex items-center gap-2 text-2xl font-bold">
              <span>🦞</span>
              <span>{t("RustClaw Key 登录", "RustClaw Key Sign In")}</span>
            </h1>
            <p className="mt-2 text-sm text-white/60">
              {t("UI 现在必须先输入有效 key 才能访问控制台。", "The console now requires a valid key before access.")}
            </p>
          </div>

          <div className="space-y-4">
            <label className="block space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">
                {t("clawd API 地址", "clawd API URL")}
              </span>
              <input
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder="http://127.0.0.1:8787"
              />
            </label>

            <label className="block space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">{t("访问 Key", "Access Key")}</span>
              <input
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={uiKeyDraft}
                onChange={(e) => setUiKeyDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void verifyUiKey(uiKeyDraft);
                  }
                }}
                placeholder={t("输入已录入数据库的 key", "Enter a key already added to the database")}
              />
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

            <div className="flex items-center gap-3">
              <button
                onClick={() => void verifyUiKey(uiKeyDraft)}
                disabled={uiAuthLoading}
                className="inline-flex items-center gap-2 rounded-xl bg-[#f74c00] px-4 py-2 text-sm font-medium text-white transition hover:bg-[#ff6420] disabled:cursor-not-allowed disabled:opacity-60"
              >
                {uiAuthLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                {t("登录", "Sign in")}
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
                onClick={() => setLang((v) => (v === "zh" ? "en" : "zh"))}
                className="rounded-xl border border-white/15 bg-white/5 px-3 py-2 text-xs hover:bg-white/10"
              >
                {lang === "zh" ? "中文" : "EN"}
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-[#0f1116] text-white selection:bg-[#f74c00]/30">
      <header className="sticky top-0 z-40 border-b border-white/10 bg-[#0f1116]/90 backdrop-blur px-3 py-2 sm:px-6 sm:py-4">
        <div className="mx-auto flex max-w-7xl flex-wrap items-center justify-between gap-2 sm:gap-4">
          <div className="min-w-0">
            <h1 className="text-lg font-bold tracking-tight flex items-center gap-1.5 sm:text-2xl sm:gap-2">
              <span className="text-lg leading-none sm:text-2xl">🦞</span>
              <span className="truncate">{t("RustClaw 后台", "RustClaw Console")}</span>
            </h1>
            <p className="mt-0.5 text-xs text-white/60 sm:mt-1 sm:text-sm">
              {t("实时查看 clawd 健康状态、任务队列与服务运行信息", "Monitor clawd health, queue and runtime in real time")}
            </p>
          </div>

          <div className="flex items-center gap-1.5 sm:gap-3 shrink-0">
            <div className="flex items-center gap-1.5 rounded-lg border border-white/10 bg-white/5 px-2 py-1 sm:gap-3 sm:rounded-xl sm:px-4 sm:py-2">
              {isOnline ? (
                <CheckCircle2 className="h-3.5 w-3.5 text-emerald-400 sm:h-4 sm:w-4" />
              ) : (
                <AlertCircle className="h-3.5 w-3.5 text-red-400 sm:h-4 sm:w-4" />
              )}
              <span className="text-xs sm:text-sm">{isOnline ? t("在线", "Online") : t("离线异常", "Offline")}</span>
              {lastUpdated ? (
                <span className="hidden text-[10px] text-white/50 sm:inline sm:text-xs">{lang === "zh" ? `更新于 ${toLocalTime(lastUpdated)}` : `Updated ${toLocalTime(lastUpdated)}`}</span>
              ) : null}
            </div>
            <button
              type="button"
              onClick={logout}
              className="rounded-lg border border-white/15 bg-white/5 px-2 py-1 text-xs text-white/80 hover:bg-white/10 sm:rounded-xl sm:px-3 sm:py-2 sm:text-sm"
              title={t("退出登录，需重新输入 key", "Log out; key required to sign in again")}
            >
              {t("退出", "Log out")}
            </button>

            <button
              onClick={() => setLang((v) => (v === "zh" ? "en" : "zh"))}
              className="rounded-lg border border-white/15 bg-white/5 px-2 py-1 text-[10px] hover:bg-white/10 sm:rounded-xl sm:px-3 sm:py-2 sm:text-xs"
              title={t("切换语言", "Switch Language")}
            >
              {lang === "zh" ? "中文" : "EN"}
            </button>

            <button
              onClick={() => setViewMode((v) => (v === "dashboard" ? "logs" : "dashboard"))}
              className="relative inline-flex items-center gap-1 rounded-lg border border-sky-400/30 bg-sky-500/15 px-2 py-1 text-xs text-white hover:bg-sky-500/25 transition sm:gap-2 sm:rounded-xl sm:px-3 sm:py-2 sm:text-sm"
              title={t("查看日志页面", "Open Logs Page")}
            >
              <FileText className="h-3.5 w-3.5 sm:h-4 sm:w-4" />
              <span className="hidden sm:inline">{viewMode === "logs" ? t("控制台", "Console") : t("日志", "Logs")}</span>
            </button>
            <button
              onClick={() => setViewMode("config")}
              className={`relative inline-flex items-center gap-1 rounded-lg border px-2 py-1 text-xs transition sm:gap-2 sm:rounded-xl sm:px-3 sm:py-2 sm:text-sm ${
                viewMode === "config"
                  ? "border-amber-400/50 bg-amber-500/25 text-white"
                  : "border-white/20 bg-white/10 text-white hover:bg-white/15"
              }`}
              title={t("配置（模型、技能、API Key）", "Config (models, skills, API keys)")}
            >
              <Settings className="h-3.5 w-3.5 sm:h-4 sm:w-4" />
              <span className="hidden sm:inline">{tSlash("配置 / Config")}</span>
            </button>

            <button
              onClick={() => {
                if (viewMode === "logs") {
                  openChatPanel();
                  return;
                }
                setChatDialogOpen((v) => !v);
              }}
              className="relative inline-flex items-center gap-1 rounded-lg border border-[#f74c00]/30 bg-[#f74c00]/15 px-2 py-1 text-xs text-white hover:bg-[#f74c00]/25 transition sm:gap-2 sm:rounded-xl sm:px-3 sm:py-2 sm:text-sm"
              title={t("小龙虾聊天", "Lobster Chat")}
            >
              <span className="text-sm leading-none sm:text-base">🦞</span>
              <span className="hidden sm:inline">{t("聊天", "Chat")}</span>
              <MessageCircle className="h-3.5 w-3.5 sm:h-4 sm:w-4" />
            </button>
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-7xl space-y-3 p-3 sm:space-y-6 sm:p-6">
        {viewMode === "logs" ? (
          <section className="rounded-xl border border-white/10 bg-white/5 p-3 sm:rounded-2xl sm:p-5">
            <div className="mb-3 flex flex-wrap items-center justify-between gap-2 sm:mb-4 sm:gap-3">
              <h2 className="text-base font-semibold sm:text-lg">{t("最新日志", "Latest Logs")}</h2>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => void fetchLatestLog()}
                  disabled={logLoading}
                  className="inline-flex items-center justify-center gap-2 rounded-xl bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {logLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                  {tSlash("刷新 / Refresh")}
                </button>
              </div>
            </div>

            <div className="mb-3 grid gap-2 sm:gap-3 md:grid-cols-4">
              <label className="space-y-1 sm:space-y-2">
                <span className="text-[10px] uppercase tracking-widest text-white/50 sm:text-xs">{t("日志文件", "Log File")}</span>
                <select
                  className="w-full rounded-lg border border-white/15 bg-black/30 px-2 py-1.5 text-xs outline-none ring-[#f74c00] focus:ring-2 sm:rounded-xl sm:px-3 sm:py-2 sm:text-sm"
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

              <label className="space-y-1 sm:space-y-2">
                <span className="text-[10px] uppercase tracking-widest text-white/50 sm:text-xs">{t("尾部行数", "Tail Lines")}</span>
                <select
                  className="w-full rounded-lg border border-white/15 bg-black/30 px-2 py-1.5 text-xs outline-none ring-[#f74c00] focus:ring-2 sm:rounded-xl sm:px-3 sm:py-2 sm:text-sm"
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
                  <input
                    type="checkbox"
                    checked={logFollowTail}
                    onChange={(e) => setLogFollowTail(e.target.checked)}
                  />
                  {t("跟随到底部", "Follow tail")}
                </label>
              </div>

              <div className="flex items-end text-xs text-white/50">
                {logLastUpdated
                  ? `${t("更新时间", "Updated")}: ${toLocalTime(logLastUpdated)}`
                  : t("尚未加载", "Not loaded yet")}
              </div>
            </div>

            {logError ? (
              <p className="mb-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                {t("日志读取失败", "Log read failed")}: {logError}
              </p>
            ) : null}

            <pre
              ref={logContainerRef}
              className="h-[60vh] overflow-auto rounded-lg border border-white/10 bg-[#12151f] p-2 text-[10px] text-white/85 sm:h-[70vh] sm:rounded-xl sm:p-3 sm:text-xs"
            >
              {logText || t("日志为空", "Log is empty")}
            </pre>
          </section>
        ) : viewMode === "config" ? (
          <>
            <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
              <h2 className="mb-4 text-lg font-semibold">{tSlash("配置 / Config")}</h2>
              <p className="mb-4 text-xs text-white/50">
                {t("模型、技能开关与 LLM 厂商 API Key 写入 config.toml 等；修改后需重启相关服务生效。", "Models, skill switches and provider API keys are written to config.toml etc.; restart required to apply.")}
              </p>
              {providerKeysError ? (
                <p className="mb-3 rounded border border-red-500/30 bg-red-500/10 px-2 py-1.5 text-sm text-red-200">{providerKeysError}</p>
              ) : null}
              {providerKeysSaveMessage ? (
                <p className="mb-3 rounded border border-emerald-500/30 bg-emerald-500/10 px-2 py-1.5 text-sm text-emerald-200">{providerKeysSaveMessage}</p>
              ) : null}
              <div className="mt-3 flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  onClick={() => void restartClawd()}
                  disabled={restartClawdLoading}
                  className="inline-flex items-center gap-1 rounded-lg border border-amber-400/40 bg-amber-500/20 px-3 py-1.5 text-xs font-medium text-white transition hover:bg-amber-500/30 disabled:cursor-not-allowed disabled:opacity-60"
                >
                  {restartClawdLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                  {tSlash("重启 clawd / Restart clawd")}
                </button>
                <span className="text-[10px] text-white/50">
                  {t("修改配置或 API Key 后点击以使生效；页面可能短暂断开。", "Click after changing config or API keys to apply; page may disconnect briefly.")}
                </span>
              </div>
              {restartClawdMessage ? (
                <p className={`mt-2 rounded border px-2 py-1.5 text-sm ${restartClawdMessage.startsWith(t("重启失败", "Restart failed")) ? "border-red-500/30 bg-red-500/10 text-red-200" : "border-emerald-500/30 bg-emerald-500/10 text-emerald-200"}`}>
                  {restartClawdMessage}
                </p>
              ) : null}
            </section>

            {modelConfigLoading && !modelConfigDraft ? (
              <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
                <p className="text-xs text-white/50">{tSlash("模型配置加载中... / Loading model config...")}</p>
              </section>
            ) : null}
            {modelConfigDraft ? (
              <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
                <div className="mb-2 flex flex-wrap items-center justify-between gap-2 sm:mb-3 sm:gap-3">
                  <h2 className="text-lg font-semibold">{tSlash("模型配置 / Model Config")}</h2>
                  <button
                    onClick={() => void saveModelConfig()}
                    disabled={modelConfigSaving || modelConfigLoading}
                    className="inline-flex items-center gap-1 rounded-lg bg-[#f74c00] px-2 py-1 text-[10px] font-medium text-white transition hover:bg-[#ff5c1a] disabled:cursor-not-allowed disabled:opacity-60 sm:gap-2 sm:rounded-xl sm:px-3 sm:py-1.5 sm:text-xs"
                  >
                    {modelConfigSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                    {tSlash("保存 / Save")}
                  </button>
                </div>
                <p className="mb-3 text-xs text-white/50">{t("写入 config.toml / image.toml / audio.toml；需重启后生效。", "Writes to config.toml, image.toml, audio.toml; restart required.")}</p>
                {modelConfigError ? <p className="mt-2 rounded border border-red-500/30 bg-red-500/10 px-2 py-1.5 text-sm text-red-200">{modelConfigError}</p> : null}
                {modelConfigSaveMessage ? <p className="mt-2 rounded border border-emerald-500/30 bg-emerald-500/10 px-2 py-1.5 text-sm text-emerald-200">{modelConfigSaveMessage}</p> : null}
                <div className="mt-3 space-y-4">
                  <div className="rounded-lg border border-white/10 bg-black/20 p-3">
                    <h4 className="mb-2 text-xs font-medium uppercase tracking-widest text-white/50">{tSlash("主模型设置 / Main Model (Text LLM)")}</h4>
                    <div className="grid gap-2 sm:grid-cols-2">
                      <label className="space-y-1"><span className="text-[10px] text-white/50">Vendor</span>
                        <input className="w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-sm" value={modelConfigDraft.llm.vendor} onChange={(e) => setModelConfigDraft((p) => p ? { ...p, llm: { ...p.llm, vendor: e.target.value } } : p)} />
                      </label>
                      <label className="space-y-1"><span className="text-[10px] text-white/50">Model</span>
                        <input className="w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-sm" value={modelConfigDraft.llm.model} onChange={(e) => setModelConfigDraft((p) => p ? { ...p, llm: { ...p.llm, model: e.target.value } } : p)} />
                      </label>
                    </div>
                    <div className="mt-3 flex flex-wrap items-center justify-between gap-2">
                      <span className="text-[10px] text-white/50">{tSlash("API Key")}</span>
                      <button
                        onClick={() => void saveProviderKeysGroup("llm")}
                        disabled={providerKeysSaving !== null || providerKeysLoading}
                        className="inline-flex items-center gap-1 rounded-lg bg-[#f74c00] px-2 py-1 text-xs font-medium text-white transition hover:bg-[#ff5c1a] disabled:cursor-not-allowed disabled:opacity-60"
                      >
                        {providerKeysSaving === "llm" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                        {tSlash("保存 / Save")}
                      </button>
                    </div>
                    <p className="text-[10px] text-white/50">{t("config.toml [llm].api_key", "config.toml [llm].api_key")}</p>
                    {providerKeysLoading ? (
                      <p className="mt-2 text-xs text-white/50">{tSlash("加载中... / Loading...")}</p>
                    ) : (
                      <label className="mt-2 block space-y-1">
                        <input
                          type="password"
                          autoComplete="off"
                          className="w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-sm"
                          placeholder={(providerKeysData.llm ?? {})[modelConfigDraft.llm.vendor] || t("未配置", "not set")}
                          value={providerKeysDraft.llm?.[modelConfigDraft.llm.vendor] ?? ""}
                          onChange={(e) => {
                            const v = modelConfigDraft.llm.vendor;
                            setProviderKeysDraft((prev) => ({
                              ...prev,
                              llm: { ...(prev.llm ?? {}), [v]: e.target.value },
                            }));
                          }}
                        />
                      </label>
                    )}
                  </div>
                  <div className="rounded-lg border border-white/10 bg-black/20 p-3">
                    <h4 className="mb-2 text-xs font-medium uppercase tracking-widest text-white/50">{tSlash("图像 / Image")}</h4>
                    {(["image_edit", "image_generation", "image_vision"] as const).map((field) => (
                      <div key={field} className="mb-2 grid gap-2 sm:grid-cols-2">
                        <span className="text-[10px] text-white/60">{field}</span>
                        <div className="grid grid-cols-2 gap-2">
                          <input className="w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-sm" placeholder="vendor" value={modelConfigDraft[field].vendor} onChange={(e) => setModelConfigDraft((p) => p ? { ...p, [field]: { ...p[field], vendor: e.target.value } } : p)} />
                          <input className="w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-sm" placeholder="model" value={modelConfigDraft[field].model} onChange={(e) => setModelConfigDraft((p) => p ? { ...p, [field]: { ...p[field], model: e.target.value } } : p)} />
                        </div>
                      </div>
                    ))}
                    <div className="mt-3 flex flex-wrap items-center justify-between gap-2">
                      <span className="text-[10px] text-white/50">{tSlash("API Key")}</span>
                      <button
                        onClick={() => void saveProviderKeysGroup("image")}
                        disabled={providerKeysSaving !== null || providerKeysLoading}
                        className="inline-flex items-center gap-1 rounded-lg bg-[#f74c00] px-2 py-1 text-xs font-medium text-white transition hover:bg-[#ff5c1a] disabled:cursor-not-allowed disabled:opacity-60"
                      >
                        {providerKeysSaving === "image" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                        {tSlash("保存 / Save")}
                      </button>
                    </div>
                    <p className="text-[10px] text-white/50">{t("configs/image.toml [image_*].providers.<vendor>.api_key", "configs/image.toml")}</p>
                    {providerKeysLoading ? (
                      <p className="mt-2 text-xs text-white/50">{tSlash("加载中... / Loading...")}</p>
                    ) : (
                      <div className="mt-2 space-y-3">
                        {(["image_edit", "image_generation", "image_vision"] as const).map((section) => {
                          const vendor = modelConfigDraft[section].vendor;
                          return (
                            <div key={`img-${section}`}>
                              <span className="text-[10px] text-white/50">{section}</span>
                              <input
                                type="password"
                                autoComplete="off"
                                className="mt-1 w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-sm"
                                placeholder={(providerKeysData.image ?? {})[section]?.[vendor] || t("未配置", "not set")}
                                value={providerKeysDraft.image?.[section]?.[vendor] ?? ""}
                                onChange={(e) =>
                                  setProviderKeysDraft((prev) => ({
                                    ...prev,
                                    image: {
                                      ...(prev.image ?? {}),
                                      [section]: { ...(prev.image?.[section] ?? {}), [vendor]: e.target.value },
                                    },
                                  }))
                                }
                              />
                            </div>
                          );
                        })}
                      </div>
                    )}
                  </div>
                  <div className="rounded-lg border border-white/10 bg-black/20 p-3">
                    <h4 className="mb-2 text-xs font-medium uppercase tracking-widest text-white/50">{tSlash("语音转文字 / Audio Transcribe")}</h4>
                    <div className="grid gap-2 sm:grid-cols-2">
                      <label className="space-y-1"><span className="text-[10px] text-white/50">Vendor</span>
                        <input className="w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-sm" value={modelConfigDraft.audio_transcribe.vendor} onChange={(e) => setModelConfigDraft((p) => p ? { ...p, audio_transcribe: { ...p.audio_transcribe, vendor: e.target.value } } : p)} />
                      </label>
                      <label className="space-y-1"><span className="text-[10px] text-white/50">Model</span>
                        <input className="w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-sm" value={modelConfigDraft.audio_transcribe.model} onChange={(e) => setModelConfigDraft((p) => p ? { ...p, audio_transcribe: { ...p.audio_transcribe, model: e.target.value } } : p)} />
                      </label>
                    </div>
                    <div className="mt-3">
                      <span className="text-[10px] text-white/50">{tSlash("API Key")}</span>
                      {providerKeysLoading ? (
                        <p className="mt-1 text-xs text-white/50">{tSlash("加载中... / Loading...")}</p>
                      ) : (
                        <input
                          type="password"
                          autoComplete="off"
                          className="mt-1 w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-sm"
                          placeholder={(providerKeysData.audio?.audio_transcribe ?? {})[modelConfigDraft.audio_transcribe.vendor] || t("未配置", "not set")}
                          value={providerKeysDraft.audio?.audio_transcribe?.[modelConfigDraft.audio_transcribe.vendor] ?? ""}
                          onChange={(e) => {
                            const v = modelConfigDraft.audio_transcribe.vendor;
                            setProviderKeysDraft((prev) => ({
                              ...prev,
                              audio: {
                                ...(prev.audio ?? {}),
                                audio_transcribe: { ...(prev.audio?.audio_transcribe ?? {}), [v]: e.target.value },
                              },
                            }));
                          }}
                        />
                      )}
                    </div>
                  </div>
                  <div className="rounded-lg border border-white/10 bg-black/20 p-3">
                    <h4 className="mb-2 text-xs font-medium uppercase tracking-widest text-white/50">{tSlash("文字转语音 / Audio Synthesize")}</h4>
                    <div className="grid gap-2 sm:grid-cols-2">
                      <label className="space-y-1"><span className="text-[10px] text-white/50">Vendor</span>
                        <input className="w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-sm" value={modelConfigDraft.audio_synthesize.vendor} onChange={(e) => setModelConfigDraft((p) => p ? { ...p, audio_synthesize: { ...p.audio_synthesize, vendor: e.target.value } } : p)} />
                      </label>
                      <label className="space-y-1"><span className="text-[10px] text-white/50">Model</span>
                        <input className="w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-sm" value={modelConfigDraft.audio_synthesize.model} onChange={(e) => setModelConfigDraft((p) => p ? { ...p, audio_synthesize: { ...p.audio_synthesize, model: e.target.value } } : p)} />
                      </label>
                    </div>
                    <div className="mt-3 flex flex-wrap items-center justify-between gap-2">
                      <span className="text-[10px] text-white/50">{tSlash("API Key")}</span>
                      <button
                        onClick={() => void saveProviderKeysGroup("audio")}
                        disabled={providerKeysSaving !== null || providerKeysLoading}
                        className="inline-flex items-center gap-1 rounded-lg bg-[#f74c00] px-2 py-1 text-xs font-medium text-white transition hover:bg-[#ff5c1a] disabled:cursor-not-allowed disabled:opacity-60"
                      >
                        {providerKeysSaving === "audio" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                        {tSlash("保存 / Save")}
                      </button>
                    </div>
                    <p className="text-[10px] text-white/50">{t("configs/audio.toml [audio_*].providers.<vendor>.api_key", "configs/audio.toml")}</p>
                    {providerKeysLoading ? (
                      <p className="mt-2 text-xs text-white/50">{tSlash("加载中... / Loading...")}</p>
                    ) : (
                      <div className="mt-2">
                        <input
                          type="password"
                          autoComplete="off"
                          className="w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-sm"
                          placeholder={(providerKeysData.audio?.audio_synthesize ?? {})[modelConfigDraft.audio_synthesize.vendor] || t("未配置", "not set")}
                          value={providerKeysDraft.audio?.audio_synthesize?.[modelConfigDraft.audio_synthesize.vendor] ?? ""}
                          onChange={(e) => {
                            const v = modelConfigDraft.audio_synthesize.vendor;
                            setProviderKeysDraft((prev) => ({
                              ...prev,
                              audio: {
                                ...(prev.audio ?? {}),
                                audio_synthesize: { ...(prev.audio?.audio_synthesize ?? {}), [v]: e.target.value },
                              },
                            }));
                          }}
                        />
                      </div>
                    )}
                  </div>
                </div>
              </section>
            ) : null}

            <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
              <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
                <h2 className="text-lg font-semibold">{tSlash("技能开关 / Skill Switches")}</h2>
                <button onClick={() => void saveSkillSwitches()} disabled={skillSwitchSaving || skillsConfigLoading || !hasUnsavedSkillSwitchChanges} className="inline-flex items-center gap-1 rounded-lg bg-[#f74c00] px-2 py-1 text-xs font-medium text-white transition hover:bg-[#ff5c1a] disabled:cursor-not-allowed disabled:opacity-60">
                  {skillSwitchSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                  {tSlash("保存开关 / Save Switches")}
                </button>
              </div>
              <p className="text-xs text-white/50">{t("只改 skill_switches；需重启 clawd 生效。", "Updates skill_switches only; restart clawd to apply.")}</p>
              {skillsConfigError ? <p className="mt-2 rounded border border-red-500/30 bg-red-500/10 px-2 py-1.5 text-sm text-red-200">{skillsConfigError}</p> : null}
              {skillSwitchSaveMessage ? <p className="mt-2 rounded border border-emerald-500/30 bg-emerald-500/10 px-2 py-1.5 text-sm text-emerald-200">{skillSwitchSaveMessage}</p> : null}
              {hasUnsavedSkillSwitchChanges ? <p className="mt-2 rounded border border-amber-500/30 bg-amber-500/10 px-2 py-1.5 text-sm text-amber-200">{t("有未保存的开关变更。", "Unsaved switch changes.")}</p> : null}
              <div className="mt-3 space-y-4">
                {imageSkillsList.length > 0 ? <div><h4 className="mb-2 text-xs font-medium uppercase tracking-widest text-white/50">{tSlash("图像技能 / Image")}</h4><div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">{imageSkillsList.map((name) => { const re = visibleRuntimeSkills.includes(name); const ce = configuredEnabledSkills.has(name); const pa = re !== ce; return (<label key={name} className="flex items-center justify-between gap-2 rounded border border-white/10 bg-[#12151f] px-2 py-1.5 text-xs"><span className="truncate text-white/85">{name}</span><span className="flex shrink-0 items-center gap-1"><span className={ce ? "text-emerald-200" : "text-amber-200"}>{ce ? t("已开启", "enabled") : t("已关闭", "disabled")}</span>{pa ? <span className="text-sky-200 text-[10px]">{t("待重启", "pending")}</span> : null}<button type="button" onClick={() => toggleSkillEnabled(name, !ce)} className="rounded border border-white/20 bg-white/5 px-1.5 py-0.5 text-[10px]">{ce ? t("关闭", "Disable") : t("开启", "Enable")}</button></span></label>); })}</div></div> : null}
                {audioSkillsList.length > 0 ? <div><h4 className="mb-2 text-xs font-medium uppercase tracking-widest text-white/50">{tSlash("语音技能 / Audio")}</h4><div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">{audioSkillsList.map((name) => { const re = visibleRuntimeSkills.includes(name); const ce = configuredEnabledSkills.has(name); const pa = re !== ce; return (<label key={name} className="flex items-center justify-between gap-2 rounded border border-white/10 bg-[#12151f] px-2 py-1.5 text-xs"><span className="truncate text-white/85">{name}</span><span className="flex shrink-0 items-center gap-1"><span className={ce ? "text-emerald-200" : "text-amber-200"}>{ce ? t("已开启", "enabled") : t("已关闭", "disabled")}</span>{pa ? <span className="text-sky-200 text-[10px]">{t("待重启", "pending")}</span> : null}<button type="button" onClick={() => toggleSkillEnabled(name, !ce)} className="rounded border border-white/20 bg-white/5 px-1.5 py-0.5 text-[10px]">{ce ? t("关闭", "Disable") : t("开启", "Enable")}</button></span></label>); })}</div></div> : null}
                {otherSkillsList.length > 0 ? <div><h4 className="mb-2 text-xs font-medium uppercase tracking-widest text-white/50">{tSlash("其他技能 / Other")}</h4><div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">{otherSkillsList.map((name) => { const re = visibleRuntimeSkills.includes(name); const ce = configuredEnabledSkills.has(name); const pa = re !== ce; return (<label key={name} className="flex items-center justify-between gap-2 rounded border border-white/10 bg-[#12151f] px-2 py-1.5 text-xs"><span className="truncate text-white/85">{name}</span><span className="flex shrink-0 items-center gap-1"><span className={ce ? "text-emerald-200" : "text-amber-200"}>{ce ? t("已开启", "enabled") : t("已关闭", "disabled")}</span>{pa ? <span className="text-sky-200 text-[10px]">{t("待重启", "pending")}</span> : null}<button type="button" onClick={() => toggleSkillEnabled(name, !ce)} className="rounded border border-white/20 bg-white/5 px-1.5 py-0.5 text-[10px]">{ce ? t("关闭", "Disable") : t("开启", "Enable")}</button></span></label>); })}</div></div> : null}
                {baseSkillsList.length > 0 ? <div><h4 className="mb-2 text-xs font-medium uppercase tracking-widest text-white/50">{tSlash("基本技能 / Base")}</h4><p className="mb-2 text-xs text-amber-200/90">{t("不建议关闭。", "Not recommended to disable.")}</p><div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">{baseSkillsList.map((name) => { const re = visibleRuntimeSkills.includes(name); const ce = configuredEnabledSkills.has(name); const pa = re !== ce; return (<label key={name} className="flex items-center justify-between gap-2 rounded border border-white/10 bg-[#12151f] px-2 py-1.5 text-xs"><span className="truncate text-white/85">{name}</span><span className="flex shrink-0 items-center gap-1"><span className={ce ? "text-emerald-200" : "text-amber-200"}>{ce ? t("已开启", "enabled") : t("已关闭", "disabled")}</span>{pa ? <span className="text-sky-200 text-[10px]">{t("待重启", "pending")}</span> : null}<button type="button" onClick={() => toggleSkillEnabled(name, !ce)} className="rounded border border-white/20 bg-white/5 px-1.5 py-0.5 text-[10px]">{ce ? t("关闭", "Disable") : t("开启", "Enable")}</button></span></label>); })}</div></div> : null}
                {managedSkills.length === 0 ? <span className="text-xs text-white/50">{skillsConfigLoading ? tSlash("加载中... / Loading...") : "--"}</span> : null}
              </div>
            </section>
          </>
        ) : (
          <>
        {chatDialogOpen && (
          <section className="rounded-2xl border border-white/10 bg-white/5">
            <div className="flex items-center justify-between border-b border-white/10 px-4 py-3">
              <div className="flex items-center gap-2">
                <span className="text-base leading-none">🦞</span>
                <h2 className="text-sm font-semibold">
                  {t("小龙虾聊天", "Lobster Chat")}
                  <span className="ml-2 text-xs font-normal text-white/60">
                    {t("鉴权", "Auth")}: key
                  </span>
                </h2>
              </div>
              <button
                onClick={() => setChatDialogOpen(false)}
                className="rounded-lg p-1 text-white/60 hover:bg-white/10 hover:text-white"
                title={t("收起", "Collapse")}
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="px-4 py-3">
              <div className="mb-3 flex flex-wrap items-center gap-3 text-sm">
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
                  className="rounded-lg border border-white/15 bg-white/5 px-2 py-1 text-xs hover:bg-white/10"
                >
                  {t("清空记录", "Clear")}
                </button>
              </div>

              <div className="h-72 overflow-auto rounded-xl border border-white/10 bg-black/30 p-3 space-y-3">
                {chatMessages.map((msg) => (
                  <div key={msg.id} className="space-y-1">
                    <div className="flex items-center gap-2 text-[11px] text-white/50">
                      <span>{msg.role}</span>
                      <span>{toLocalTime(msg.ts)}</span>
                    </div>
                    <div
                      className={
                        msg.role === "user"
                          ? "max-w-[95%] rounded-xl bg-[#f74c00]/20 px-3 py-2 text-sm text-white"
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
                  className="min-h-20 w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                placeholder={t("输入消息并发送到 clawd ask...", "Type message and send to clawd ask...")}
                  value={chatInput}
                  onChange={(e) => setChatInput(e.target.value)}
                  onKeyDown={handleChatInputKeyDown}
                />
                <button
                  onClick={() => void sendChatMessage()}
                  disabled={chatSending || !chatInput.trim()}
                  className="inline-flex items-center justify-center gap-2 rounded-xl bg-[#f74c00] px-4 py-2 text-sm font-medium text-white transition hover:bg-[#ff5c1a] disabled:cursor-not-allowed disabled:opacity-60"
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
            </div>
          </section>
        )}

        {(queuePressureHigh || runningTooOld || !isOnline) && (
          <section className="rounded-xl border border-amber-500/30 bg-amber-500/10 p-3 sm:rounded-2xl sm:p-4">
            <div className="flex items-start gap-2 sm:gap-3">
              <BellRing className="mt-0.5 h-4 w-4 shrink-0 text-amber-300 sm:h-5 sm:w-5" />
              <div className="min-w-0 space-y-0.5 text-xs sm:space-y-1 sm:text-sm">
                <p className="font-semibold text-amber-200">{t("监控告警", "Alerts")}</p>
                {!isOnline ? <p className="text-amber-100">- 无法访问 clawd 健康接口，请检查服务或地址。/ Health endpoint unreachable.</p> : null}
                {queuePressureHigh ? (
                  <p className="text-amber-100">
                    - 队列任务数为 {health?.queue_length ?? 0}，已达到阈值 {queueWarn}。/ Queue reached threshold.
                  </p>
                ) : null}
                {runningTooOld ? (
                  <p className="text-amber-100">
                    - 最久运行任务已持续 {formatDuration(health?.running_oldest_age_seconds)}，超过阈值{" "}
                    {formatDuration(ageWarnSeconds)}。/ Oldest running task exceeded threshold.
                  </p>
                ) : null}
              </div>
            </div>
          </section>
        )}

        <section className="rounded-xl border border-white/10 bg-white/5 p-3 sm:rounded-2xl sm:p-5">
          <div className="grid gap-2 sm:gap-4 md:grid-cols-2 xl:grid-cols-[2fr_1fr_1fr_1fr_1fr_auto]">
            <label className="space-y-1 sm:space-y-2">
              <span className="text-[10px] uppercase tracking-widest text-white/50 sm:text-xs">{t("clawd API 地址", "clawd API URL")}</span>
              <input
                className="w-full rounded-lg border border-white/15 bg-black/30 px-2 py-1.5 text-xs outline-none ring-[#f74c00] focus:ring-2 sm:rounded-xl sm:px-3 sm:py-2 sm:text-sm"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder={t(
                  "默认本机 8787；跨机或外网访问时填 clawd 地址",
                  "Default localhost:8787; fill in clawd URL when accessing from another machine or network"
                )}
                title={t(
                  "clawd API 地址。默认 127.0.0.1:8787，需要时修改",
                  "clawd API URL. Default 127.0.0.1:8787; change when needed"
                )}
              />
            </label>

            <label className="space-y-1 sm:space-y-2">
              <span className="text-[10px] uppercase tracking-widest text-white/50 sm:text-xs">{t("自动刷新", "Auto Refresh")}</span>
              <select
                className="w-full rounded-lg border border-white/15 bg-black/30 px-2 py-1.5 text-xs outline-none ring-[#f74c00] focus:ring-2 sm:rounded-xl sm:px-3 sm:py-2 sm:text-sm"
                value={pollingSeconds}
                onChange={(e) => setPollingSeconds(Number(e.target.value))}
              >
                <option value={3}>{t("3 秒", "3s")}</option>
                <option value={5}>{t("5 秒", "5s")}</option>
                <option value={10}>{t("10 秒", "10s")}</option>
                <option value={0}>{t("关闭", "Off")}</option>
              </select>
            </label>

            <label className="space-y-1 sm:space-y-2">
              <span className="text-[10px] uppercase tracking-widest text-white/50 sm:text-xs">{t("队列告警阈值", "Queue Alert")}</span>
              <input
                type="number"
                min={1}
                className="w-full rounded-lg border border-white/15 bg-black/30 px-2 py-1.5 text-xs outline-none ring-[#f74c00] focus:ring-2 sm:rounded-xl sm:px-3 sm:py-2 sm:text-sm"
                value={queueWarn}
                onChange={(e) => setQueueWarn(Math.max(1, Number(e.target.value) || 1))}
              />
            </label>

            <label className="space-y-1 sm:space-y-2">
              <span className="text-[10px] uppercase tracking-widest text-white/50 sm:text-xs">{t("运行时长告警(秒)", "Runtime Alert(s)")}</span>
              <input
                type="number"
                min={10}
                className="w-full rounded-lg border border-white/15 bg-black/30 px-2 py-1.5 text-xs outline-none ring-[#f74c00] focus:ring-2 sm:rounded-xl sm:px-3 sm:py-2 sm:text-sm"
                value={ageWarnSeconds}
                onChange={(e) => setAgeWarnSeconds(Math.max(10, Number(e.target.value) || 10))}
              />
            </label>

            <div className="flex items-end">
              <button
                onClick={() => void fetchHealth()}
                disabled={loading}
                className="inline-flex w-full items-center justify-center gap-1.5 rounded-lg bg-[#f74c00] px-3 py-1.5 text-xs font-medium text-white transition hover:bg-[#ff5c1a] disabled:cursor-not-allowed disabled:opacity-60 sm:gap-2 sm:rounded-xl sm:px-4 sm:py-2 sm:text-sm"
              >
                {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin sm:h-4 sm:w-4" /> : <RefreshCw className="h-3.5 w-3.5 sm:h-4 sm:w-4" />}
                {t("立即刷新", "Refresh")}
              </button>
            </div>

            <div className="flex items-end text-[10px] text-white/50 sm:text-xs">
              {pollingSeconds > 0 ? t(`每 ${pollingSeconds}s 自动轮询`, `Poll every ${pollingSeconds}s`) : t("自动轮询已关闭", "Polling off")}
            </div>
          </div>
          {error ? (
            <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200 min-h-[2.75rem] flex items-center">
              {t("接口错误", "API error")}: {error}
            </p>
          ) : (
            <div className="mt-3 min-h-[2.75rem]" aria-hidden />
          )}
        </section>

        <section className="grid gap-2 sm:gap-4 sm:grid-cols-2 xl:grid-cols-4">
          <StatCard title={tSlash("服务版本 / Version")} value={health?.version || "--"} />
          <StatCard title={tSlash("运行时长 / Uptime")} value={formatDuration(health?.uptime_seconds)} />
          <StatCard title={tSlash("队列任务数 / Queue")} value={health?.queue_length ?? "--"} hint="status=queued" />
          <StatCard title={tSlash("执行中任务数 / Running")} value={health?.running_length ?? "--"} hint="status=running" />
          <StatCard title={tSlash("最久运行任务 / Oldest Task")} value={formatDuration(health?.running_oldest_age_seconds)} />
          <StatCard title={tSlash("任务超时阈值 / Timeout")} value={formatDuration(health?.task_timeout_seconds)} />
          <StatCard title={tSlash("进程内存 RSS / RSS")} value={formatBytes(health?.memory_rss_bytes ?? null)} />
          <StatCard title="Worker 状态" value={health?.worker_state || "--"} />
        </section>

        <section className="grid gap-3 sm:gap-6 lg:grid-cols-2">
          <div className="rounded-xl border border-white/10 bg-white/5 p-3 sm:rounded-2xl sm:p-5">
            <h2 className="mb-2 flex items-center gap-1.5 text-base font-semibold sm:mb-4 sm:gap-2 sm:text-lg">
              <Server className="h-4 w-4 text-[#f74c00] sm:h-5 sm:w-5" />
              {tSlash("服务健康 / Service Health")}
            </h2>
            <div className="space-y-2 sm:space-y-3">
              <div className="flex items-center justify-between rounded-lg border border-white/10 bg-black/20 px-2 py-2 sm:rounded-xl sm:px-4 sm:py-3">
                <div className="flex min-w-0 items-center gap-1.5 sm:gap-2">
                  <Database className="h-3.5 w-3.5 shrink-0 text-white/70 sm:h-4 sm:w-4" />
                  <span className="truncate text-xs sm:text-sm">clawd /v1/health</span>
                </div>
                <span className={`shrink-0 text-xs sm:text-sm ${isOnline ? "text-emerald-300" : "text-red-300"}`}>
                  {isOnline ? "正常" : "不可达"}
                </span>
              </div>

              {adapterHealthRows.map((row) => (
                <div key={row.key} className="rounded-lg border border-white/10 bg-black/20 px-2 py-2 sm:rounded-xl sm:px-4 sm:py-3">
                  <div className="flex items-center justify-between gap-2 sm:gap-4">
                    <div className="flex min-w-0 items-center gap-1.5 sm:gap-2">
                      <Server className="h-3.5 w-3.5 shrink-0 text-white/70 sm:h-4 sm:w-4" />
                      <span className="truncate text-xs sm:text-sm">{row.label}</span>
                    </div>
                    <div className="shrink-0 text-right">
                      <span
                        className={
                          row.healthy === true
                            ? "block text-xs text-emerald-300 sm:text-sm"
                            : row.healthy === false
                              ? "block text-xs text-amber-300 sm:text-sm"
                              : "block text-xs text-white/50 sm:text-sm"
                        }
                      >
                        {row.healthy === true
                          ? tSlash("运行中 / Running")
                          : row.healthy === false
                            ? tSlash("未检测到 / Not Found")
                            : tSlash("未知 / Unknown")}
                      </span>
                      <p className="text-[11px] text-white/40 mt-0.5">
                        {tSlash("进程 / Proc")}: {row.processCount == null ? "--" : row.processCount} | RSS {formatBytes(row.memoryRssBytes ?? null)}
                      </p>
                      <div className="mt-1.5 flex flex-wrap justify-end gap-1 sm:mt-2 sm:gap-2">
                        {row.key === "whatsapp_web" && waLoginStatus?.connected !== true ? (
                          <button
                            onClick={() => setWaLoginDialogOpen(true)}
                            className="rounded border border-sky-500/30 bg-sky-500/10 px-1.5 py-0.5 text-[10px] text-sky-200 hover:bg-sky-500/20 sm:rounded-lg sm:px-2 sm:py-1 sm:text-xs"
                          >
                            {tSlash("扫码登录 / QR Login")}
                          </button>
                        ) : null}
                        {row.key === "whatsapp_web" && waLoginStatus?.connected === true ? (
                          <div className="flex items-center gap-1 sm:gap-2">
                            <span className="rounded border border-emerald-500/30 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] text-emerald-200 sm:rounded-lg sm:px-2 sm:py-1 sm:text-xs">
                              {tSlash("已登录 / Connected")}
                            </span>
                            <button
                              onClick={() => void logoutWhatsappWeb()}
                              disabled={waLogoutLoading}
                              className="rounded border border-red-500/30 bg-red-500/10 px-1.5 py-0.5 text-[10px] text-red-200 hover:bg-red-500/20 disabled:opacity-50 sm:rounded-lg sm:px-2 sm:py-1 sm:text-xs"
                            >
                              {waLogoutLoading ? tSlash("处理中 / Working") : tSlash("退出登录 / Logout")}
                            </button>
                          </div>
                        ) : null}
                        <button
                          onClick={() => void controlService(row.serviceName, "start")}
                          disabled={Boolean(serviceActionLoading[row.serviceName]) || row.healthy === true}
                          className="rounded border border-emerald-500/30 bg-emerald-500/15 px-1.5 py-0.5 text-[10px] text-emerald-200 hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-50 sm:rounded-lg sm:px-2 sm:py-1 sm:text-xs"
                        >
                          {serviceActionLoading[row.serviceName] ? tSlash("处理中 / Working") : tSlash("启动 / Start")}
                        </button>
                        <button
                          onClick={() => void controlService(row.serviceName, "stop")}
                          disabled={Boolean(serviceActionLoading[row.serviceName]) || row.healthy !== true}
                          className="rounded border border-red-500/30 bg-red-500/10 px-1.5 py-0.5 text-[10px] text-red-200 hover:bg-red-500/20 disabled:cursor-not-allowed disabled:opacity-50 sm:rounded-lg sm:px-2 sm:py-1 sm:text-xs"
                        >
                          {serviceActionLoading[row.serviceName] ? tSlash("处理中 / Working") : tSlash("停止 / Stop")}
                        </button>
                        <button
                          onClick={() => void controlService(row.serviceName, "restart")}
                          disabled={Boolean(serviceActionLoading[row.serviceName])}
                          className="rounded border border-sky-500/30 bg-sky-500/10 px-1.5 py-0.5 text-[10px] text-sky-200 hover:bg-sky-500/20 disabled:cursor-not-allowed disabled:opacity-50 sm:rounded-lg sm:px-2 sm:py-1 sm:text-xs"
                          title={tSlash("先停止再启动 / Stop then start")}
                        >
                          {serviceActionLoading[row.serviceName] ? tSlash("处理中 / Working") : tSlash("重启 / Restart")}
                        </button>
                      </div>
                    </div>
                  </div>

                  {row.key === "whatsapp_web" && waLoginDialogOpen ? (
                    <div className="mt-3 border-t border-white/10 pt-3 text-left">
                      <div className="mb-2 flex items-center justify-between">
                        <h3 className="text-sm font-semibold">{tSlash("WhatsApp Web 登录 / WhatsApp Web Login")}</h3>
                        <button
                          onClick={() => setWaLoginDialogOpen(false)}
                          className="rounded-lg p-1 text-white/60 hover:bg-white/10 hover:text-white"
                          title={t("收起", "Collapse")}
                        >
                          <X className="h-4 w-4" />
                        </button>
                      </div>
                      <div className="text-sm text-white/80">
                        {tSlash("连接状态 / Connection")}:{" "}
                        <span className={waLoginStatus?.connected ? "text-emerald-300" : "text-amber-300"}>
                          {waLoginStatus?.connected ? tSlash("已登录 / Connected") : tSlash("未登录 / Not Connected")}
                        </span>
                      </div>
                      {waLoginStatus?.connected ? (
                        <p className="mt-2 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
                          {tSlash("WhatsApp Web 已登录，无需扫码。 / WhatsApp Web already connected.")}
                        </p>
                      ) : waLoginStatus?.qr_data_url ? (
                        <div className="mt-2 inline-block rounded-xl border border-white/15 bg-white p-3">
                          <img src={waLoginStatus.qr_data_url} alt="WhatsApp QR" className="h-56 w-56" />
                        </div>
                      ) : (
                        <p className="mt-2 rounded-lg border border-white/10 bg-black/30 px-3 py-2 text-sm text-white/70">
                          {waLoginLoading
                            ? tSlash("正在拉取二维码... / Fetching QR...")
                            : tSlash("暂无可用二维码，请稍候或重启 whatsapp_webd。 / QR not ready yet, please wait or restart whatsapp_webd.")}
                        </p>
                      )}
                      {waLoginStatus?.last_error ? (
                        <p className="mt-2 text-xs text-amber-300">
                          {tSlash("最近错误 / Last error")}: {waLoginStatus.last_error}
                        </p>
                      ) : null}
                      {waLoginError ? (
                        <p className="mt-2 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                          {waLoginError}
                        </p>
                      ) : null}
                      <div className="mt-2">
                        <button
                          onClick={() => void fetchWhatsappWebLoginStatus()}
                          disabled={waLoginLoading}
                          className="inline-flex items-center gap-2 rounded-xl bg-white/10 px-3 py-2 text-xs hover:bg-white/20 disabled:opacity-50"
                        >
                          {waLoginLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                          {tSlash("刷新状态 / Refresh")}
                        </button>
                      </div>
                    </div>
                  ) : null}
                </div>
              ))}
              {serviceActionMessage ? (
                <p className="rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-xs text-white/80">{serviceActionMessage}</p>
              ) : null}

              <div className="rounded-lg border border-white/10 bg-black/20 px-2 py-2 sm:rounded-xl sm:px-4 sm:py-3">
                <div className="flex items-center gap-1.5 sm:gap-2">
                  <Timer className="h-3.5 w-3.5 text-white/70 sm:h-4 sm:w-4" />
                  <span className="text-xs sm:text-sm">{tSlash("预留适配器 / Future Adapters")}</span>
                </div>
                <div className="mt-1.5 flex flex-wrap gap-1 sm:mt-2 sm:gap-2">
                  {(health?.future_adapters_enabled?.length ?? 0) > 0 ? (
                    health?.future_adapters_enabled?.map((name) => (
                      <span key={name} className="rounded border border-amber-400/30 bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-200 sm:rounded-md sm:px-2 sm:text-xs">
                        {name}
                      </span>
                    ))
                  ) : (
                    <span className="text-xs text-white/50">--</span>
                  )}
                </div>
              </div>
            </div>
          </div>

          <div className="rounded-xl border border-white/10 bg-white/5 p-3 sm:rounded-2xl sm:p-5">
            <h2 className="mb-2 flex items-center gap-1.5 text-base font-semibold sm:mb-4 sm:gap-2 sm:text-lg">
              <Clock3 className="h-4 w-4 text-[#f74c00] sm:h-5 sm:w-5" />
              <span className="text-xs sm:text-base">最近采样（最多 24 条，本地趋势）/ Recent Samples (24 max)</span>
            </h2>
            <div className="mb-3 grid grid-cols-3 gap-2 sm:mb-4 sm:gap-3">
              <div className="rounded-lg border border-white/10 bg-black/20 p-2 sm:rounded-xl sm:p-3">
                <p className="text-[10px] text-white/50 sm:text-xs">{tSlash("队列趋势 / Queue")}</p>
                <div className="mt-2 flex h-10 items-end gap-1">
                  {snapshots.slice(-16).map((s) => (
                    <div
                      key={`${s.ts}-q`}
                      className="w-2 rounded-sm bg-sky-400/80"
                      style={{ height: `${Math.max(8, Math.min(100, s.queue * 12))}%` }}
                      title={`${toLocalTime(s.ts)} | queue=${s.queue}`}
                    />
                  ))}
                </div>
              </div>
              <div className="rounded-lg border border-white/10 bg-black/20 p-2 sm:rounded-xl sm:p-3">
                <p className="text-[10px] text-white/50 sm:text-xs">{tSlash("运行中趋势 / Running")}</p>
                <div className="mt-2 flex h-10 items-end gap-1">
                  {snapshots.slice(-16).map((s) => (
                    <div
                      key={`${s.ts}-r`}
                      className="w-2 rounded-sm bg-violet-400/80"
                      style={{ height: `${Math.max(8, Math.min(100, s.running * 16))}%` }}
                      title={`${toLocalTime(s.ts)} | running=${s.running}`}
                    />
                  ))}
                </div>
              </div>
              <div className="rounded-lg border border-white/10 bg-black/20 p-2 sm:rounded-xl sm:p-3">
                <p className="text-[10px] text-white/50 sm:text-xs">{tSlash("内存趋势 / Memory")}</p>
                <div className="mt-2 flex h-10 items-end gap-1">
                  {snapshots.slice(-16).map((s) => (
                    <div
                      key={`${s.ts}-m`}
                      className="w-2 rounded-sm bg-emerald-400/80"
                      style={{
                        height: `${Math.max(
                          8,
                          Math.min(100, (((s.memory ?? 0) / memoryMax) * 100) || 8),
                        )}%`,
                      }}
                      title={`${toLocalTime(s.ts)} | memory=${formatBytes(s.memory)}`}
                    />
                  ))}
                </div>
              </div>
            </div>
            <div className="max-h-[200px] overflow-auto rounded-lg border border-white/10 bg-black/20 sm:max-h-[280px] sm:rounded-xl">
              <table className="w-full text-[10px] sm:text-sm">
                <thead className="sticky top-0 bg-[#151923] text-left text-white/60">
                  <tr>
                    <th className="px-2 py-1 sm:px-3 sm:py-2">{tSlash("时间 / Time")}</th>
                    <th className="px-2 py-1 sm:px-3 sm:py-2">{tSlash("队列 / Queue")}</th>
                    <th className="px-2 py-1 sm:px-3 sm:py-2">{tSlash("运行中 / Running")}</th>
                    <th className="px-2 py-1 sm:px-3 sm:py-2">{tSlash("内存 / Memory")}</th>
                  </tr>
                </thead>
                <tbody>
                  {timeline.length === 0 ? (
                    <tr>
                      <td className="px-2 py-2 text-white/40 sm:px-3 sm:py-4" colSpan={4}>
                        {tSlash("暂无采样数据 / No samples")}
                      </td>
                    </tr>
                  ) : (
                    timeline.map((item) => (
                      <tr key={item.ts} className="border-t border-white/5">
                        <td className="px-2 py-1 font-mono text-white/70 sm:px-3 sm:py-2">{toLocalTime(item.ts)}</td>
                        <td className="px-2 py-1 text-white/80 sm:px-3 sm:py-2">{item.queue}</td>
                        <td className="px-2 py-1 text-white/80 sm:px-3 sm:py-2">{item.running}</td>
                        <td className="px-2 py-1 text-white/80 sm:px-3 sm:py-2">{formatBytes(item.memory)}</td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
          </div>
        </section>

        <section className="rounded-xl border border-white/10 bg-white/5 p-3 sm:rounded-2xl sm:p-5">
          <h2 className="mb-2 text-base font-semibold sm:mb-4 sm:text-lg">原始数据（本地调试）/ Raw Data (Debug)</h2>
          <pre className="max-h-48 overflow-auto rounded-lg border border-white/10 bg-[#12151f] p-2 text-[10px] text-white/80 sm:max-h-72 sm:p-3 sm:text-xs">
            {JSON.stringify(health, null, 2)}
          </pre>
        </section>

        <section className="rounded-xl border border-white/10 bg-white/5 p-3 sm:rounded-2xl sm:p-5">
          <div className="mb-2 flex flex-wrap items-center justify-between gap-2 sm:mb-3 sm:gap-3">
            <h2 className="text-base font-semibold sm:text-lg">{tSlash("当前技能列表 / Active Skills")}</h2>
            <div className="flex items-center gap-1 sm:gap-2">
              <button
                onClick={() => void fetchSkills()}
                disabled={skillsLoading}
                className="inline-flex items-center justify-center gap-1 rounded-lg bg-white/10 px-2 py-1 text-[10px] font-medium transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-50 sm:rounded-xl sm:px-3 sm:py-1.5 sm:text-xs"
              >
                {skillsLoading ? <Loader2 className="h-3 w-3 animate-spin sm:h-3.5 sm:w-3.5" /> : <RefreshCw className="h-3 w-3 sm:h-3.5 sm:w-3.5" />}
                {tSlash("刷新运行态 / Refresh Runtime")}
              </button>
              <button
                onClick={() => void fetchSkillsConfig()}
                disabled={skillsConfigLoading}
                className="inline-flex items-center justify-center gap-1 rounded-lg bg-white/10 px-2 py-1 text-[10px] font-medium transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-50 sm:rounded-xl sm:px-3 sm:py-1.5 sm:text-xs"
              >
                {skillsConfigLoading ? <Loader2 className="h-3 w-3 animate-spin sm:h-3.5 sm:w-3.5" /> : <RefreshCw className="h-3 w-3 sm:h-3.5 sm:w-3.5" />}
                {tSlash("刷新配置 / Refresh Config")}
              </button>
            </div>
          </div>
          <p className="text-[10px] text-white/50 sm:text-xs">
            {tSlash("技能数量 / Skill Count")}: {visibleRuntimeSkills.length}
            {skillsData?.skill_runner_path ? ` | skill-runner: ${skillsData.skill_runner_path}` : ""}
          </p>
          {skillsError ? (
            <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
              {tSlash("技能读取失败 / Skills fetch failed")}: {skillsError}
            </p>
          ) : null}
          <div className="mt-2 flex flex-wrap gap-1 sm:mt-3 sm:gap-2">
            {visibleRuntimeSkills.length > 0 ? (
              visibleRuntimeSkills.map((name) => (
                <span key={name} className="rounded border border-sky-400/30 bg-sky-500/10 px-1.5 py-0.5 text-[10px] text-sky-200 sm:rounded-md sm:px-2 sm:py-1 sm:text-xs">
                  {name}
                </span>
              ))
            ) : (
              <span className="text-xs text-white/50">{skillsLoading ? tSlash("加载中... / Loading...") : "--"}</span>
            )}
          </div>
          <div className="mt-3 rounded-lg border border-amber-500/20 bg-amber-500/5 p-3 sm:rounded-xl sm:p-4">
            <p className="text-xs text-white/70">
              {t("模型、技能开关与 API Key 请在「配置」页修改。", "Edit models, skill switches and API keys on the Config page.")}
            </p>
            <button
              onClick={() => setViewMode("config")}
              className="mt-2 inline-flex items-center gap-2 rounded-lg bg-amber-500/20 px-3 py-1.5 text-xs font-medium text-amber-200 transition hover:bg-amber-500/30"
            >
              <Settings className="h-3.5 w-3.5" />
              {tSlash("前往配置页 / Go to Config")}
            </button>
          </div>
        </section>

        <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
          <h2 className="mb-4 text-lg font-semibold">{tSlash("与 clawd 交互 / Interact with clawd")}</h2>
          <div className="grid gap-4 md:grid-cols-2">
            <label className="space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">{tSlash("任务类型 / Task Type")}</span>
              <select
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={interactionKind}
                onChange={(e) => setInteractionKind(e.target.value as "ask" | "run_skill")}
              >
                <option value="ask">ask</option>
                <option value="run_skill">run_skill</option>
              </select>
            </label>
            <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-2 text-sm">
              <p className="text-white/80">{tSlash("本地交互账号已就绪 / Local account is ready")}</p>
              <p className="mt-1 text-xs text-white/50">role={interactionRole}</p>
              {localContextLoading ? <p className="mt-1 text-xs text-white/50">{tSlash("加载中... / Loading...")}</p> : null}
              {localContextError ? <p className="mt-1 text-xs text-red-300">{tSlash("上下文错误 / Context error")}: {localContextError}</p> : null}
            </div>
          </div>
          <div className="mt-4 grid gap-4 md:grid-cols-2">
            <label className="space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">channel</span>
              <select
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={interactionChannel}
                onChange={(e) => setInteractionChannel(e.target.value as "ui" | "telegram" | "whatsapp" | "feishu")}
              >
                <option value="ui">ui</option>
                <option value="telegram">telegram</option>
                <option value="whatsapp">whatsapp</option>
                <option value="feishu">feishu</option>
              </select>
            </label>
            <label className="space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">payload.adapter (optional)</span>
              <input
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={interactionAdapter}
                onChange={(e) => setInteractionAdapter(e.target.value)}
                placeholder="telegram_bot / whatsapp_cloud / whatsapp_web / feishu"
              />
            </label>
            <label className="space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">external_user_id (optional)</span>
              <input
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={interactionExternalUserId}
                onChange={(e) => setInteractionExternalUserId(e.target.value)}
                placeholder={t("外部用户 ID（跨平台）", "External user id")}
              />
            </label>
            <label className="space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">external_chat_id (optional)</span>
              <input
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={interactionExternalChatId}
                onChange={(e) => setInteractionExternalChatId(e.target.value)}
                placeholder={t("外部会话 ID（WhatsApp 建议填写）", "External chat id")}
              />
            </label>
          </div>

          {interactionKind === "ask" ? (
            <div className="mt-4 space-y-4">
              <label className="space-y-2 block">
                <span className="text-xs uppercase tracking-widest text-white/50">ask.text</span>
                <textarea
                  className="min-h-28 w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                  value={interactionAskText}
                  onChange={(e) => setInteractionAskText(e.target.value)}
                />
              </label>
              <label className="inline-flex items-center gap-2 text-sm text-white/80">
                <input
                  type="checkbox"
                  checked={interactionAgentMode}
                  onChange={(e) => setInteractionAgentMode(e.target.checked)}
                />
                agent_mode
              </label>
            </div>
          ) : (
            <div className="mt-4 space-y-4">
              <label className="space-y-2 block">
                <span className="text-xs uppercase tracking-widest text-white/50">run_skill.skill_name</span>
                <input
                  className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                  value={interactionSkillName}
                  onChange={(e) => setInteractionSkillName(e.target.value)}
                />
              </label>
              <label className="space-y-2 block">
                <span className="text-xs uppercase tracking-widest text-white/50">{tSlash("run_skill.args (JSON 或字符串 / string)")}</span>
                <textarea
                  className="min-h-28 w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
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
              className="inline-flex items-center justify-center gap-2 rounded-xl bg-[#f74c00] px-4 py-2 text-sm font-medium text-white transition hover:bg-[#ff5c1a] disabled:cursor-not-allowed disabled:opacity-60"
            >
              {interactionLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
              {tSlash("提交任务 / Submit")}
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
          <h2 className="mb-4 text-lg font-semibold">{tSlash("任务查询 / Task Query")}</h2>
          <div className="grid gap-4 md:grid-cols-[1fr_auto]">
            <input
              className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
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
                  <p className="inline-block rounded-md bg-[#f74c00]/20 px-2 py-1 font-mono text-[#ffb08a]">
                    {taskResult.status}
                  </p>
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
        )}
      </main>

    </div>
  );
}
