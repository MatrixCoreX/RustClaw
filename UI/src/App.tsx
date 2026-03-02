import { useEffect, useMemo, useState } from "react";
import {
  AlertCircle,
  BellRing,
  CheckCircle2,
  Clock3,
  Database,
  Loader2,
  MessageCircle,
  RefreshCw,
  Server,
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

interface SkillsResponse {
  skills: string[];
  skill_runner_path?: string;
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
  serviceName: "telegramd" | "whatsappd" | "whatsapp_webd";
  healthy: boolean | null | undefined;
  processCount: number | null | undefined;
  memoryRssBytes: number | null | undefined;
}

const STORAGE_KEYS = {
  baseUrl: "rustclaw.monitor.baseUrl",
  polling: "rustclaw.monitor.pollingSeconds",
  queueWarn: "rustclaw.monitor.queueWarn",
  ageWarn: "rustclaw.monitor.ageWarnSeconds",
  lang: "rustclaw.monitor.lang",
} as const;

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
    <div className="rounded-2xl border border-white/10 bg-white/5 p-5">
      <p className="text-xs uppercase tracking-widest text-white/50">{title}</p>
      <p className="mt-2 text-2xl font-bold text-white">{value}</p>
      {hint ? <p className="mt-1 text-xs text-white/50">{hint}</p> : null}
    </div>
  );
}

