import { useEffect, useMemo, useRef, useState } from "react";
import {
  Brain,
  Check,
  Database,
  FileText,
  LayoutDashboard,
  Loader2,
  MessageCircle,
  Network,
  RefreshCw,
  Sparkles,
  SquareTerminal,
  Server,
  Timer,
  Wrench,
} from "lucide-react";
import QRCode from "qrcode";
import { AuthKeysPage } from "./components/AuthKeysPage";
import { ChatPage } from "./components/ChatPage";
import { CommunicationSetupPage } from "./components/CommunicationSetupPage";
import { ConsoleLayout } from "./components/ConsoleLayout";
import { DashboardPage } from "./components/DashboardPage";
import { FactoryResetModal } from "./components/FactoryResetModal";
import { LogsPage } from "./components/LogsPage";
import { MemoryPage } from "./components/MemoryPage";
import { ModelConfigPage } from "./components/ModelConfigPage";
import { NniPage } from "./components/NniPage";
import { SignInPage } from "./components/SignInPage";
import { SkillsPage } from "./components/SkillsPage";
import { TasksPage } from "./components/TasksPage";
import {
  countCompletedDashboardSteps,
  getDashboardOverviewItems,
} from "./lib/dashboard-home";
import { maskStoredKey } from "./lib/auth-keys";
import { fileToDataUrl, formatVisionResultText } from "./lib/chat-attachments";
import { boundChannelsLabel as formatBoundChannelsLabel, channelLabel as formatChannelLabel } from "./lib/channel-display";
import { formatBytes, formatDuration, sleep, toLocalTime } from "./lib/display-format";
import {
  formatDateTimeHuman as formatDateTimeHumanValue,
  formatUnixDateTime as formatUnixDateTimeValue,
} from "./lib/date-format";
import {
  fetchFeishuBindSession,
  getFeishuBindStatusCopy,
  getFeishuSetupGuidance,
  getFeishuStepStatus,
  isFeishuBindTerminalStatus,
  startFeishuBindSession,
  type FeishuBindSessionResponse,
} from "./lib/feishu-bind";
import {
  MULTIMODAL_KEYS,
  buildMultimodalMetaView,
  type MultimodalKey,
} from "./lib/model-config";
import { serviceDisplayName } from "./lib/service-actions";
import { extractTaskText } from "./lib/task-result";
import {
  buildWorkspaceUpdateView,
  formatWorkspaceUpdateStatus,
  formatWorkspaceUpdateStep,
  formatWorkspaceUpdateTime,
} from "./lib/workspace-update";
import {
  NNI_HEARTBEAT_ERRORS_PAGE_SIZE,
  NNI_HEARTBEAT_RECORDS_PAGE_SIZE,
  useNniRuntime,
} from "./hooks/useNniRuntime";
import { useMemoryRuntime } from "./hooks/useMemoryRuntime";
import { useLogsRuntime } from "./hooks/useLogsRuntime";
import { useFactoryResetRuntime } from "./hooks/useFactoryResetRuntime";
import { useModelConfigRuntime } from "./hooks/useModelConfigRuntime";
import { useSkillsRuntime } from "./hooks/useSkillsRuntime";
import { useChannelConfigRuntime } from "./hooks/useChannelConfigRuntime";
import { useAuthKeysRuntime } from "./hooks/useAuthKeysRuntime";
import { useChannelBindingRuntime } from "./hooks/useChannelBindingRuntime";
import { useServiceActionsRuntime } from "./hooks/useServiceActionsRuntime";

import type {
  ApiResponse,
  HealthResponse,
  TaskQueryResponse,
  ActiveTaskItem,
  ActiveTasksResponse,
  SubmitTaskResponse,
  WorkspaceUpdateMode,
  WorkspaceUpdateStatus,
  PiAppStatusResponse,
  LocalInteractionContextResponse,
  AuthIdentityResponse,
  NniDeviceMeta,
  AgentConfigItem,
  WhatsappWebLoginStatus,
  WechatLoginStatus,
  WechatQrStartResponse,
  WechatQrWaitResponse,
  ChatMessage,
  ChatImageAttachment,
  AdapterHealthRow,
  ChannelPreset,
  ServiceStatusRow,
  DashboardCommunicationRow,
  ChannelName,
  ConsolePage,
} from "./types/api";

const CONSOLE_PAGES: ConsolePage[] = ["dashboard", "chat", "nni", "services", "channels", "models", "skills", "memory", "logs", "tasks"];