export default function App() {
  const [lang, setLang] = useState<"zh" | "en">(() => {
    const saved = window.localStorage.getItem(STORAGE_KEYS.lang);
    return saved === "en" ? "en" : "zh";
  });
  const [baseUrl, setBaseUrl] = useState(() => {
    return window.localStorage.getItem(STORAGE_KEYS.baseUrl) || "http://127.0.0.1:8787";
  });
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

  const [taskId, setTaskId] = useState("");
  const [taskLoading, setTaskLoading] = useState(false);
  const [taskResult, setTaskResult] = useState<TaskQueryResponse | null>(null);
  const [taskError, setTaskError] = useState<string | null>(null);
  const [trackingTaskId, setTrackingTaskId] = useState<string | null>(null);

  const [interactionKind, setInteractionKind] = useState<"ask" | "run_skill">("ask");
  const [interactionChannel, setInteractionChannel] = useState<"telegram" | "whatsapp">("telegram");
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

  const t = (zh: string, en: string) => (lang === "zh" ? zh : en);
  const tSlash = (mixed: string) => {
    const [zh, en] = mixed.split(" / ");
    return lang === "zh" ? zh : en ?? zh;
  };

  const isOnline = Boolean(health) && !error;
  const queuePressureHigh = (health?.queue_length ?? 0) >= queueWarn;
  const runningTooOld = (health?.running_oldest_age_seconds ?? 0) >= ageWarnSeconds;

  const fetchHealth = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch(`${baseUrl.replace(/\/$/, "")}/v1/health`);
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
    serviceName: "telegramd" | "whatsappd" | "whatsapp_webd",
    action: "start" | "stop",
  ) => {
    setServiceActionMessage(null);
    setServiceActionLoading((prev) => ({ ...prev, [serviceName]: true }));
    try {
      const res = await fetch(`${baseUrl.replace(/\/$/, "")}/v1/services/${serviceName}/${action}`, {
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
      const res = await fetch(`${baseUrl.replace(/\/$/, "")}/v1/whatsapp-web/login-status`);
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
      const res = await fetch(`${baseUrl.replace(/\/$/, "")}/v1/whatsapp-web/logout`, {
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
      const res = await fetch(`${baseUrl.replace(/\/$/, "")}/v1/local/interaction-context`);
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
      const res = await fetch(`${baseUrl.replace(/\/$/, "")}/v1/skills`);
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

  const fetchTaskById = async (id: string): Promise<TaskQueryResponse> => {
    const res = await fetch(`${baseUrl.replace(/\/$/, "")}/v1/tasks/${id.trim()}`);
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
    if (interactionUserId == null || interactionChatId == null) {
      setInteractionError("未获取到本地可用账号，请先检查 clawd 用户配置。");
      return;
    }
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
        user_id: interactionUserId,
        chat_id: interactionChatId,
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

      const res = await fetch(`${baseUrl.replace(/\/$/, "")}/v1/tasks`, {
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
    if (interactionUserId == null || interactionChatId == null) {
      setChatError("未获取到本地可用账号，请先检查 clawd 用户配置。");
      return;
    }
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
        user_id: interactionUserId,
        chat_id: interactionChatId,
        channel: interactionChannel,
        ...(interactionExternalUserId.trim() ? { external_user_id: interactionExternalUserId.trim() } : {}),
        ...(interactionExternalChatId.trim() ? { external_chat_id: interactionExternalChatId.trim() } : {}),
        kind: "ask" as const,
        payload: chatPayload,
      };
      const submitRes = await fetch(`${baseUrl.replace(/\/$/, "")}/v1/tasks`, {
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
    void fetchHealth();
    void fetchSkills();
    void fetchLocalInteractionContext();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (pollingSeconds <= 0) return;
    const timer = window.setInterval(() => {
      void fetchHealth();
    }, pollingSeconds * 1000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [baseUrl, pollingSeconds]);

  useEffect(() => {
    window.localStorage.setItem(STORAGE_KEYS.baseUrl, baseUrl);
  }, [baseUrl]);

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
    void fetchSkills();
    void fetchLocalInteractionContext();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [baseUrl]);

  useEffect(() => {
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
  }, [trackingTaskId, baseUrl]);

  useEffect(() => {
    if (!waLoginDialogOpen) return;
    void fetchWhatsappWebLoginStatus();
    const timer = window.setInterval(() => {
      void fetchWhatsappWebLoginStatus(true);
    }, 2000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [waLoginDialogOpen, baseUrl]);

  useEffect(() => {
    // Keep whatsapp web login status fresh for row actions.
    void fetchWhatsappWebLoginStatus(true);
    const timer = window.setInterval(() => {
      void fetchWhatsappWebLoginStatus(true);
    }, 5000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [baseUrl]);

  const timeline = useMemo(() => snapshots.slice().reverse(), [snapshots]);
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
        key: "whatsapp_cloud",
        label: "whatsapp_cloud",
        serviceName: "whatsappd",
        healthy: health?.whatsapp_cloud_healthy ?? health?.whatsappd_healthy,
        processCount: health?.whatsapp_cloud_process_count ?? health?.whatsappd_process_count,
        memoryRssBytes: health?.whatsapp_cloud_memory_rss_bytes ?? health?.whatsappd_memory_rss_bytes,
      },
      {
        key: "whatsapp_web",
        label: "whatsapp_web",
        serviceName: "whatsapp_webd",
        healthy: health?.whatsapp_web_healthy,
        processCount: health?.whatsapp_web_process_count,
        memoryRssBytes: health?.whatsapp_web_memory_rss_bytes,
      },
    ];
    // Running services first; not-started/unknown move to bottom.
    rows.sort((a, b) => Number(b.healthy === true) - Number(a.healthy === true));
    return rows;
  }, [health]);

  return (
    <div className="min-h-screen bg-[#0f1116] text-white selection:bg-[#f74c00]/30">
      <header className="sticky top-0 z-40 border-b border-white/10 bg-[#0f1116]/90 backdrop-blur px-6 py-4">
        <div className="mx-auto flex max-w-7xl flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
              <span className="text-2xl leading-none">🦞</span>
              <span>{t("RustClaw 后台", "RustClaw Console")}</span>
            </h1>
            <p className="mt-1 text-sm text-white/60">
              {t("实时查看 clawd 健康状态、任务队列与服务运行信息", "Monitor clawd health, queue and runtime in real time")}
            </p>
          </div>

          <div className="flex items-center gap-3">
            <div className="flex items-center gap-3 rounded-xl border border-white/10 bg-white/5 px-4 py-2">
              {isOnline ? (
                <CheckCircle2 className="h-4 w-4 text-emerald-400" />
              ) : (
                <AlertCircle className="h-4 w-4 text-red-400" />
              )}
              <span className="text-sm">{isOnline ? t("在线", "Online") : t("离线异常", "Offline")}</span>
              {lastUpdated ? (
                <span className="text-xs text-white/50">{lang === "zh" ? `更新于 ${toLocalTime(lastUpdated)}` : `Updated ${toLocalTime(lastUpdated)}`}</span>
              ) : null}
            </div>

            <button
              onClick={() => setLang((v) => (v === "zh" ? "en" : "zh"))}
              className="rounded-xl border border-white/15 bg-white/5 px-3 py-2 text-xs hover:bg-white/10"
              title={t("切换语言", "Switch Language")}
            >
              {lang === "zh" ? "中文" : "EN"}
            </button>

            <button
              onClick={() => setChatDialogOpen((v) => !v)}
              className="relative inline-flex items-center gap-2 rounded-xl border border-[#f74c00]/30 bg-[#f74c00]/15 px-3 py-2 text-sm text-white hover:bg-[#f74c00]/25 transition"
              title={t("小龙虾聊天", "Lobster Chat")}
            >
              <span className="text-base leading-none">🦞</span>
              <span className="hidden sm:inline">{t("聊天", "Chat")}</span>
              <MessageCircle className="h-4 w-4" />
            </button>
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-7xl space-y-6 p-6">
        {chatDialogOpen && (
          <section className="rounded-2xl border border-white/10 bg-white/5">
            <div className="flex items-center justify-between border-b border-white/10 px-4 py-3">
              <div className="flex items-center gap-2">
                <span className="text-base leading-none">🦞</span>
                <h2 className="text-sm font-semibold">{t("小龙虾聊天", "Lobster Chat")}</h2>
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

        {waLoginDialogOpen && (
          <section className="rounded-2xl border border-white/10 bg-white/5">
            <div className="flex items-center justify-between border-b border-white/10 px-4 py-3">
              <h2 className="text-sm font-semibold">{tSlash("WhatsApp Web 登录 / WhatsApp Web Login")}</h2>
              <button
                onClick={() => setWaLoginDialogOpen(false)}
                className="rounded-lg p-1 text-white/60 hover:bg-white/10 hover:text-white"
                title={t("关闭", "Close")}
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="px-4 py-4 space-y-3">
              <div className="text-sm text-white/80">
                {tSlash("连接状态 / Connection")}:{" "}
                <span className={waLoginStatus?.connected ? "text-emerald-300" : "text-amber-300"}>
                  {waLoginStatus?.connected ? tSlash("已登录 / Connected") : tSlash("未登录 / Not Connected")}
                </span>
              </div>
              {waLoginStatus?.connected ? (
                <p className="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
                  {tSlash("WhatsApp Web 已登录，无需扫码。 / WhatsApp Web already connected.")}
                </p>
              ) : waLoginStatus?.qr_data_url ? (
                <div className="inline-block rounded-xl border border-white/15 bg-white p-3">
                  <img src={waLoginStatus.qr_data_url} alt="WhatsApp QR" className="h-64 w-64" />
                </div>
              ) : (
                <p className="rounded-lg border border-white/10 bg-black/30 px-3 py-2 text-sm text-white/70">
                  {waLoginLoading
                    ? tSlash("正在拉取二维码... / Fetching QR...")
                    : tSlash("暂无可用二维码，请稍候或重启 whatsapp_webd。 / QR not ready yet, please wait or restart whatsapp_webd.")}
                </p>
              )}
              {waLoginStatus?.last_error ? (
                <p className="text-xs text-amber-300">
                  {tSlash("最近错误 / Last error")}: {waLoginStatus.last_error}
                </p>
              ) : null}
              {waLoginError ? (
                <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                  {waLoginError}
                </p>
              ) : null}
              <div>
                <button
                  onClick={() => void fetchWhatsappWebLoginStatus()}
                  disabled={waLoginLoading}
                  className="inline-flex items-center gap-2 rounded-xl bg-white/10 px-3 py-2 text-xs hover:bg-white/20 disabled:opacity-50"
                >
                  {waLoginLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                  {tSlash("刷新状态 / Refresh")}
                </button>
                {waLoginStatus?.connected ? (
                  <button
                    onClick={() => void logoutWhatsappWeb()}
                    disabled={waLogoutLoading}
                    className="ml-2 inline-flex items-center gap-2 rounded-xl border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-200 hover:bg-red-500/20 disabled:opacity-50"
                  >
                    {waLogoutLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                    {tSlash("退出登录 / Logout")}
                  </button>
                ) : null}
              </div>
            </div>
          </section>
        )}

        {(queuePressureHigh || runningTooOld || !isOnline) && (
          <section className="rounded-2xl border border-amber-500/30 bg-amber-500/10 p-4">
            <div className="flex items-start gap-3">
              <BellRing className="mt-0.5 h-5 w-5 text-amber-300" />
              <div className="space-y-1 text-sm">
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

        <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-[2fr_1fr_1fr_1fr_1fr_auto]">
            <label className="space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">{t("clawd API 地址", "clawd API URL")}</span>
              <input
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder="http://127.0.0.1:8787"
              />
            </label>

            <label className="space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">{t("自动刷新", "Auto Refresh")}</span>
              <select
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
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
              <span className="text-xs uppercase tracking-widest text-white/50">{t("队列告警阈值", "Queue Alert")}</span>
              <input
                type="number"
                min={1}
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={queueWarn}
                onChange={(e) => setQueueWarn(Math.max(1, Number(e.target.value) || 1))}
              />
            </label>

            <label className="space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">{t("运行时长告警(秒)", "Runtime Alert(s)")}</span>
              <input
                type="number"
                min={10}
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={ageWarnSeconds}
                onChange={(e) => setAgeWarnSeconds(Math.max(10, Number(e.target.value) || 10))}
              />
            </label>

            <div className="flex items-end">
              <button
                onClick={() => void fetchHealth()}
                disabled={loading}
                className="inline-flex w-full items-center justify-center gap-2 rounded-xl bg-[#f74c00] px-4 py-2 font-medium text-white transition hover:bg-[#ff5c1a] disabled:cursor-not-allowed disabled:opacity-60"
              >
                {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                {t("立即刷新", "Refresh")}
              </button>
            </div>

            <div className="flex items-end text-xs text-white/50">
              {pollingSeconds > 0 ? t(`每 ${pollingSeconds}s 自动轮询`, `Poll every ${pollingSeconds}s`) : t("自动轮询已关闭", "Polling off")}
            </div>
          </div>
          {error ? (
            <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
              {t("接口错误", "API error")}: {error}
            </p>
          ) : null}
        </section>

        <section className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          <StatCard title={tSlash("服务版本 / Version")} value={health?.version || "--"} />
          <StatCard title={tSlash("运行时长 / Uptime")} value={formatDuration(health?.uptime_seconds)} />
          <StatCard title={tSlash("队列任务数 / Queue")} value={health?.queue_length ?? "--"} hint="status=queued" />
          <StatCard title={tSlash("执行中任务数 / Running")} value={health?.running_length ?? "--"} hint="status=running" />
          <StatCard title={tSlash("最久运行任务 / Oldest Task")} value={formatDuration(health?.running_oldest_age_seconds)} />
          <StatCard title={tSlash("任务超时阈值 / Timeout")} value={formatDuration(health?.task_timeout_seconds)} />
          <StatCard title={tSlash("进程内存 RSS / RSS")} value={formatBytes(health?.memory_rss_bytes ?? null)} />
          <StatCard title="Worker 状态" value={health?.worker_state || "--"} />
        </section>

        <section className="grid gap-6 lg:grid-cols-2">
          <div className="rounded-2xl border border-white/10 bg-white/5 p-5">
            <h2 className="mb-4 flex items-center gap-2 text-lg font-semibold">
              <Server className="h-5 w-5 text-[#f74c00]" />
              {tSlash("服务健康 / Service Health")}
            </h2>
            <div className="space-y-3">
              <div className="flex items-center justify-between rounded-xl border border-white/10 bg-black/20 px-4 py-3">
                <div className="flex items-center gap-2">
                  <Database className="h-4 w-4 text-white/70" />
                  <span>clawd /v1/health</span>
                </div>
                <span className={isOnline ? "text-emerald-300" : "text-red-300"}>
                  {isOnline ? "正常" : "不可达"}
                </span>
              </div>

              {adapterHealthRows.map((row) => (
                <div key={row.key} className="flex items-center justify-between rounded-xl border border-white/10 bg-black/20 px-4 py-3">
                  <div className="flex items-center gap-2">
                    <Server className="h-4 w-4 text-white/70" />
                    <span>{row.label}</span>
                  </div>
                  <div className="text-right">
                    <span
                      className={
                        row.healthy === true
                          ? "text-emerald-300"
                          : row.healthy === false
                            ? "text-amber-300"
                            : "text-white/50"
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
                    <div className="mt-2 flex justify-end gap-2">
                      {row.key === "whatsapp_web" && waLoginStatus?.connected !== true ? (
                        <button
                          onClick={() => setWaLoginDialogOpen(true)}
                          className="rounded-lg border border-sky-500/30 bg-sky-500/10 px-2 py-1 text-xs text-sky-200 hover:bg-sky-500/20"
                        >
                          {tSlash("扫码登录 / QR Login")}
                        </button>
                      ) : null}
                      {row.key === "whatsapp_web" && waLoginStatus?.connected === true ? (
                        <div className="flex items-center gap-2">
                          <span className="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-xs text-emerald-200">
                            {tSlash("已登录 / Connected")}
                          </span>
                          <button
                            onClick={() => void logoutWhatsappWeb()}
                            disabled={waLogoutLoading}
                            className="rounded-lg border border-red-500/30 bg-red-500/10 px-2 py-1 text-xs text-red-200 hover:bg-red-500/20 disabled:opacity-50"
                          >
                            {waLogoutLoading ? tSlash("处理中 / Working") : tSlash("退出登录 / Logout")}
                          </button>
                        </div>
                      ) : null}
                      <button
                        onClick={() => void controlService(row.serviceName, "start")}
                        disabled={Boolean(serviceActionLoading[row.serviceName]) || row.healthy === true}
                        className="rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-2 py-1 text-xs text-emerald-200 hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        {serviceActionLoading[row.serviceName] ? tSlash("处理中 / Working") : tSlash("启动 / Start")}
                      </button>
                      <button
                        onClick={() => void controlService(row.serviceName, "stop")}
                        disabled={Boolean(serviceActionLoading[row.serviceName]) || row.healthy !== true}
                        className="rounded-lg border border-red-500/30 bg-red-500/10 px-2 py-1 text-xs text-red-200 hover:bg-red-500/20 disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        {serviceActionLoading[row.serviceName] ? tSlash("处理中 / Working") : tSlash("停止 / Stop")}
                      </button>
                    </div>
                  </div>
                </div>
              ))}
              {serviceActionMessage ? (
                <p className="rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-xs text-white/80">{serviceActionMessage}</p>
              ) : null}

              <div className="rounded-xl border border-white/10 bg-black/20 px-4 py-3">
                <div className="flex items-center gap-2">
                  <Timer className="h-4 w-4 text-white/70" />
                  <span>{tSlash("预留适配器 / Future Adapters")}</span>
                </div>
                <div className="mt-2 flex flex-wrap gap-2">
                  {(health?.future_adapters_enabled?.length ?? 0) > 0 ? (
                    health?.future_adapters_enabled?.map((name) => (
                      <span key={name} className="rounded-md border border-amber-400/30 bg-amber-500/10 px-2 py-0.5 text-xs text-amber-200">
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

          <div className="rounded-2xl border border-white/10 bg-white/5 p-5">
            <h2 className="mb-4 flex items-center gap-2 text-lg font-semibold">
              <Clock3 className="h-5 w-5 text-[#f74c00]" />
              最近采样（最多 24 条，本地趋势）/ Recent Samples (24 max)
            </h2>
            <div className="mb-4 grid grid-cols-3 gap-3">
              <div className="rounded-xl border border-white/10 bg-black/20 p-3">
                <p className="text-xs text-white/50">{tSlash("队列趋势 / Queue")}</p>
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
              <div className="rounded-xl border border-white/10 bg-black/20 p-3">
                <p className="text-xs text-white/50">{tSlash("运行中趋势 / Running")}</p>
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
              <div className="rounded-xl border border-white/10 bg-black/20 p-3">
                <p className="text-xs text-white/50">{tSlash("内存趋势 / Memory")}</p>
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
            <div className="max-h-[280px] overflow-auto rounded-xl border border-white/10 bg-black/20">
              <table className="w-full text-sm">
                <thead className="sticky top-0 bg-[#151923] text-left text-white/60">
                  <tr>
                    <th className="px-3 py-2">{tSlash("时间 / Time")}</th>
                    <th className="px-3 py-2">{tSlash("队列 / Queue")}</th>
                    <th className="px-3 py-2">{tSlash("运行中 / Running")}</th>
                    <th className="px-3 py-2">{tSlash("内存 / Memory")}</th>
                  </tr>
                </thead>
                <tbody>
                  {timeline.length === 0 ? (
                    <tr>
                      <td className="px-3 py-4 text-white/40" colSpan={4}>
                        {tSlash("暂无采样数据 / No samples")}
                      </td>
                    </tr>
                  ) : (
                    timeline.map((item) => (
                      <tr key={item.ts} className="border-t border-white/5">
                        <td className="px-3 py-2 font-mono text-white/70">{toLocalTime(item.ts)}</td>
                        <td className="px-3 py-2 text-white/80">{item.queue}</td>
                        <td className="px-3 py-2 text-white/80">{item.running}</td>
                        <td className="px-3 py-2 text-white/80">{formatBytes(item.memory)}</td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
          </div>
        </section>

        <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
          <h2 className="mb-4 text-lg font-semibold">原始数据（本地调试）/ Raw Data (Debug)</h2>
          <pre className="max-h-72 overflow-auto rounded-lg border border-white/10 bg-[#12151f] p-3 text-xs text-white/80">
            {JSON.stringify(health, null, 2)}
          </pre>
        </section>

        <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
          <div className="mb-3 flex items-center justify-between gap-3">
            <h2 className="text-lg font-semibold">{tSlash("当前技能列表 / Active Skills")}</h2>
            <button
              onClick={() => void fetchSkills()}
              disabled={skillsLoading}
              className="inline-flex items-center justify-center gap-2 rounded-xl bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {skillsLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
              {tSlash("刷新 / Refresh")}
            </button>
          </div>
          <p className="text-xs text-white/50">
            {tSlash("技能数量 / Skill Count")}: {skillsData?.skills?.length ?? 0}
            {skillsData?.skill_runner_path ? ` | skill-runner: ${skillsData.skill_runner_path}` : ""}
          </p>
          {skillsError ? (
            <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
              {tSlash("技能读取失败 / Skills fetch failed")}: {skillsError}
            </p>
          ) : null}
          <div className="mt-3 flex flex-wrap gap-2">
            {(skillsData?.skills?.length ?? 0) > 0 ? (
              skillsData?.skills?.map((name) => (
                <span key={name} className="rounded-md border border-sky-400/30 bg-sky-500/10 px-2 py-1 text-xs text-sky-200">
                  {name}
                </span>
              ))
            ) : (
              <span className="text-xs text-white/50">{skillsLoading ? tSlash("加载中... / Loading...") : "--"}</span>
            )}
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
              <p className="text-white/80">
                {tSlash("本地交互账号 / Local Account")}: user_id=<span className="font-mono">{interactionUserId ?? "--"}</span>，chat_id=
                <span className="font-mono">{interactionChatId ?? "--"}</span>
              </p>
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
                onChange={(e) => setInteractionChannel(e.target.value as "telegram" | "whatsapp")}
              >
                <option value="telegram">telegram</option>
                <option value="whatsapp">whatsapp</option>
              </select>
            </label>
            <label className="space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">payload.adapter (optional)</span>
              <input
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={interactionAdapter}
                onChange={(e) => setInteractionAdapter(e.target.value)}
                placeholder="telegram_bot / whatsapp_cloud / whatsapp_web"
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
                {tSlash("已提交 / Submitted")}: `{interactionSubmittedTaskId}` {trackingTaskId ? tSlash("（自动跟踪中 / auto tracking）") : ""}
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
      </main>

    </div>
  );
}