const STORAGE_KEYS = {
  baseUrl: "rustclaw.monitor.baseUrl",
  webdBaseUrl: "rustclaw.monitor.webdBaseUrl",
  userKey: "rustclaw.monitor.userKey",
  authMode: "rustclaw.monitor.authMode",
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

/** 根据当前页面地址推断 webd 默认地址；获取不到主机名时用 127.0.0.1 */
function getDefaultWebdBaseUrl(): string {
  if (typeof window === "undefined" || !window.location) return "http://127.0.0.1:8788";
  const loc = window.location;
  let hostname = (loc.hostname && loc.hostname.trim()) || "";
  if (!hostname && loc.host) {
    hostname = loc.host.split(":")[0]?.trim() || "";
  }
  const protocol = loc.protocol && loc.protocol !== "file:" ? loc.protocol : "http:";
  if (hostname) return `${protocol}//${hostname}:8788`;
  return "http://127.0.0.1:8788";
}

function readNumber(key: string, fallback: number): number {
  const raw = window.localStorage.getItem(key);
  if (!raw) return fallback;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
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
  const [authMode, setAuthMode] = useState<"key" | "webd" | null>(() => {
    const saved = window.localStorage.getItem(STORAGE_KEYS.authMode);
    if (saved === "webd" || saved === "key") return saved;
    return null;
  });
  const [loginTab, setLoginTab] = useState<"key" | "webd">("webd");
  const [webdBaseUrlDraft, setWebdBaseUrlDraft] = useState(() => {
    const saved = window.localStorage.getItem(STORAGE_KEYS.webdBaseUrl);
    if (saved != null && saved.trim() !== "") return saved.trim();
    return getDefaultWebdBaseUrl();
  });
  const [webdUsername, setWebdUsername] = useState("");
  const [webdPassword, setWebdPassword] = useState("");
  const [uiKeyDraft, setUiKeyDraft] = useState("");
  const [uiAuthReady, setUiAuthReady] = useState(false);
  const [uiAuthLoading, setUiAuthLoading] = useState(false);
  const [uiAuthError, setUiAuthError] = useState<string | null>(null);
  const [authIdentity, setAuthIdentity] = useState<AuthIdentityResponse | null>(null);
  const [authMeLoading, setAuthMeLoading] = useState(false);
  const [authMeError, setAuthMeError] = useState<string | null>(null);
  const authFlowEpochRef = useRef(0);
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
  const workspaceUpdateSilentFailuresRef = useRef(0);
  const [systemRestarting, setSystemRestarting] = useState(false);
  const [systemRestartMessage, setSystemRestartMessage] = useState<string | null>(null);
  const [piAppStatus, setPiAppStatus] = useState<PiAppStatusResponse | null>(null);
  const [piAppRestarting, setPiAppRestarting] = useState(false);
  const [piAppRestartMessage, setPiAppRestartMessage] = useState<string | null>(null);
  const [workspaceUpdateStatus, setWorkspaceUpdateStatus] = useState<WorkspaceUpdateStatus | null>(null);
  const [workspaceUpdateLoading, setWorkspaceUpdateLoading] = useState(false);
  const [workspaceUpdateCanceling, setWorkspaceUpdateCanceling] = useState(false);
  const [workspaceUpdateMessage, setWorkspaceUpdateMessage] = useState<string | null>(null);

  const [taskId, setTaskId] = useState("");
  const [taskLoading, setTaskLoading] = useState(false);
  const [taskResult, setTaskResult] = useState<TaskQueryResponse | null>(null);
  const [taskError, setTaskError] = useState<string | null>(null);
  const [trackingTaskId, setTrackingTaskId] = useState<string | null>(null);
  const [activeTasks, setActiveTasks] = useState<ActiveTaskItem[]>([]);
  const [activeTasksLoading, setActiveTasksLoading] = useState(false);
  const [activeTasksError, setActiveTasksError] = useState<string | null>(null);
  const [activeTasksLastUpdated, setActiveTasksLastUpdated] = useState<number | null>(null);
  const [resumeDrafts, setResumeDrafts] = useState<Record<string, string>>({});
  const [resumeSubmittingTaskId, setResumeSubmittingTaskId] = useState<string | null>(null);
  const [resumeTaskMessage, setResumeTaskMessage] = useState<string | null>(null);
  const [resumeTaskError, setResumeTaskError] = useState<string | null>(null);
  const [cancelingTaskIndex, setCancelingTaskIndex] = useState<number | null>(null);
  const [cancelTaskMessage, setCancelTaskMessage] = useState<string | null>(null);
  const [cancelTaskError, setCancelTaskError] = useState<string | null>(null);

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
  const [waLoginDialogOpen, setWaLoginDialogOpen] = useState(false);
  const [waLoginLoading, setWaLoginLoading] = useState(false);
  const [waLoginError, setWaLoginError] = useState<string | null>(null);
  const [waLoginStatus, setWaLoginStatus] = useState<WhatsappWebLoginStatus | null>(null);
  const [waWebBridgeReachable, setWaWebBridgeReachable] = useState(false);
  const [waLogoutLoading, setWaLogoutLoading] = useState(false);
  const [wechatLoginLoading, setWechatLoginLoading] = useState(false);
  const [wechatLoginError, setWechatLoginError] = useState<string | null>(null);
  const [wechatLoginStatus, setWechatLoginStatus] = useState<WechatLoginStatus | null>(null);
  const [wechatSessionKey, setWechatSessionKey] = useState<string | null>(null);
  const [wechatQrStarting, setWechatQrStarting] = useState(false);
  const [wechatQrPreviewRequested, setWechatQrPreviewRequested] = useState(false);
  const [feishuBindLoading, setFeishuBindLoading] = useState(false);
  const [feishuBindError, setFeishuBindError] = useState<string | null>(null);
  const [feishuBindSession, setFeishuBindSession] = useState<FeishuBindSessionResponse | null>(null);
  const [feishuBindQrDataUrl, setFeishuBindQrDataUrl] = useState<string | null>(null);
  const [feishuResetLoading, setFeishuResetLoading] = useState(false);
  const [diagnosticsRefreshing, setDiagnosticsRefreshing] = useState(false);
  const [currentPage, setCurrentPage] = useState<ConsolePage>(() => {
    const saved = window.localStorage.getItem(STORAGE_KEYS.currentPage);
    return saved && CONSOLE_PAGES.includes(saved as ConsolePage) ? (saved as ConsolePage) : "dashboard";
  });
  const logContainerRef = useRef<HTMLPreElement | null>(null);
  const chatImageInputRef = useRef<HTMLInputElement | null>(null);

  const t = (zh: string, en: string) => (lang === "zh" ? zh : en);
  const isAdminIdentity = authIdentity?.role?.toLowerCase() === "admin";
  const tSlash = (mixed: string) => {
    const [zh, en] = mixed.split(" / ");
    return lang === "zh" ? zh : en ?? zh;
  };
  const dateLocale = lang === "zh" ? "zh-CN" : "en-US";
  const channelLabel = (channel: ChannelName) => formatChannelLabel(channel, lang);
  const boundChannelsLabel = useMemo(() => {
    return formatBoundChannelsLabel(health?.bound_channels, lang);
  }, [health?.bound_channels, lang]);
  const formatDateTimeHuman = (raw: string | null | undefined) => {
    return formatDateTimeHumanValue(raw, dateLocale);
  };
  const formatUnixDateTime = (ts: number | null | undefined) => {
    return formatUnixDateTimeValue(ts, dateLocale);
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
      wechat: {
        summary: t("适合绑定微信用户或会话身份。", "Best for binding WeChat user or conversation identities."),
        userHint: t("通常填写微信侧用户 ID。", "Usually the WeChat-side user ID."),
        chatHint: t("如果后端区分会话，可补充会话 ID 或 peer 标识。", "If your backend distinguishes sessions, also fill the session or peer id."),
        exampleUser: "wx_user_xxxxxxxx",
        exampleChat: "wx_peer_xxxxxxxx",
        note: t("首版建议直接使用后端事件里给出的用户/会话字段，不要手动猜字段名。", "For the MVP, use the exact user/session identifiers from backend events instead of guessing field names."),
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
  const authHeaders = authMode !== "webd" && uiKey ? { "X-RustClaw-Key": uiKey } : {};
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
    const credentials =
      authMode === "webd" ? "include" : init?.credentials ?? "same-origin";
    try {
      return await fetch(targetUrl, {
        ...init,
        credentials,
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
  const {
    nniStatus,
    nniStatusLoading,
    nniStatusError,
    nniActionLoading,
    nniActionResult,
    nniActionError,
    nniActionMessage,
    nniJoined,
    nniRemoteNodes,
    nniRemoteNodeCount,
    nniHeartbeatRequestCount,
    nniHeartbeatRetryLimit,
    nniLastHeartbeatAtTs,
    nniLastHeartbeatNetworkFailures,
    nniHeartbeatRecords,
    nniHeartbeatRecordsPage,
    nniHeartbeatRecordsTotal,
    nniHeartbeatRecordsTotalPages,
    nniHeartbeatRecordsLoading,
    nniHeartbeatRecordsClearing,
    nniHeartbeatRecordsError,
    nniHeartbeatRecordsMessage,
    nniHeartbeatErrors,
    nniHeartbeatErrorsPage,
    nniHeartbeatErrorsTotal,
    nniHeartbeatErrorsTotalPages,
    nniHeartbeatErrorsLoading,
    nniHeartbeatErrorsClearing,
    nniHeartbeatErrorsError,
    nniHeartbeatErrorsMessage,
    nniConfigLoading,
    nniConfigSaving,
    nniConfigError,
    nniConfigMessage,
    setNniActionMessage,
    setNniActionError,
    fetchNniDeviceStatus,
    setNniJoinedPersisted,
    joinNni,
    testJoinNni,
    fetchNniConfig,
    saveNniConfig,
    updateNniRemoteNodes,
    fetchNniHeartbeatRecords,
    clearNniHeartbeatRecords,
    fetchNniHeartbeatErrors,
    clearNniHeartbeatErrors,
    runNniDeviceAction,
  } = useNniRuntime({ apiFetch, t, lang });
  const {
    memoryOverview,
    memoryPreferences,
    memoryFacts,
    memoryRecent,
    memoryLoading,
    memoryError,
    memoryMessage,
    memoryActionLoading,
    memorySettingsSaving,
    memoryClearScope,
    setMemoryClearScope,
    fetchMemoryData,
    deleteMemoryItem,
    expireMemoryItem,
    clearMemoryScope,
    updateMemoryLongTermEnabled,
  } = useMemoryRuntime({ apiFetch, t });
  const {
    selectedLogFile,
    setSelectedLogFile,
    logTailLines,
    setLogTailLines,
    logLoading,
    logError,
    logText,
    logLastUpdated,
    logFollowTail,
    setLogFollowTail,
    fetchLatestLog,
  } = useLogsRuntime({
    apiFetch,
    t,
    apiBase,
    currentPage,
    pollingSeconds,
    uiAuthReady,
    logContainerRef,
  });
  const {
    authKeysList,
    sortedAuthKeysList,
    authKeysLoading,
    authKeysError,
    authKeyCreateLoading,
    authKeyCreateError,
    authKeyActionLoading,
    authKeyActionError,
    authKeyCopyingTarget,
    authKeyCopiedTarget,
    newlyCreatedKey,
    webdLoginEditorKeyId,
    webdLoginUsernameDraft,
    webdLoginPasswordDraft,
    setWebdLoginUsernameDraft,
    setWebdLoginPasswordDraft,
    fetchAuthKeys,
    createAuthKey,
    promptCreateCustomAuthKey,
    copyAuthKey,
    dismissNewlyCreatedKey,
    updateAuthKey,
    promptUpdateAuthKeyRole,
    openWebdLoginEditor,
    closeWebdLoginEditor,
    deleteAuthKey,
    saveWebdLoginEditor,
    clearAuthKeysList,
  } = useAuthKeysRuntime({ apiFetch, t });
  const {
    confirmWord: factoryResetConfirmWord,
    dialogOpen: factoryResetDialogOpen,
    countdown: factoryResetCountdown,
    confirmText: factoryResetConfirmText,
    setConfirmText: setFactoryResetConfirmText,
    loading: factoryResetLoading,
    error: factoryResetError,
    result: factoryResetResult,
    canConfirm: factoryResetCanConfirm,
    openDialog: openFactoryResetDialog,
    closeDialog: closeFactoryResetDialog,
    runFactoryReset,
  } = useFactoryResetRuntime({
    apiFetch,
    t,
    onResetComplete: (result) => {
      authFlowEpochRef.current += 1;
      window.localStorage.removeItem(STORAGE_KEYS.userKey);
      window.localStorage.removeItem(STORAGE_KEYS.authMode);
      setAuthMode(null);
      setUiKey("");
      setUiKeyDraft(result.admin_user_key);
      setUiAuthReady(false);
      setUiAuthLoading(false);
      setAuthIdentity(null);
      setAuthMeError(null);
      setInteractionUserId(null);
      setInteractionChatId(null);
      setInteractionRole("-");
      setLoginTab("webd");
      setWebdUsername(result.webd_username || "rustclaw");
      setWebdPassword("");
      clearAuthKeysList();
    },
  });
  const {
    llmConfigLoading,
    llmConfigError,
    llmConfigData,
    llmDraftVendor,
    llmDraftModel,
    llmConfigSaving,
    llmConfigSaveMessage,
    llmDraftBaseUrl,
    llmDraftApiKey,
    llmDraftApiFormat,
    llmTestLoading,
    llmTestMessage,
    llmTestError,
    multimodalConfigData,
    multimodalConfigLoading,
    multimodalConfigError,
    multimodalDraft,
    multimodalConfigSaving,
    multimodalConfigSaveMessage,
    modelsAdvancedOpen,
    selectedLlmVendorInfo,
    hasCustomLlmVendor,
    hasUnsavedLlmChanges,
    llmConfigured,
    llmStepStatus,
    hasUnsavedMultimodalChanges,
    setLlmDraftModel,
    setLlmDraftBaseUrl,
    setLlmDraftApiKey,
    setLlmDraftApiFormat,
    setModelsAdvancedOpen,
    fetchLlmConfig,
    saveLlmConfig,
    testLlmConfig,
    fetchMultimodalConfig,
    saveMultimodalConfig,
    setMultimodalDraftKey,
    applyLlmVendorDraft,
    clearLlmConfigError,
  } = useModelConfigRuntime({
    apiFetch,
    t,
    onBeforeSaveLlm: () => setSystemRestartMessage(null),
  });
  const {
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
  } = useSkillsRuntime({ apiFetch, t });
  const {
    wechatConfigLoading,
    wechatConfigError,
    wechatConfigData,
    feishuConfigLoading,
    feishuConfigError,
    feishuConfigData,
    telegramConfigLoading,
    telegramConfigError,
    telegramConfigData,
    telegramConfigSaving,
    telegramConfigSaveMessage,
    primaryTelegramBot,
    telegramBotTokenConfigured,
    hasUnsavedTelegramConfigChanges,
    fetchWechatConfig,
    fetchFeishuConfig,
    fetchTelegramConfig,
    setTelegramPrimaryBotDraftField,
    saveTelegramConfig,
  } = useChannelConfigRuntime({ apiFetch, t });
  const activeUserKey = authMode === "key" && uiKey.trim() ? uiKey.trim() : "";
  const activeIdentityIds =
    activeUserKey || interactionUserId == null || interactionChatId == null
      ? {}
      : { user_id: interactionUserId, chat_id: interactionChatId };

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
    const authEpoch = authFlowEpochRef.current;
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
      if (authEpoch !== authFlowEpochRef.current) return false;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `key 校验失败 (${res.status})`);
      }
      setUiKey(normalized);
      setUiKeyDraft(normalized);
      setUiAuthReady(true);
      setAuthMeError(null);
      applyIdentity(body.data);
      setAuthMode("key");
      window.localStorage.setItem(STORAGE_KEYS.authMode, "key");
      if (persist) {
        window.localStorage.setItem(STORAGE_KEYS.userKey, normalized);
      }
      return true;
    } catch (err) {
      if (authEpoch !== authFlowEpochRef.current) return false;
      setUiAuthReady(false);
      setUiKey("");
      setAuthIdentity(null);
      setInteractionUserId(null);
      setInteractionChatId(null);
      setInteractionRole("-");
      const message = err instanceof Error ? err.message : "未知错误";
      setUiAuthError(message);
      window.localStorage.removeItem(STORAGE_KEYS.userKey);
      setAuthMode(null);
      window.localStorage.removeItem(STORAGE_KEYS.authMode);
      return false;
    } finally {
      if (authEpoch !== authFlowEpochRef.current) return;
      setUiAuthLoading(false);
    }
  };

  const logout = async () => {
    authFlowEpochRef.current += 1;
    if (authMode === "webd") {
      try {
        const webdBase = apiBase.replace(/\/$/, "");
        await fetch(`${webdBase}/webd/logout`, { method: "POST", credentials: "include" });
      } catch {
        // ignore network errors on logout
      }
    }
    window.localStorage.removeItem(STORAGE_KEYS.userKey);
    window.localStorage.removeItem(STORAGE_KEYS.authMode);
    setAuthMode(null);
    setUiKey("");
    setUiKeyDraft("");
    setUiAuthReady(false);
    setUiAuthLoading(false);
    setUiAuthError(null);
    setAuthIdentity(null);
    setAuthMeError(null);
    setInteractionUserId(null);
    setInteractionChatId(null);
    setInteractionRole("-");
  };

  const loginWebd = async () => {
    const authEpoch = authFlowEpochRef.current;
    const u = webdUsername.trim();
    if (!u || !webdPassword) {
      setUiAuthError(t("请输入用户名和密码", "Please enter username and password"));
      return;
    }
    setUiAuthLoading(true);
    setUiAuthError(null);
    const webdBase = (webdBaseUrlDraft.trim() || window.location.origin).replace(/\/$/, "");
    const loginUrl = `${webdBase}/webd/login`;
    let failingUrl = loginUrl;
    try {
      const res = await fetch(loginUrl, {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ username: u, password: webdPassword }),
      });
      if (authEpoch !== authFlowEpochRef.current) return;
      const body = (await res.json()) as { ok?: boolean; error?: string };
      if (!res.ok || !body.ok) {
        throw new Error(body.error ?? `${t("登录失败", "Sign-in failed")} (${res.status})`);
      }
      setBaseUrl(webdBase);
      window.localStorage.setItem(STORAGE_KEYS.baseUrl, webdBase);
      window.localStorage.removeItem(STORAGE_KEYS.userKey);
      setUiKey("");
      setUiKeyDraft("");
      setWebdPassword("");
      setAuthMode("webd");
      window.localStorage.setItem(STORAGE_KEYS.authMode, "webd");
      setUiAuthReady(true);
      setAuthMeError(null);
      const meUrl = `${webdBase}/v1/auth/me`;
      failingUrl = meUrl;
      const meRes = await fetch(meUrl, { credentials: "include" });
      if (authEpoch !== authFlowEpochRef.current) return;
      const meBody = (await meRes.json()) as ApiResponse<AuthIdentityResponse>;
      if (!meRes.ok || !meBody.ok || !meBody.data) {
        setUiAuthReady(false);
        setAuthMode(null);
        window.localStorage.removeItem(STORAGE_KEYS.authMode);
        setUiAuthError(
          t("登录成功但无法获取身份信息", "Signed in but failed to load profile"),
        );
        return;
      }
      applyIdentity(meBody.data);
    } catch (err) {
      if (authEpoch !== authFlowEpochRef.current) return;
      setUiAuthReady(false);
      setAuthIdentity(null);
      setInteractionUserId(null);
      setInteractionChatId(null);
      setInteractionRole("-");
      const message =
        err instanceof Error ? normalizeFetchError(err, failingUrl) : t("未知错误", "Unknown error");
      setUiAuthError(message);
    } finally {
      if (authEpoch !== authFlowEpochRef.current) return;
      setUiAuthLoading(false);
    }
  };

  const fetchHealth = async (options?: { silent?: boolean }) => {
    if (!options?.silent) {
      setLoading(true);
      setError(null);
    }
    try {
      const res = await apiFetch(`/v1/health`);
      const body = (await res.json()) as ApiResponse<HealthResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `health 请求失败 (${res.status})`);
      }
      setHealth(body.data);
    } catch (err) {
      if (!options?.silent) {
        const message = err instanceof Error ? err.message : "未知错误";
        setError(message);
      }
    } finally {
      if (!options?.silent) {
        setLoading(false);
      }
    }
  };

  const {
    channelBindingChannel,
    setChannelBindingChannel,
    channelBindingExternalUserId,
    setChannelBindingExternalUserId,
    channelBindingExternalChatId,
    setChannelBindingExternalChatId,
    channelResolveLoading,
    channelResolveError,
    channelResolveResult,
    channelBindLoading,
    channelBindError,
    channelBindMessage,
    resolveChannelBinding,
    bindChannelToCurrentKey,
  } = useChannelBindingRuntime({
    apiFetch,
    t,
    activeUserKey,
    channelLabel,
    onIdentityApplied: applyIdentity,
    onHealthRefresh: async () => {
      await fetchHealth();
    },
  });
  const {
    serviceActionLoading,
    serviceActionMessage,
    setServiceActionMessage,
    controlService,
  } = useServiceActionsRuntime({
    apiFetch,
    t,
    onHealthRefresh: async () => {
      await fetchHealth();
    },
  });

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
      setServiceActionMessage({
        tone: "success",
        text: t("已发起 WhatsApp Web 退出登录。", "WhatsApp Web logout requested."),
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setWaLoginError(message);
    } finally {
      setWaLogoutLoading(false);
    }
  };

  const fetchWechatLoginStatus = async (silent = false) => {
    if (!silent) {
      setWechatLoginLoading(true);
      setWechatLoginError(null);
    }
    try {
      const res = await apiFetch(`/v1/wechat/login-status`);
      const body = (await res.json()) as ApiResponse<WechatLoginStatus>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `获取微信登录状态失败 (${res.status})`);
      }
      setWechatLoginStatus(body.data);
      if (body.data.qr_ready && body.data.session_key) {
        setWechatSessionKey(body.data.session_key);
      } else if (!body.data.qr_ready || body.data.connected) {
        setWechatSessionKey(null);
      }
      if (!silent) {
        setWechatLoginError(null);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      if (!silent) {
        setWechatLoginError(message);
      }
    } finally {
      if (!silent) {
        setWechatLoginLoading(false);
      }
    }
  };

  const startWechatQrLogin = async (force = true) => {
    setWechatQrStarting(true);
    setWechatQrPreviewRequested(true);
    setWechatLoginError(null);
    setWechatSessionKey(null);
    setWechatLoginStatus((prev) => ({
      ...(prev ?? {}),
      connected: false,
      qr_ready: false,
      qrcode_url: null,
      qr_status: "generating",
      message: t("正在生成二维码...", "Generating QR code..."),
      last_error: null,
      status: "qr_generating",
      last_update_ts: Date.now(),
    }));
    try {
      const res = await apiFetch(`/v1/wechat/login-qr/start`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ force }),
      });
      const body = (await res.json()) as ApiResponse<WechatQrStartResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `生成微信二维码失败 (${res.status})`);
      }
      setWechatSessionKey(body.data.session_key);
      setWechatLoginStatus((prev) => ({
        ...(prev ?? {}),
        connected: false,
        qr_ready: true,
        qr_status: "wait",
        qrcode_url: body.data.qrcode_url,
        message: body.data.message,
        last_error: null,
        status: "qr_ready",
        last_update_ts: Date.now(),
      }));
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setWechatLoginError(message);
    } finally {
      setWechatQrStarting(false);
    }
  };

  const pollWechatQrLogin = async (sessionKey: string) => {
    try {
      const res = await apiFetch(`/v1/wechat/login-qr/wait`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ session_key: sessionKey, timeout_ms: 1500 }),
      });
      const body = (await res.json()) as ApiResponse<WechatQrWaitResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `等待微信登录失败 (${res.status})`);
      }
      if (body.data.connected) {
        setWechatSessionKey(null);
        await fetchWechatLoginStatus(true);
      } else if (body.data.message && !body.data.message.includes("超时")) {
        setWechatLoginStatus((prev) => ({
          ...(prev ?? {}),
          connected: false,
          qr_ready: true,
          qr_status: body.data.qr_status ?? prev?.qr_status ?? "wait",
          message: body.data.message,
          status: "qr_ready",
        }));
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      const transientQrPollFailure =
        message.includes("get_qrcode_status") ||
        message.includes("poll QR status failed") ||
        message.includes("wechat QR wait failed");
      if (transientQrPollFailure) {
        setWechatLoginStatus((prev) => (
          prev
            ? {
                ...prev,
                message: t(
                  "二维码已经生成，可以继续扫码。状态轮询刚刚抖了一下，界面会继续自动刷新。",
                  "The QR code is ready and can still be scanned. Status polling briefly failed and will retry automatically.",
                ),
              }
            : prev
        ));
        return;
      }
      if (!message.includes("超时")) {
        setWechatLoginError(message);
      }
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

  const beginFeishuBind = async () => {
    setFeishuBindLoading(true);
    setFeishuBindError(null);
    try {
      const session = await startFeishuBindSession(apiFetch);
      setFeishuBindSession(session);
    } catch (err) {
      setFeishuBindError(err instanceof Error ? err.message : "未知错误");
    } finally {
      setFeishuBindLoading(false);
    }
  };

  const refreshFeishuBindSession = async (sessionId: number, silent = false) => {
    if (!silent) {
      setFeishuBindLoading(true);
      setFeishuBindError(null);
    }
    try {
      const session = await fetchFeishuBindSession(apiFetch, sessionId);
      setFeishuBindSession(session);
      if (session.status === "bound") {
        await fetchFeishuConfig();
        await fetchHealth();
      }
      return session;
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      if (!silent) {
        setFeishuBindError(message);
      }
      return null;
    } finally {
      if (!silent) {
        setFeishuBindLoading(false);
      }
    }
  };

  const resetFeishuSetup = async () => {
    const confirmed = window.confirm(
      t(
        "确认重置飞书接入吗？这会清空飞书配置里的关键凭据，并删除当前 Key 的飞书绑定状态与待绑定会话。",
        "Reset Feishu setup? This clears the Feishu credentials and removes the current key's Feishu bindings and pending setup sessions.",
      ),
    );
    if (!confirmed) return;
    setFeishuResetLoading(true);
    setFeishuBindError(null);
    try {
      const res = await apiFetch(`/v1/admin/feishu/reset`, { method: "POST" });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `飞书重置失败 (${res.status})`);
      }
      setFeishuBindSession(null);
      setFeishuBindQrDataUrl(null);
      await fetchFeishuConfig();
      await fetchHealth();
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setFeishuBindError(message);
    } finally {
      setFeishuResetLoading(false);
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

  const renderMultimodalModelMeta = (key: MultimodalKey) => {
    const metaView = buildMultimodalMetaView(multimodalConfigData?.[key], lang);
    if (!metaView) return null;
    return (
      <div className="flex flex-wrap items-center gap-1.5 pl-[7.5rem] text-[11px] text-white/55 max-sm:pl-0">
        {metaView.capabilityBadges.map((capability) => (
          <span key={`capability-${key}-${capability}`} className="rounded-md border border-sky-400/25 bg-sky-500/10 px-2 py-1 text-sky-100/85">
            {capability}
          </span>
        ))}
        {metaView.visibleModels.length > 0 ? (
          <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-white/70">
            {t("可选模型", "Models")}: {metaView.visibleModels.join(", ")}
            {metaView.hiddenModelCount > 0 ? ` +${metaView.hiddenModelCount}` : ""}
          </span>
        ) : null}
        {metaView.metaBadges.map((badge) => (
          <span key={`meta-${key}-${badge}`} className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-white/65">
            {badge}
          </span>
        ))}
      </div>
    );
  };

  const fetchWorkspaceUpdateStatus = async (silent = false): Promise<WorkspaceUpdateStatus | null> => {
    if (!silent) {
      setWorkspaceUpdateLoading(true);
      setWorkspaceUpdateMessage(null);
    }
    try {
      const res = await apiFetch("/v1/admin/workspace-update");
      const body = (await res.json()) as ApiResponse<WorkspaceUpdateStatus>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `更新状态查询失败 (${res.status})`);
      }
      setWorkspaceUpdateStatus(body.data);
      return body.data;
    } catch (err) {
      if (!silent) {
        const message = err instanceof Error ? err.message : "未知错误";
        setWorkspaceUpdateMessage(`${t("查询更新状态失败", "Failed to query update status")}: ${message}`);
      }
      return null;
    } finally {
      if (!silent) {
        setWorkspaceUpdateLoading(false);
      }
    }
  };

  const startWorkspaceUpdate = async (mode: WorkspaceUpdateMode = "full") => {
    const modeConfig: Record<WorkspaceUpdateMode, { confirm: string; endpoint: string; started: string }> = {
      full: {
        confirm: t(
          "系统会先正常拉取远端版本；如果拉取被本地冲突文件阻挡，只覆盖这些冲突文件，其他本地改动和额外文件保持不动。随后会完整编译并重启 clawd。确认现在开始吗？",
          "The system will pull the remote version first. If local conflicting files block the pull, only those conflict files will be overwritten; other local changes and extra files are left untouched. It will then run a full build and restart clawd. Start now?",
        ),
        endpoint: "/v1/admin/workspace-update",
        started: t("更新已开始，下面会自动刷新进度。", "Update started. Progress will refresh automatically."),
      },
      ui_only: {
        confirm: t(
          "只编译并部署 UI，不拉取远端版本，也不重启 clawd。确认现在开始吗？",
          "Build and deploy the UI only. This will not pull the remote version or restart clawd. Start now?",
        ),
        endpoint: "/v1/admin/workspace-update/build-ui",
        started: t("UI 编译已开始，下面会自动刷新进度。", "UI build started. Progress will refresh automatically."),
      },
      clawd_only: {
        confirm: t(
          "只编译 clawd，完成后只重启 clawd；不拉取远端版本，也不编译 UI。确认现在开始吗？",
          "Build clawd only, then restart clawd only. This will not pull the remote version or build the UI. Start now?",
        ),
        endpoint: "/v1/admin/workspace-update/build-clawd",
        started: t("clawd 编译已开始，下面会自动刷新进度。", "clawd build started. Progress will refresh automatically."),
      },
      release_deploy: {
        confirm: t(
          "直接下载 GitHub Releases 里适合当前机器的预编译包并部署；会保留 configs、data、logs 和 .pids，完成后重启 clawd。确认现在开始吗？",
          "Download and deploy the prebuilt GitHub Release package for this machine. configs, data, logs, and .pids will be preserved, then clawd will restart. Start now?",
        ),
        endpoint: "/v1/admin/workspace-update/deploy-release",
        started: t("Release 包部署已开始，下面会自动刷新进度。", "Release package deployment started. Progress will refresh automatically."),
      },
    };
    const selectedMode = modeConfig[mode];
    const confirmed = window.confirm(selectedMode.confirm);
    if (!confirmed) return;
    setWorkspaceUpdateLoading(true);
    setWorkspaceUpdateMessage(null);
    try {
      const res = await apiFetch(selectedMode.endpoint, { method: "POST" });
      const body = (await res.json()) as ApiResponse<WorkspaceUpdateStatus>;
      if (!res.ok || !body.ok || !body.data) {
        if (res.status === 409 && body.data) {
          setWorkspaceUpdateStatus(body.data);
          setWorkspaceUpdateMessage(
            t("更新已经在进行中，下面会继续刷新现有进度。", "An update is already running. Existing progress will keep refreshing."),
          );
          return;
        }
        throw new Error(body.error || `更新启动失败 (${res.status})`);
      }
      setWorkspaceUpdateStatus(body.data);
      setWorkspaceUpdateMessage(selectedMode.started);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setWorkspaceUpdateMessage(`${t("启动更新失败", "Failed to start update")}: ${message}`);
    } finally {
      setWorkspaceUpdateLoading(false);
    }
  };

  const cancelWorkspaceUpdate = async () => {
    const confirmed = window.confirm(
      t(
        workspaceUpdateStatus?.mode === "release_deploy"
          ? "停止当前部署？已经完成的下载或文件复制不会自动回滚，后续可重新点击下载 Release 部署。"
          : "停止当前编译？已经完成的拉取或文件复制不会自动回滚，后续可重新点击完整编译。",
        workspaceUpdateStatus?.mode === "release_deploy"
          ? "Stop the current deployment? Completed download or copy steps will not be rolled back. You can deploy the Release again later."
          : "Stop the current build? Completed pull or copy steps will not be rolled back. You can run Build All again later.",
      ),
    );
    if (!confirmed) return;
    setWorkspaceUpdateCanceling(true);
    setWorkspaceUpdateMessage(null);
    try {
      const res = await apiFetch("/v1/admin/workspace-update/cancel", { method: "POST" });
      const body = (await res.json()) as ApiResponse<WorkspaceUpdateStatus>;
      if (!res.ok || !body.ok || !body.data) {
        if (body.data) setWorkspaceUpdateStatus(body.data);
        throw new Error(body.error || `停止编译失败 (${res.status})`);
      }
      setWorkspaceUpdateStatus(body.data);
      setWorkspaceUpdateMessage(t("已请求停止编译，正在结束当前进程。", "Stop requested. Ending the current build process."));
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setWorkspaceUpdateMessage(`${t("停止编译失败", "Failed to stop build")}: ${message}`);
    } finally {
      setWorkspaceUpdateCanceling(false);
    }
  };

  const restartSystem = async () => {
    setSystemRestarting(true);
    setSystemRestartMessage(null);
    clearLlmConfigError();
    clearSkillsConfigError();
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

  const fetchPiAppStatus = async () => {
    try {
      const res = await apiFetch(`/v1/pi-app/status`);
      const body = (await res.json()) as ApiResponse<PiAppStatusResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `Pi App status failed (${res.status})`);
      }
      setPiAppStatus(body.data);
    } catch {
      setPiAppStatus(null);
    }
  };

  const restartPiApp = async () => {
    setPiAppRestarting(true);
    setPiAppRestartMessage(null);
    try {
      const res = await apiFetch(`/v1/pi-app/restart`, { method: "POST" });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `Pi App restart failed (${res.status})`);
      }
      setPiAppRestartMessage(t("已发起 Pi App 小程序重启。", "Pi App restart requested."));
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setPiAppRestartMessage(`${t("Pi App 重启失败", "Pi App restart failed")}: ${message}`);
    } finally {
      setPiAppRestarting(false);
      void fetchPiAppStatus();
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

  const fetchActiveTasks = async (silent = false): Promise<ActiveTaskItem[]> => {
    if (interactionUserId == null || interactionChatId == null) {
      if (!silent) {
        setActiveTasksError(t("本地身份还没有加载完成。", "Local identity is not loaded yet."));
      }
      return [];
    }
    if (!silent) {
      setActiveTasksLoading(true);
      setActiveTasksError(null);
    }
    try {
      const res = await apiFetch(`/v1/tasks/active`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          user_id: interactionUserId,
          chat_id: interactionChatId,
        }),
      });
      const body = (await res.json()) as ApiResponse<ActiveTasksResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `任务列表读取失败 (${res.status})`);
      }
      const tasks = body.data.tasks ?? [];
      setActiveTasks(tasks);
      setActiveTasksError(null);
      setActiveTasksLastUpdated(Date.now());
      return tasks;
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setActiveTasksError(message);
      return [];
    } finally {
      if (!silent) {
        setActiveTasksLoading(false);
      }
    }
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

  const submitResumeForTask = async (resumeTaskId: string) => {
    const text = (resumeDrafts[resumeTaskId] ?? "").trim();
    if (!text) {
      setResumeTaskMessage(null);
      setResumeTaskError(t("请先填写要继续发送的内容。", "Enter the follow-up text first."));
      return;
    }
    setResumeSubmittingTaskId(resumeTaskId);
    setResumeTaskMessage(null);
    setResumeTaskError(null);
    try {
      const payload: Record<string, unknown> = {
        text,
        resume_task_id: resumeTaskId,
        resume_trigger: "user_followup",
      };
      const adapterName = interactionAdapter.trim();
      if (adapterName) {
        payload.adapter = adapterName;
      }
      const body: Record<string, unknown> = {
        channel: interactionChannel,
        kind: "ask",
        payload,
        ...(activeUserKey ? { user_key: activeUserKey } : {}),
        ...activeIdentityIds,
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
        throw new Error(resp.error || `resume submit failed (${res.status})`);
      }

      setResumeDrafts((prev) => {
        const next = { ...prev };
        delete next[resumeTaskId];
        return next;
      });
      setResumeTaskMessage(t("已提交继续执行请求。", "Resume request submitted."));
      setInteractionSubmittedTaskId(resp.data.task_id);
      setTaskId(resp.data.task_id);
      setTrackingTaskId(resp.data.task_id);
      setTaskResult(null);
      setTaskError(null);
      void fetchActiveTasks(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : "Unknown";
      setResumeTaskError(message);
    } finally {
      setResumeSubmittingTaskId(null);
    }
  };

  const cancelActiveTask = async (item: ActiveTaskItem) => {
    if (interactionUserId == null || interactionChatId == null) {
      setCancelTaskMessage(null);
      setCancelTaskError(t("本地身份还没有加载完成。", "Local identity is not loaded yet."));
      return;
    }
    setCancelingTaskIndex(item.index);
    setCancelTaskMessage(null);
    setCancelTaskError(null);
    try {
      const res = await apiFetch(`/v1/tasks/cancel-one`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          user_id: interactionUserId,
          chat_id: interactionChatId,
          index: item.index,
        }),
      });
      const body = (await res.json()) as ApiResponse<{ canceled?: number }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `cancel task failed (${res.status})`);
      }
      setCancelTaskMessage(t("任务取消请求已提交。", "Task cancel request submitted."));
      await fetchActiveTasks(true);
      if (taskResult?.task_id === item.task_id) {
        void queryTaskById(item.task_id, false);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : "Unknown";
      setCancelTaskError(message);
    } finally {
      setCancelingTaskIndex(null);
    }
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
        channel: interactionChannel,
        kind: interactionKind,
        payload,
        ...(activeUserKey ? { user_key: activeUserKey } : {}),
        ...activeIdentityIds,
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
      void fetchActiveTasks(true);
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
        channel: interactionChannel,
        kind: "ask",
        ...(activeUserKey ? { user_key: activeUserKey } : {}),
        ...activeIdentityIds,
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
    if (authMode === "webd") {
      if (uiAuthLoading) return;
      if (uiAuthReady && authIdentity) return;
      const authEpoch = authFlowEpochRef.current;
      void (async () => {
        setUiAuthLoading(true);
        setUiAuthError(null);
        try {
          const targetUrl = `${apiBase.replace(/\/$/, "")}/v1/auth/me`;
          const res = await fetch(targetUrl, { credentials: "include" });
          if (authEpoch !== authFlowEpochRef.current) return;
          const body = (await res.json()) as ApiResponse<AuthIdentityResponse>;
          if (!res.ok || !body.ok || !body.data) {
            setUiAuthReady(false);
            setAuthIdentity(null);
            setInteractionUserId(null);
            setInteractionChatId(null);
            setInteractionRole("-");
            setAuthMode(null);
            window.localStorage.removeItem(STORAGE_KEYS.authMode);
            setUiAuthError(
              t("Web 会话已失效，请重新登录", "Web session expired; please sign in again."),
            );
            return;
          }
          applyIdentity(body.data);
          setUiAuthReady(true);
          setAuthMeError(null);
        } catch (err) {
          if (authEpoch !== authFlowEpochRef.current) return;
          setUiAuthReady(false);
          setAuthMode(null);
          window.localStorage.removeItem(STORAGE_KEYS.authMode);
          const message =
            err instanceof Error ? normalizeFetchError(err, `${apiBase.replace(/\/$/, "")}/v1/auth/me`) : t("未知错误", "Unknown error");
          setUiAuthError(message);
        } finally {
          if (authEpoch !== authFlowEpochRef.current) return;
          setUiAuthLoading(false);
        }
      })();
      return;
    }
    if (authMode !== "key") {
      setUiAuthReady(false);
      setAuthIdentity(null);
      setInteractionUserId(null);
      setInteractionChatId(null);
      setInteractionRole("-");
      return;
    }
    if (uiAuthLoading) return;
    if (uiAuthReady && authIdentity) return;
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
  }, [apiBase, authMode, uiKey, uiAuthLoading, uiAuthReady, authIdentity]);

  useEffect(() => {
    if (!uiAuthReady || pollingSeconds <= 0) return;
    void fetchHealth();
    void fetchAuthMe();
    void fetchSkills();
    void fetchSkillsConfig();
    void fetchLlmConfig();
    void fetchMultimodalConfig();
    void fetchNniConfig();
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
    window.localStorage.setItem(STORAGE_KEYS.webdBaseUrl, webdBaseUrlDraft);
  }, [webdBaseUrlDraft]);

  useEffect(() => {
    if (uiKey) {
      window.localStorage.setItem(STORAGE_KEYS.userKey, uiKey);
    } else {
      window.localStorage.removeItem(STORAGE_KEYS.userKey);
    }
  }, [uiKey]);

  useEffect(() => {
    if (authMode === null) {
      window.localStorage.removeItem(STORAGE_KEYS.authMode);
    } else {
      window.localStorage.setItem(STORAGE_KEYS.authMode, authMode);
    }
  }, [authMode]);

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
    window.localStorage.setItem(STORAGE_KEYS.themeMode, "dark");
    document.documentElement.dataset.theme = "dark";
  }, []);

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
    void fetchWechatConfig();
    void fetchFeishuConfig();
    void fetchTelegramConfig();
    void fetchLlmConfig();
    void fetchNniConfig();
    void fetchLocalInteractionContext();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady || !isAdminIdentity) return;
    void fetchWorkspaceUpdateStatus(true);
    void fetchPiAppStatus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady, isAdminIdentity]);

  useEffect(() => {
    if (!uiAuthReady || !isAdminIdentity) return;
    const status = workspaceUpdateStatus?.status;
    if (status !== "running" && status !== "restarting") return;
    const interval = window.setInterval(async () => {
      const next = await fetchWorkspaceUpdateStatus(true);
      if (!next) {
        workspaceUpdateSilentFailuresRef.current += 1;
        if (status === "restarting" && workspaceUpdateSilentFailuresRef.current >= 3) {
          setWorkspaceUpdateMessage(
            t(
              "RustClaw 可能仍在重启。你可以稍后点击“检查远端版本”确认服务是否恢复。",
              "RustClaw may still be restarting. You can click Check remote shortly to confirm recovery.",
            ),
          );
        }
        return;
      }
      workspaceUpdateSilentFailuresRef.current = 0;
      if (next?.status === "restarting") {
        await sleep(1800);
        await fetchHealth({ silent: true });
      }
    }, 2500);
    return () => window.clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady, isAdminIdentity, workspaceUpdateStatus?.status]);

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
    if (currentPage !== "tasks") return;
    if (interactionUserId == null || interactionChatId == null) return;
    void fetchActiveTasks(true);
    const interval = window.setInterval(() => {
      void fetchActiveTasks(true);
    }, 5000);
    return () => window.clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPage, apiBase, uiAuthReady, interactionUserId, interactionChatId]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (currentPage === "channels") {
      void fetchAuthKeys();
      void fetchWechatConfig();
      void fetchFeishuConfig();
      void fetchTelegramConfig();
    }
  }, [currentPage, uiAuthReady, isAdminIdentity]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (currentPage !== "nni") return;
    void fetchNniDeviceStatus();
    void fetchNniConfig(true);
    void fetchNniHeartbeatErrors(nniHeartbeatErrorsPage);
    void fetchNniHeartbeatRecords(nniHeartbeatRecordsPage);
    const timer = window.setInterval(() => {
      void fetchNniConfig(true);
      void fetchNniHeartbeatErrors(nniHeartbeatErrorsPage, true);
      void fetchNniHeartbeatRecords(nniHeartbeatRecordsPage, true);
    }, 60_000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPage, apiBase, uiAuthReady, nniHeartbeatErrorsPage, nniHeartbeatRecordsPage]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (currentPage !== "memory") return;
    void fetchMemoryData();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPage, apiBase, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!waLoginDialogOpen) return;
    if (health?.whatsapp_web_healthy !== true) {
      setWaWebBridgeReachable(false);
      setWaLoginError(null);
      return;
    }
    void fetchWhatsappWebLoginStatus();
    const timer = window.setInterval(() => {
      void fetchWhatsappWebLoginStatus(true);
    }, 2000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [waLoginDialogOpen, apiBase, uiAuthReady, health?.whatsapp_web_healthy]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (health?.whatsapp_web_healthy !== true) {
      setWaWebBridgeReachable(false);
      return;
    }
    // Keep whatsapp web login status fresh for row actions.
    void fetchWhatsappWebLoginStatus(true);
    const timer = window.setInterval(() => {
      void fetchWhatsappWebLoginStatus(true);
    }, 5000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady, health?.whatsapp_web_healthy]);

  useEffect(() => {
    if (!uiAuthReady) return;
    void fetchWechatLoginStatus(true);
    const timer = window.setInterval(() => {
      void fetchWechatLoginStatus(true);
    }, 5000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!wechatSessionKey) return;
    if (wechatLoginStatus?.connected) return;
    const timer = window.setInterval(() => {
      void pollWechatQrLogin(wechatSessionKey);
      void fetchWechatLoginStatus(true);
    }, 2000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wechatSessionKey, wechatLoginStatus?.connected, apiBase, uiAuthReady]);

  useEffect(() => {
    const entryUrl = feishuBindSession?.entry_url?.trim() ?? "";
    if (!entryUrl) {
      setFeishuBindQrDataUrl(null);
      return;
    }
    let cancelled = false;
    void QRCode.toDataURL(entryUrl, {
      width: 288,
      margin: 1,
      color: {
        dark: "#111827",
        light: "#ffffff",
      },
    })
      .then((url) => {
        if (!cancelled) {
          setFeishuBindQrDataUrl(url);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setFeishuBindError(err instanceof Error ? err.message : "未知错误");
          setFeishuBindQrDataUrl(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [feishuBindSession?.entry_url]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!feishuBindSession) return;
    if (isFeishuBindTerminalStatus(feishuBindSession.status)) return;
    const timer = window.setInterval(() => {
      void refreshFeishuBindSession(feishuBindSession.session_id, true);
    }, 1800);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [uiAuthReady, feishuBindSession?.session_id, feishuBindSession?.status]);

  const maskedSavedUiKey = useMemo(() => {
    if (authMode === "webd") return "";
    return maskStoredKey(uiKey);
  }, [uiKey, authMode]);
  const maskedIdentityKey = useMemo(() => {
    const currentKey = authIdentity?.user_key?.trim() || "";
    return currentKey ? maskStoredKey(currentKey) : "";
  }, [authIdentity?.user_key]);
  const adapterHealthRows = useMemo<AdapterHealthRow[]>(() => {
    const servicePriority: Record<AdapterHealthRow["key"], number> = {
      wechat_bot: 0,
      telegram_bot: 1,
      feishu_bot: 2,
      lark_bot: 3,
      whatsapp_cloud: 4,
      whatsapp_web: 5,
    };
    const rows: AdapterHealthRow[] = [
      {
        key: "wechat_bot",
        label: serviceDisplayName("wechat_bot", t),
        serviceName: "wechatd",
        healthy: health?.wechatd_healthy,
        processCount: health?.wechatd_process_count,
        memoryRssBytes: health?.wechatd_memory_rss_bytes,
      },
      {
        key: "telegram_bot",
        label: serviceDisplayName("telegram_bot", t),
        serviceName: "telegramd",
        healthy: health?.telegram_bot_healthy ?? health?.telegramd_healthy,
        processCount: health?.telegram_bot_process_count ?? health?.telegramd_process_count,
        memoryRssBytes: health?.telegram_bot_memory_rss_bytes ?? health?.telegramd_memory_rss_bytes,
      },
      {
        key: "whatsapp_cloud",
        label: serviceDisplayName("whatsapp_cloud", t),
        serviceName: "whatsappd",
        healthy: health?.whatsapp_cloud_healthy ?? health?.whatsappd_healthy,
        processCount: health?.whatsapp_cloud_process_count ?? health?.whatsappd_process_count,
        memoryRssBytes: health?.whatsapp_cloud_memory_rss_bytes ?? health?.whatsappd_memory_rss_bytes,
      },
      {
        key: "whatsapp_web",
        label: serviceDisplayName("whatsapp_web", t),
        serviceName: "whatsapp_webd",
        healthy: health?.whatsapp_web_healthy,
        processCount: health?.whatsapp_web_process_count,
        memoryRssBytes: health?.whatsapp_web_memory_rss_bytes,
      },
      {
        key: "feishu_bot",
        label: serviceDisplayName("feishu_bot", t),
        serviceName: "feishud",
        healthy: health?.feishud_healthy,
        processCount: health?.feishud_process_count,
        memoryRssBytes: health?.feishud_memory_rss_bytes,
      },
      {
        key: "lark_bot",
        label: serviceDisplayName("lark_bot", t),
        serviceName: "larkd",
        healthy: health?.larkd_healthy,
        processCount: health?.larkd_process_count,
        memoryRssBytes: health?.larkd_memory_rss_bytes,
      },
    ];
    return [...rows].sort((a, b) => (servicePriority[a.key] ?? 999) - (servicePriority[b.key] ?? 999));
  }, [health, lang]);
  const serviceStatusRows = useMemo<ServiceStatusRow[]>(() => {
    return adapterHealthRows.map((row) => {
      if (row.key === "wechat_bot") {
        if (row.healthy === true && wechatLoginStatus?.connected === true) {
          return {
            ...row,
            category: "ready",
            statusLabel: t("已登录可用", "Connected and ready"),
            detail: t("进程正常，微信通道已完成登录。", "Daemon is healthy and the WeChat channel is connected."),
          };
        }
        if (row.healthy === true) {
          return {
            ...row,
            category: "attention",
            statusLabel: t("进程已起，待登录", "Running, login required"),
            detail: wechatLoginStatus?.qr_status === "scaned"
              ? t("二维码已被扫描，请在手机上完成确认。", "The QR code was scanned. Please confirm on the phone.")
              : wechatLoginStatus?.qr_ready
                ? t("二维码已就绪，可以直接扫码登录微信。", "QR is ready and can be scanned to log in.")
                : t("进程已启动，但当前还没有可用微信登录态。", "Daemon is running, but there is no active WeChat login yet."),
          };
        }
        if (row.healthy === false) {
          return {
            ...row,
            category: "stopped",
            statusLabel: t("进程未运行", "Daemon stopped"),
            detail: t("先启动 wechatd，再在下方生成二维码完成登录。", "Start wechatd first, then generate a QR code below to log in."),
          };
        }
        return {
          ...row,
          category: "unknown",
          statusLabel: t("状态未知", "Unknown"),
          detail: t("暂时无法判断 wechatd 当前状态。", "Unable to determine the current wechatd state."),
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
  }, [adapterHealthRows, lang, wechatLoginStatus]);
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
  const healthStatusLoading = health == null && error == null;
  const wechatStatusLoading = healthStatusLoading || wechatConfigLoading || (wechatConfigData == null && wechatConfigError == null);
  const telegramStatusLoading = healthStatusLoading || telegramConfigLoading || (telegramConfigData == null && telegramConfigError == null);
  const feishuStatusLoading = healthStatusLoading || feishuConfigLoading || (feishuConfigData == null && feishuConfigError == null);
  const wechatStepStatus = useMemo<"done" | "attention" | "todo">(() => {
    if (!wechatConfigData?.enabled) return "todo";
    if (health?.wechatd_healthy === true && wechatLoginStatus?.connected) return "done";
    return "attention";
  }, [health?.wechatd_healthy, wechatConfigData?.enabled, wechatLoginStatus?.connected]);
  const telegramStepStatus = useMemo<"done" | "attention" | "todo">(() => {
    if (!telegramBotTokenConfigured) return "todo";
    if (health?.telegramd_healthy === true) return "done";
    return "attention";
  }, [health?.telegramd_healthy, telegramBotTokenConfigured]);
  const dashboardCommunicationRows = useMemo<DashboardCommunicationRow[]>(() => {
    const gatewayKinds = new Set((health?.gateway_instance_statuses ?? []).map((item) => item.kind));
    const enabledKeys = new Set<string>();
    if (wechatConfigData?.enabled) enabledKeys.add("wechat_bot");
    if (telegramBotTokenConfigured || (health?.telegram_configured_bot_count ?? 0) > 0 || gatewayKinds.has("telegram")) {
      enabledKeys.add("telegram_bot");
    }
    if (feishuConfigData?.enabled || feishuConfigData?.bind_ready || gatewayKinds.has("feishu")) {
      enabledKeys.add("feishu_bot");
    }
    if (gatewayKinds.has("lark") || health?.larkd_healthy != null || health?.larkd_process_count != null) {
      enabledKeys.add("lark_bot");
    }
    if (gatewayKinds.has("whatsapp_cloud") || health?.whatsapp_cloud_healthy != null || health?.whatsapp_cloud_process_count != null) {
      enabledKeys.add("whatsapp_cloud");
    }
    if (gatewayKinds.has("whatsapp_web") || health?.whatsapp_web_healthy != null || health?.whatsapp_web_process_count != null) {
      enabledKeys.add("whatsapp_web");
    }

    return serviceStatusRows
      .filter((row) => enabledKeys.has(row.key) && row.healthy === true)
      .map((row) => {
        const usesSharedGatewayMemory =
          row.memoryRssBytes == null &&
          row.healthy === true &&
          ["telegram_bot", "whatsapp_cloud", "whatsapp_web", "feishu_bot", "lark_bot"].includes(row.key) &&
          (health?.channel_gateway_memory_rss_bytes ?? null) != null;
        const memoryValue = usesSharedGatewayMemory ? health?.channel_gateway_memory_rss_bytes ?? null : row.memoryRssBytes;
        return {
          ...row,
          memoryLabel: formatBytes(memoryValue),
          usesSharedGatewayMemory,
        };
      });
  }, [
    feishuConfigData?.bind_ready,
    feishuConfigData?.enabled,
    health?.channel_gateway_memory_rss_bytes,
    health?.gateway_instance_statuses,
    health?.larkd_healthy,
    health?.larkd_process_count,
    health?.telegram_configured_bot_count,
    health?.whatsapp_cloud_healthy,
    health?.whatsapp_cloud_process_count,
    health?.whatsapp_web_healthy,
    health?.whatsapp_web_process_count,
    serviceStatusRows,
    telegramBotTokenConfigured,
    wechatConfigData?.enabled,
  ]);
  const feishuBindStatusCopy = useMemo(
    () => getFeishuBindStatusCopy(feishuBindSession?.status ?? "pending"),
    [feishuBindSession?.status],
  );
  const feishuCurrentKeyBound = feishuConfigData?.current_key_bound === true;
  const feishuSetupGuidance = useMemo(
    () =>
      getFeishuSetupGuidance({
        bindReady: feishuConfigData?.bind_ready ?? false,
        hasUnsavedConfigChanges: false,
        serviceHealthy: health?.feishud_healthy === true,
        hasActiveSession: Boolean(
          feishuBindSession && !isFeishuBindTerminalStatus(feishuBindSession.status),
        ),
        bound: feishuCurrentKeyBound || feishuBindSession?.status === "bound",
      }),
    [feishuBindSession, feishuConfigData?.bind_ready, feishuCurrentKeyBound, health?.feishud_healthy],
  );
  const feishuStepStatus = useMemo<"done" | "attention" | "todo">(() => {
    return getFeishuStepStatus({
      bindReady: feishuConfigData?.bind_ready ?? false,
      serviceHealthy: health?.feishud_healthy === true,
      session: feishuBindSession,
      currentKeyBound: feishuCurrentKeyBound,
    });
  }, [feishuBindSession, feishuConfigData?.bind_ready, feishuCurrentKeyBound, health?.feishud_healthy]);
  const canControlFeishuService = feishuSetupGuidance.canStartService || health?.feishud_healthy === true;
  const wechatStatusSummary = useMemo(() => {
    if (wechatStatusLoading) {
      return t("正在读取微信当前状态。", "Loading the current WeChat status.");
    }
    if (wechatStepStatus === "done") {
      return t("设置和登录都已完成，现在可以直接通过微信发送消息。", "Setup and sign-in are complete. You can now send messages through WeChat.");
    }
    if (wechatStepStatus === "attention") {
      return t("微信已经接近可用。完成剩下的启动或扫码即可。", "WeChat is almost ready. Finish the remaining service start or QR sign-in steps.");
    }
    return t("还没有开始微信接入。按页面提示完成设置即可。", "WeChat setup has not started yet. Follow the prompts on the card to finish setup.");
  }, [lang, t, wechatStatusLoading, wechatStepStatus]);
  const wechatServiceReady = health?.wechatd_healthy === true;
  const wechatQrVisible = wechatQrPreviewRequested && Boolean(wechatLoginStatus?.qrcode_url);
  const wechatAwaitingPhoneConfirm = wechatLoginStatus?.qr_status === "scaned";
  const wechatInlineHeadline = useMemo(() => {
    if (!wechatServiceReady) {
      return t("先启动微信服务，再生成二维码。", "Start the WeChat service before generating a QR code.");
    }
    if (wechatQrStarting || wechatLoginStatus?.qr_status === "generating") {
      return t("新的二维码正在生成。", "A new QR code is being generated.");
    }
    if (wechatLoginStatus?.connected) {
      return t("微信已经连接成功，可以直接收发消息。", "WeChat is connected and ready to send or receive messages.");
    }
    if (wechatAwaitingPhoneConfirm) {
      return t("二维码已被扫描，请在手机上确认登录。", "The QR code was scanned. Please confirm the login on your phone.");
    }
    if (wechatQrVisible) {
      return t("请使用手机微信扫描左侧二维码。", "Please scan the QR code on the left with WeChat.");
    }
    return t("服务就绪后，生成二维码即可开始扫码登录。", "Once the service is ready, generate a QR code to begin sign-in.");
  }, [lang, t, wechatAwaitingPhoneConfirm, wechatLoginStatus?.connected, wechatLoginStatus?.qr_status, wechatQrStarting, wechatQrVisible, wechatServiceReady]);
  const wechatInlineHint = useMemo(() => {
    if (wechatLoginStatus?.connected) {
      return t("保持当前登录状态即可，不需要再重复扫码。", "Keep the current session as is. There is no need to scan again.");
    }
    return wechatLoginStatus?.message || t("界面会自动刷新扫码状态；如果长时间没有变化，可以手动刷新。", "The setup area refreshes scan status automatically. If nothing changes for a while, you can refresh it manually.");
  }, [lang, t, wechatLoginStatus?.connected, wechatLoginStatus?.message]);
  const telegramStatusSummary = useMemo(() => {
    if (telegramStatusLoading) {
      return t("正在读取 Telegram 当前状态。", "Loading the current Telegram status.");
    }
    if (telegramStepStatus === "done") {
      return t("Telegram 已经可用。你可以直接在 Telegram 里收发消息。", "Telegram is ready. You can send and receive messages there now.");
    }
    if (hasUnsavedTelegramConfigChanges) {
      return t("你刚改了 Telegram 设置，先保存，再启动服务。", "You changed the Telegram settings. Save them first, then start the service.");
    }
    if (telegramStepStatus === "attention") {
      return t("Bot Token 已填好，再启动一次服务就可以了。", "The bot token is ready. Start the service once more to finish setup.");
    }
    return t("填入 Bot Token 后保存，再启动服务，就可以开始使用 Telegram。", "Enter the bot token, save it, and start the service to begin using Telegram.");
  }, [hasUnsavedTelegramConfigChanges, lang, t, telegramStatusLoading, telegramStepStatus]);
  const feishuStatusSummary = useMemo(() => {
    if (feishuStatusLoading) {
      return t("正在读取飞书当前状态。", "Loading the current Feishu status.");
    }
    return lang === "zh" ? feishuSetupGuidance.zhSummary : feishuSetupGuidance.enSummary;
  }, [feishuSetupGuidance.enSummary, feishuSetupGuidance.zhSummary, feishuStatusLoading, lang, t]);
  const testMessageStepStatus = useMemo<"done" | "attention" | "todo">(() => {
    const hasAssistantReply = chatMessages.some((msg) => msg.role === "assistant");
    if (hasAssistantReply) return "done";
    if (llmStepStatus === "done") {
      return "attention";
    }
    return "todo";
  }, [chatMessages, llmStepStatus]);
  const pageMeta = useMemo(
    () => ({
      dashboard: {
        title: t("首页", "Home"),
        desc: t("先看现在能不能用、下一步该点哪里，再决定要不要进更高级的页面。", "See whether things are working, what to do next, and only then move into advanced pages."),
      },
      chat: {
        title: t("对话交互", "Chat Interaction"),
        desc: t("在这里发一条最简单的测试消息，确认模型和已接入通信方式已经真正可用。", "Send a simple test message here to confirm the model and connected communication methods really work."),
      },
      nni: {
        title: t("NNI", "NNI"),
        desc: t("查看 Network Native Intelligence 状态，处理设备公钥、时间戳签名和 TNG 证书链。", "Check Network Native Intelligence status and manage device public keys, timestamp signatures, and the TNG certificate chain."),
      },
      services: {
        title: t("通信接入", "Communication Setup"),
        desc: t("微信、Telegram 和飞书都在这里接入。按你要使用的通信方式完成配置即可。", "Connect WeChat, Telegram, and Feishu here. Configure only the communication method you plan to use."),
      },
      channels: {
        title: t("账号绑定", "Account Binding"),
        desc: t("这里用于生成访问 Key，以及处理账号绑定。", "Use this page to generate access keys and manage account bindings."),
      },
      models: {
        title: t("大模型", "Models"),
        desc: t("这是第一步。先把主模型配好，RustClaw 才能正常理解消息和执行大多数任务。", "This is step one. Configure the main LLM first so RustClaw can understand messages and run most tasks."),
      },
      skills: {
        title: t("工具与技能设置", "Tool and Skill Settings"),
        desc: t("这里管理固定开启的工具能力，以及可按需开启的技能。", "Manage always-on tool capabilities and skills that can be enabled as needed."),
      },
      memory: {
        title: t("记忆管理", "Memory"),
        desc: t("查看 RustClaw 记住了什么，按需要删除、过期或清空。", "Review what RustClaw remembers, and delete, expire, or clear items when needed."),
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
        label: t("对话测试", "Chat"),
        hint: t("试消息", "test reply"),
        icon: <MessageCircle className="h-4 w-4" />,
      },
      {
        id: "nni" as const,
        label: "NNI",
        hint: t("设备签名", "device sign"),
        icon: <Network className="h-4 w-4" />,
      },
      {
        id: "channels" as const,
        label: t("账号绑定", "Account Binding"),
        hint: t("key 和绑定", "keys and bindings"),
        icon: <Database className="h-4 w-4" />,
      },
      {
        id: "models" as const,
        label: t("大模型", "Models"),
        hint: t("先配置", "step one"),
        icon: <Sparkles className="h-4 w-4" />,
      },
      {
        id: "services" as const,
        label: t("通信接入", "Communication Setup"),
        hint: t("通微信 TG", "connect comms"),
        icon: <Server className="h-4 w-4" />,
      },
      {
        id: "skills" as const,
        label: t("工具与技能", "Tools & Skills"),
        hint: t("管理能力", "manage capabilities"),
        icon: <Wrench className="h-4 w-4" />,
      },
      {
        id: "memory" as const,
        label: t("记忆管理", "Memory"),
        hint: t("可删除", "review"),
        icon: <Brain className="h-4 w-4" />,
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
  const onboardingSteps = useMemo(
    () => [
      {
        key: "llm",
        title: t("先设置大模型", "Set up the LLM"),
        description: t("选择厂商、模型并保存。没有这一步，大多数功能都还不能正常工作。", "Choose a vendor and model, then save it. Most RustClaw features depend on this step."),
        status: llmStepStatus,
        page: "models" as const,
        cta: t("去设置模型", "Open Models"),
      },
      {
        key: "chat",
        title: t("发送测试消息", "Send a test message"),
        description: t("先发一条简单消息，确认主模型已经能够正常回复。", "Send a simple message first to confirm the main model can reply normally."),
        status: testMessageStepStatus,
        page: "chat" as const,
        cta: t("去测试消息", "Open Chat"),
      },
      {
        key: "wechat",
        title: t("连接机器人", "Connect the bot"),
        description: t("如果你准备接入微信、Telegram 或飞书，就到通信接入页继续完成配置、启动服务和登录验证。", "If you are ready to connect WeChat, Telegram, or Feishu, continue in Communication Setup to finish configuration, start the service, and complete sign-in verification."),
        status: wechatStepStatus,
        page: "services" as const,
        cta: t("去通信接入", "Open Communication Setup"),
      },
    ],
    [lang, llmStepStatus, testMessageStepStatus, wechatStepStatus],
  );
  const dashboardOverviewItems = useMemo(
    () =>
      getDashboardOverviewItems({
        isOnline,
        memoryLabel: formatBytes(health?.memory_rss_bytes ?? null),
        uptimeLabel: formatDuration(health?.uptime_seconds),
      }),
    [health?.memory_rss_bytes, health?.uptime_seconds, isOnline],
  );
  const workspaceUpdateView = useMemo(() => buildWorkspaceUpdateView(workspaceUpdateStatus, lang), [workspaceUpdateStatus, lang]);
  const workspaceUpdateRestarting = workspaceUpdateView.restarting;
  const workspaceUpdateRunning = workspaceUpdateView.running;
  const workspaceUpdateHasRemoteDiff = workspaceUpdateView.hasRemoteDiff;
  const workspaceUpdateDisplayStatus = workspaceUpdateView.displayStatus;
  const workspaceUpdateProgressPercent = workspaceUpdateView.progressPercent;
  const workspaceUpdateProgressActive = workspaceUpdateView.progressActive;
  const workspaceUpdateProgressLabel = workspaceUpdateView.progressLabel;
  const workspaceUpdateLogPreview = workspaceUpdateView.logPreview;
  const workspaceUpdateNotice = workspaceUpdateView.notice;
  const workspaceUpdateStepLabel = (step?: string) => formatWorkspaceUpdateStep(step, lang);
  const workspaceUpdateStatusLabel = (status?: string) => formatWorkspaceUpdateStatus(status, workspaceUpdateStatus?.mode, lang);
  const workspaceUpdateTimeLabel = (ts?: number | null) => formatWorkspaceUpdateTime(ts, lang);
  const factoryResetModal = factoryResetDialogOpen ? (
    <FactoryResetModal
      t={t}
      confirmWord={factoryResetConfirmWord}
      countdown={factoryResetCountdown}
      confirmText={factoryResetConfirmText}
      loading={factoryResetLoading}
      error={factoryResetError}
      result={factoryResetResult}
      canConfirm={factoryResetCanConfirm}
      onConfirmTextChange={setFactoryResetConfirmText}
      onClose={closeFactoryResetDialog}
      onRunFactoryReset={runFactoryReset}
    />
  ) : null;


  if (!uiAuthReady) {
    return (
      <SignInPage
        t={t}
        lang={lang}
        loginTab={loginTab}
        baseUrl={baseUrl}
        uiKey={uiKey}
        uiKeyDraft={uiKeyDraft}
        maskedSavedUiKey={maskedSavedUiKey}
        webdBaseUrlDraft={webdBaseUrlDraft}
        webdUsername={webdUsername}
        webdPassword={webdPassword}
        uiAuthLoading={uiAuthLoading}
        uiAuthError={uiAuthError}
        factoryResetModal={factoryResetModal}
        onBaseUrlChange={setBaseUrl}
        onUiKeyDraftChange={setUiKeyDraft}
        onWebdBaseUrlDraftChange={setWebdBaseUrlDraft}
        onWebdUsernameChange={setWebdUsername}
        onWebdPasswordChange={setWebdPassword}
        onVerifyUiKey={verifyUiKey}
        onLoginWebd={loginWebd}
        onSwitchLoginTab={(tab) => {
          setLoginTab(tab);
          setUiAuthError(null);
        }}
        onToggleLanguage={() => setLang((value) => (value === "zh" ? "en" : "zh"))}
      />
    );
  }


  return (
    <ConsoleLayout
      t={t}
      lang={lang}
      authMode={authMode}
      authIdentity={authIdentity}
      isAdminIdentity={isAdminIdentity}
      currentPage={currentPage}
      navItems={navItems}
      maskedIdentityKey={maskedIdentityKey}
      maskedSavedUiKey={maskedSavedUiKey}
      factoryResetModal={factoryResetModal}
      onCurrentPageChange={setCurrentPage}
      onToggleLanguage={() => setLang((value) => (value === "zh" ? "en" : "zh"))}
      onLogout={logout}
      onOpenFactoryReset={openFactoryResetDialog}
    >
          {currentPage === "dashboard" ? (
            <DashboardPage
              t={t}
              onboardingSteps={onboardingSteps}
              dashboardOverviewItems={dashboardOverviewItems}
              isAdminIdentity={isAdminIdentity}
              workspaceUpdateLoading={workspaceUpdateLoading}
              workspaceUpdateRunning={workspaceUpdateRunning}
              workspaceUpdateHasRemoteDiff={workspaceUpdateHasRemoteDiff}
              workspaceUpdateStatus={workspaceUpdateStatus}
              workspaceUpdateCanceling={workspaceUpdateCanceling}
              workspaceUpdateMessage={workspaceUpdateMessage}
              workspaceUpdateRestarting={workspaceUpdateRestarting}
              workspaceUpdateDisplayStatus={workspaceUpdateDisplayStatus}
              workspaceUpdateProgressPercent={workspaceUpdateProgressPercent}
              workspaceUpdateProgressActive={workspaceUpdateProgressActive}
              workspaceUpdateProgressLabel={workspaceUpdateProgressLabel}
              workspaceUpdateLogPreview={workspaceUpdateLogPreview}
              workspaceUpdateNotice={workspaceUpdateNotice}
              systemRestarting={systemRestarting}
              systemRestartMessage={systemRestartMessage}
              piAppStatus={piAppStatus}
              piAppRestarting={piAppRestarting}
              piAppRestartMessage={piAppRestartMessage}
              dashboardCommunicationRows={dashboardCommunicationRows}
              queuePressureHigh={queuePressureHigh}
              runningTooOld={runningTooOld}
              isOnline={isOnline}
              queueLength={health?.queue_length ?? 0}
              runningOldestAgeLabel={formatDuration(health?.running_oldest_age_seconds)}
              onSetCurrentPage={setCurrentPage}
              onFetchWorkspaceUpdateStatus={() => fetchWorkspaceUpdateStatus(false)}
              onStartWorkspaceUpdate={startWorkspaceUpdate}
              onCancelWorkspaceUpdate={cancelWorkspaceUpdate}
              onRestartSystem={restartSystem}
              onRestartPiApp={restartPiApp}
              workspaceUpdateStepLabel={workspaceUpdateStepLabel}
              workspaceUpdateStatusLabel={workspaceUpdateStatusLabel}
              workspaceUpdateTimeLabel={workspaceUpdateTimeLabel}
            />
          ) : null}

          {currentPage === "chat" ? (
            <ChatPage
              t={t}
              chatMessages={chatMessages}
              chatInput={chatInput}
              chatImageAttachments={chatImageAttachments}
              chatAgentMode={chatAgentMode}
              chatSending={chatSending}
              chatError={chatError}
              chatImageInputRef={chatImageInputRef}
              toLocalTime={toLocalTime}
              onChatAgentModeChange={setChatAgentMode}
              onClearMessages={() =>
                setChatMessages([
                  {
                    id: `chat-clear-${Date.now()}`,
                    role: "system",
                    text: t("聊天记录已清空。", "Chat history cleared."),
                    ts: Date.now(),
                  },
                ])
              }
              onChatInputChange={setChatInput}
              onChatInputKeyDown={handleChatInputKeyDown}
              onImageSelection={handleChatImageSelection}
              onRemoveImageAttachment={removeChatImageAttachment}
              onSendMessage={sendChatMessage}
            />
          ) : null}

          {currentPage === "nni" ? (
            <NniPage
              lang={lang}
              t={t}
              nniStatus={nniStatus}
              nniStatusLoading={nniStatusLoading}
              nniStatusError={nniStatusError}
              nniActionLoading={nniActionLoading}
              nniActionResult={nniActionResult}
              nniActionError={nniActionError}
              nniActionMessage={nniActionMessage}
              nniJoined={nniJoined}
              nniRemoteNodes={nniRemoteNodes}
              nniRemoteNodeCount={nniRemoteNodeCount}
              nniHeartbeatRequestCount={nniHeartbeatRequestCount}
              nniHeartbeatRetryLimit={nniHeartbeatRetryLimit}
              nniLastHeartbeatAtTs={nniLastHeartbeatAtTs}
              nniLastHeartbeatNetworkFailures={nniLastHeartbeatNetworkFailures}
              nniHeartbeatRecords={nniHeartbeatRecords}
              nniHeartbeatRecordsPage={nniHeartbeatRecordsPage}
              nniHeartbeatRecordsTotal={nniHeartbeatRecordsTotal}
              nniHeartbeatRecordsTotalPages={nniHeartbeatRecordsTotalPages}
              nniHeartbeatRecordsLoading={nniHeartbeatRecordsLoading}
              nniHeartbeatRecordsClearing={nniHeartbeatRecordsClearing}
              nniHeartbeatRecordsError={nniHeartbeatRecordsError}
              nniHeartbeatRecordsMessage={nniHeartbeatRecordsMessage}
              nniHeartbeatRecordsPageSize={NNI_HEARTBEAT_RECORDS_PAGE_SIZE}
              nniHeartbeatErrors={nniHeartbeatErrors}
              nniHeartbeatErrorsPage={nniHeartbeatErrorsPage}
              nniHeartbeatErrorsTotal={nniHeartbeatErrorsTotal}
              nniHeartbeatErrorsTotalPages={nniHeartbeatErrorsTotalPages}
              nniHeartbeatErrorsLoading={nniHeartbeatErrorsLoading}
              nniHeartbeatErrorsClearing={nniHeartbeatErrorsClearing}
              nniHeartbeatErrorsError={nniHeartbeatErrorsError}
              nniHeartbeatErrorsMessage={nniHeartbeatErrorsMessage}
              nniHeartbeatErrorsPageSize={NNI_HEARTBEAT_ERRORS_PAGE_SIZE}
              nniConfigLoading={nniConfigLoading}
              nniConfigSaving={nniConfigSaving}
              nniConfigError={nniConfigError}
              nniConfigMessage={nniConfigMessage}
              formatUnixDateTime={formatUnixDateTime}
              onFetchDeviceStatus={fetchNniDeviceStatus}
              onSetJoinedPersisted={setNniJoinedPersisted}
              onJoin={joinNni}
              onTestJoin={testJoinNni}
              onFetchConfig={fetchNniConfig}
              onSaveConfig={saveNniConfig}
              onRemoteNodesChange={updateNniRemoteNodes}
              onFetchHeartbeatRecords={fetchNniHeartbeatRecords}
              onClearHeartbeatRecords={clearNniHeartbeatRecords}
              onFetchHeartbeatErrors={fetchNniHeartbeatErrors}
              onClearHeartbeatErrors={clearNniHeartbeatErrors}
              onRunDeviceAction={runNniDeviceAction}
              onActionMessageChange={setNniActionMessage}
              onActionErrorChange={setNniActionError}
            />
          ) : null}

          {currentPage === "services" ? (
            <CommunicationSetupPage
              lang={lang}
              t={t}
              serviceActionMessage={serviceActionMessage}
              serviceActionLoading={serviceActionLoading}
              wechatStatusLoading={wechatStatusLoading}
              wechatStepStatus={wechatStepStatus}
              wechatStatusSummary={wechatStatusSummary}
              wechatQrStarting={wechatQrStarting}
              wechatLoginStatus={wechatLoginStatus}
              wechatQrPreviewRequested={wechatQrPreviewRequested}
              wechatLoginError={wechatLoginError}
              wechatConfigEnabled={wechatConfigData?.enabled === true}
              wechatServiceHealthy={health?.wechatd_healthy === true}
              telegramStatusLoading={telegramStatusLoading}
              telegramStepStatus={telegramStepStatus}
              telegramStatusSummary={telegramStatusSummary}
              primaryTelegramBot={primaryTelegramBot}
              telegramBotTokenConfigured={telegramBotTokenConfigured}
              telegramConfigError={telegramConfigError}
              telegramConfigSaveMessage={telegramConfigSaveMessage}
              telegramConfigSaving={telegramConfigSaving}
              telegramConfigLoading={telegramConfigLoading}
              hasUnsavedTelegramConfigChanges={hasUnsavedTelegramConfigChanges}
              telegramServiceHealthy={health?.telegramd_healthy === true}
              feishuStatusLoading={feishuStatusLoading}
              feishuStepStatus={feishuStepStatus}
              feishuStatusSummary={feishuStatusSummary}
              feishuConfigError={feishuConfigError}
              feishuSetupGuidance={feishuSetupGuidance}
              feishuCurrentKeyBound={feishuCurrentKeyBound}
              feishuBindQrDataUrl={feishuBindQrDataUrl}
              feishuBindStatusCopy={feishuBindStatusCopy}
              feishuBindSession={feishuBindSession}
              feishuBindError={feishuBindError}
              feishuBindLoading={feishuBindLoading}
              feishuResetLoading={feishuResetLoading}
              isAdminIdentity={isAdminIdentity}
              feishuServiceHealthy={health?.feishud_healthy === true}
              canControlFeishuService={canControlFeishuService}
              onControlService={controlService}
              onStartWechatQrLogin={startWechatQrLogin}
              onTelegramBotTokenChange={(value) => setTelegramPrimaryBotDraftField("bot_token", value)}
              onSaveTelegramConfig={saveTelegramConfig}
              onBeginFeishuBind={beginFeishuBind}
              onResetFeishuSetup={resetFeishuSetup}
            />
          ) : null}

          {currentPage === "channels" ? (
            <AuthKeysPage
              lang={lang}
              t={t}
              tSlash={tSlash}
              isAdminIdentity={isAdminIdentity}
              authKeysList={authKeysList}
              sortedAuthKeysList={sortedAuthKeysList}
              authKeysLoading={authKeysLoading}
              authKeysError={authKeysError}
              authKeyCreateLoading={authKeyCreateLoading}
              authKeyCreateError={authKeyCreateError}
              authKeyActionLoading={authKeyActionLoading}
              authKeyActionError={authKeyActionError}
              authKeyCopyingTarget={authKeyCopyingTarget}
              authKeyCopiedTarget={authKeyCopiedTarget}
              newlyCreatedKey={newlyCreatedKey}
              webdLoginEditorKeyId={webdLoginEditorKeyId}
              webdLoginUsernameDraft={webdLoginUsernameDraft}
              webdLoginPasswordDraft={webdLoginPasswordDraft}
              onFetchAuthKeys={fetchAuthKeys}
              onCreateAuthKey={createAuthKey}
              onPromptCreateCustomAuthKey={promptCreateCustomAuthKey}
              onCopyAuthKey={copyAuthKey}
              onDismissNewlyCreatedKey={dismissNewlyCreatedKey}
              onUpdateAuthKey={updateAuthKey}
              onPromptUpdateAuthKeyRole={promptUpdateAuthKeyRole}
              onOpenWebdLoginEditor={openWebdLoginEditor}
              onCloseWebdLoginEditor={closeWebdLoginEditor}
              onDeleteAuthKey={deleteAuthKey}
              onWebdLoginUsernameDraftChange={setWebdLoginUsernameDraft}
              onWebdLoginPasswordDraftChange={setWebdLoginPasswordDraft}
              onSaveWebdLoginEditor={saveWebdLoginEditor}
            />
          ) : null}

          {currentPage === "models" ? (
            <ModelConfigPage
              t={t}
              tSlash={tSlash}
              llmConfigData={llmConfigData}
              selectedLlmVendorInfo={selectedLlmVendorInfo}
              hasCustomLlmVendor={hasCustomLlmVendor}
              llmConfigLoading={llmConfigLoading}
              llmConfigSaving={llmConfigSaving}
              llmTestLoading={llmTestLoading}
              llmDraftVendor={llmDraftVendor}
              llmDraftModel={llmDraftModel}
              llmDraftBaseUrl={llmDraftBaseUrl}
              llmDraftApiFormat={llmDraftApiFormat}
              llmDraftApiKey={llmDraftApiKey}
              llmConfigError={llmConfigError}
              llmConfigSaveMessage={llmConfigSaveMessage}
              llmTestMessage={llmTestMessage}
              llmTestError={llmTestError}
              hasUnsavedLlmChanges={hasUnsavedLlmChanges}
              modelsAdvancedOpen={modelsAdvancedOpen}
              multimodalDraft={multimodalDraft}
              multimodalConfigLoading={multimodalConfigLoading}
              multimodalConfigSaving={multimodalConfigSaving}
              multimodalConfigError={multimodalConfigError}
              multimodalConfigSaveMessage={multimodalConfigSaveMessage}
              hasUnsavedMultimodalChanges={hasUnsavedMultimodalChanges}
              onApplyLlmVendorDraft={applyLlmVendorDraft}
              onLlmDraftModelChange={setLlmDraftModel}
              onLlmDraftBaseUrlChange={setLlmDraftBaseUrl}
              onLlmDraftApiFormatChange={setLlmDraftApiFormat}
              onLlmDraftApiKeyChange={setLlmDraftApiKey}
              onTestLlmConfig={testLlmConfig}
              onSaveLlmConfig={saveLlmConfig}
              onToggleModelsAdvanced={() => setModelsAdvancedOpen((open) => !open)}
              onFetchMultimodalConfig={fetchMultimodalConfig}
              onSaveMultimodalConfig={saveMultimodalConfig}
              onMultimodalDraftChange={setMultimodalDraftKey}
              renderMultimodalModelMeta={renderMultimodalModelMeta}
            />
          ) : null}

          {currentPage === "skills" ? (
            <SkillsPage
              lang={lang}
              t={t}
              tSlash={tSlash}
              skillImportSource={skillImportSource}
              skillImportLoading={skillImportLoading}
              skillImportError={skillImportError}
              skillImportMessage={skillImportMessage}
              systemRestartMessage={systemRestartMessage}
              skillImportPreview={skillImportPreview}
              localImportPickerOpen={localImportPickerOpen}
              folderImportInputRef={folderImportInputRef}
              fileImportInputRef={fileImportInputRef}
              onSkillImportSourceChange={setSkillImportSource}
              onImportExternalSkill={importExternalSkill}
              onLocalImportPickerOpenChange={setLocalImportPickerOpen}
              onUploadImportedSkillFiles={uploadImportedSkillFiles}
              onDismissSkillImportPreview={() => setSkillImportPreview(null)}
              skillsConfigData={skillsConfigData}
              skillsConfigLoading={skillsConfigLoading}
              skillsConfigError={skillsConfigError}
              skillSwitchSaving={skillSwitchSaving}
              skillSwitchSaveMessage={skillSwitchSaveMessage}
              hasUnsavedSkillSwitchChanges={hasUnsavedSkillSwitchChanges}
              managedSkills={managedSkills}
              filteredManagedSkills={filteredManagedSkills}
              filteredSkillsTool={filteredSkillsTool}
              filteredSkillsBase={filteredSkillsBase}
              filteredSkillsImage={filteredSkillsImage}
              filteredSkillsAudio={filteredSkillsAudio}
              filteredSkillsOther={filteredSkillsOther}
              normalizedSkillsSearchQuery={normalizedSkillsSearchQuery}
              skillsSearchQuery={skillsSearchQuery}
              skillItemsByName={skillItemsByName}
              configuredEnabledSkills={configuredEnabledSkills}
              skillSwitchDraft={skillSwitchDraft}
              recentImportedSkillName={recentImportedSkillName}
              externalSkillNamesSet={externalSkillNamesSet}
              lockedSkillNamesSet={lockedSkillNamesSet}
              toolSkillNamesSet={toolSkillNamesSet}
              baseSkillNamesSet={baseSkillNamesSet}
              skillUninstallingName={skillUninstallingName}
              onFetchSkillsConfig={fetchSkillsConfig}
              onSaveSkillSwitches={() => saveSkillSwitches(restartSystem)}
              onSkillsSearchQueryChange={setSkillsSearchQuery}
              onToggleSkillEnabled={toggleSkillEnabled}
              onUninstallExternalSkill={uninstallExternalSkill}
            />
          ) : null}

          {currentPage === "memory" ? (
            <MemoryPage
              lang={lang}
              t={t}
              memoryLoading={memoryLoading}
              memoryError={memoryError}
              memoryMessage={memoryMessage}
              memoryOverview={memoryOverview}
              memoryPreferences={memoryPreferences}
              memoryFacts={memoryFacts}
              memoryRecent={memoryRecent}
              memoryActionLoading={memoryActionLoading}
              memorySettingsSaving={memorySettingsSaving}
              memoryClearScope={memoryClearScope}
              onMemoryClearScopeChange={setMemoryClearScope}
              onFetchMemoryData={fetchMemoryData}
              onDeleteMemoryItem={deleteMemoryItem}
              onExpireMemoryItem={expireMemoryItem}
              onClearMemoryScope={clearMemoryScope}
              onUpdateMemoryLongTermEnabled={updateMemoryLongTermEnabled}
            />
          ) : null}

          {currentPage === "logs" ? (
            <LogsPage
              t={t}
              tSlash={tSlash}
              selectedLogFile={selectedLogFile}
              logTailLines={logTailLines}
              logFollowTail={logFollowTail}
              logLastUpdated={logLastUpdated}
              logLoading={logLoading}
              logError={logError}
              logText={logText}
              logContainerRef={logContainerRef}
              toLocalTime={toLocalTime}
              onSelectedLogFileChange={setSelectedLogFile}
              onLogTailLinesChange={setLogTailLines}
              onLogFollowTailChange={setLogFollowTail}
              onFetchLatestLog={fetchLatestLog}
            />
          ) : null}

          {currentPage === "tasks" ? (
            <TasksPage
              lang={lang}
              t={t}
              tSlash={tSlash}
              activeTasks={activeTasks}
              activeTasksLoading={activeTasksLoading}
              activeTasksError={activeTasksError}
              activeTasksLastUpdated={activeTasksLastUpdated}
              resumeTaskError={resumeTaskError}
              resumeTaskMessage={resumeTaskMessage}
              cancelTaskError={cancelTaskError}
              cancelTaskMessage={cancelTaskMessage}
              cancelingTaskIndex={cancelingTaskIndex}
              canUseInteractionContext={interactionUserId != null && interactionChatId != null}
              resumeDrafts={resumeDrafts}
              resumeSubmittingTaskId={resumeSubmittingTaskId}
              toLocalTime={toLocalTime}
              onFetchActiveTasks={fetchActiveTasks}
              onViewTask={(taskIdToView) => {
                setTaskId(taskIdToView);
                return queryTaskById(taskIdToView);
              }}
              onCancelTask={cancelActiveTask}
              onResumeDraftChange={(taskIdToResume, value) =>
                setResumeDrafts((prev) => ({
                  ...prev,
                  [taskIdToResume]: value,
                }))
              }
              onSubmitResume={submitResumeForTask}
              interactionKind={interactionKind}
              interactionChannel={interactionChannel}
              interactionAdapter={interactionAdapter}
              interactionExternalUserId={interactionExternalUserId}
              interactionExternalChatId={interactionExternalChatId}
              interactionRole={interactionRole}
              localContextLoading={localContextLoading}
              localContextError={localContextError}
              interactionAskText={interactionAskText}
              interactionAgentMode={interactionAgentMode}
              interactionSkillName={interactionSkillName}
              interactionSkillArgs={interactionSkillArgs}
              interactionLoading={interactionLoading}
              interactionSubmittedTaskId={interactionSubmittedTaskId}
              trackingTaskId={trackingTaskId}
              interactionError={interactionError}
              onInteractionKindChange={setInteractionKind}
              onInteractionChannelChange={setInteractionChannel}
              onInteractionAdapterChange={setInteractionAdapter}
              onInteractionExternalUserIdChange={setInteractionExternalUserId}
              onInteractionExternalChatIdChange={setInteractionExternalChatId}
              onInteractionAskTextChange={setInteractionAskText}
              onInteractionAgentModeChange={setInteractionAgentMode}
              onInteractionSkillNameChange={setInteractionSkillName}
              onInteractionSkillArgsChange={setInteractionSkillArgs}
              onSubmitInteractionTask={submitInteractionTask}
              taskId={taskId}
              taskLoading={taskLoading}
              taskError={taskError}
              taskResult={taskResult}
              onTaskIdChange={setTaskId}
              onQueryTask={queryTask}
            />
          ) : null}
    </ConsoleLayout>
  );
}
