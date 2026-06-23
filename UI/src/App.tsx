import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertCircle,
  BellRing,
  Brain,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Copy,
  Cpu,
  Database,
  Download,
  FileText,
  Fingerprint,
  KeyRound,
  LayoutDashboard,
  Loader2,
  MessageCircle,
  Network,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  SquareTerminal,
  Server,
  Timer,
  Trash2,
  Wrench,
  X,
} from "lucide-react";
import QRCode from "qrcode";
import { ActiveTasksPanel } from "./components/ActiveTasksPanel";
import { ChatPage } from "./components/ChatPage";
import { LogsPage } from "./components/LogsPage";
import { ManualTaskSubmitPanel } from "./components/ManualTaskSubmitPanel";
import { MemoryPage } from "./components/MemoryPage";
import { ModelConfigPage } from "./components/ModelConfigPage";
import { SkillImportPanel } from "./components/SkillImportPanel";
import { TaskResultPanel } from "./components/TaskResultPanel";
import {
  countCompletedDashboardSteps,
  getDashboardOverviewItems,
} from "./lib/dashboard-home";
import { copyAuthKeyValue, maskStoredKey, writeTextToClipboard } from "./lib/auth-keys";
import { fileToDataUrl, formatVisionResultText } from "./lib/chat-attachments";
import { boundChannelsLabel as formatBoundChannelsLabel, channelLabel as formatChannelLabel } from "./lib/channel-display";
import { formatBytes, formatDuration, sleep, toLocalTime } from "./lib/display-format";
import {
  formatDateOnlyHuman,
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
import { hasUnsavedLlmDraftChanges, llmVendorSupportsApiFormat } from "./lib/llm-config";
import {
  MULTIMODAL_KEYS,
  buildMultimodalDraft,
  buildMultimodalMetaView,
  buildMultimodalSavePayload,
  hasUnsavedMultimodalDraftChanges,
  updateMultimodalDraftField,
  type MultimodalKey,
} from "./lib/model-config";
import {
  NNI_RUNTIME_TILES,
  nniActionLabel as formatNniActionLabel,
  nniJoinErrorMessage,
  nniPayloadHexField,
  parseNniRemoteNodeUrls,
  shortenHex,
  shortNniValue,
} from "./lib/nni-display";
import {
  formatServiceActionError,
  serviceActionErrorCode,
  serviceActionSuccessMessage,
  serviceDisplayName,
} from "./lib/service-actions";
import {
  baseSkillNamesWithFallback,
  filterSkillNamesBySearch,
  groupSkillNames,
  isUiHiddenSkill,
  isVisibleSkillName,
  normalizeSkillSearchQuery,
  skillCapabilityLabel,
  skillDescription,
  skillPlannerCapabilityLabel,
  skillRiskLabel,
  skillRuntimeIssue,
  visibleSkillNames,
} from "./lib/skill-display";
import { extractTaskText } from "./lib/task-result";
import {
  buildWorkspaceUpdateView,
  formatWorkspaceUpdateStatus,
  formatWorkspaceUpdateStep,
  formatWorkspaceUpdateTime,
} from "./lib/workspace-update";

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
  AuthKeyListItem,
  ResolveChannelBindingResponse,
  SkillListItem,
  SkillsResponse,
  SkillsConfigResponse,
  MemoryCounts,
  MemoryOverviewResponse,
  MemoryPreferenceItem,
  MemoryFactItem,
  MemoryRecentItem,
  MemoryDeleteResult,
  MemoryExpireResult,
  MemoryClearResult,
  MemorySettingsResult,
  FactoryResetResponse,
  ImportedSkillResponse,
  LlmVendorOption,
  LlmRuntimeInfo,
  LlmConfigResponse,
  LlmTestResponse,
  NniDeviceMeta,
  NniDeviceStatusResponse,
  NniDeviceActionResponse,
  NniJoinTaskResponse,
  NniJoinVerifyResponse,
  NniConfigResponse,
  NniHeartbeatRecord,
  NniHeartbeatRecordsResponse,
  NniHeartbeatErrorRecord,
  NniHeartbeatErrorsResponse,
  WechatConfigResponse,
  FeishuConfigResponse,
  AgentConfigItem,
  TelegramBotConfigItem,
  TelegramConfigResponse,
  ModelConfigItem,
  ModelConfigResponse,
  LogLatestResponse,
  WhatsappWebLoginStatus,
  WechatLoginStatus,
  WechatQrStartResponse,
  WechatQrWaitResponse,
  ChatMessage,
  BrowserFileWithPath,
  ChatImageAttachment,
  AdapterHealthRow,
  ChannelPreset,
  ServiceStatusRow,
  DashboardCommunicationRow,
  ServiceActionNotice,
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

function buildDefaultTelegramBot(): TelegramBotConfigItem {
  return {
    name: "primary",
    bot_token: "",
    bot_token_configured: false,
    bot_token_masked: null,
    agent_id: "main",
    allowlist: [],
    access_mode: "public",
    allowed_telegram_usernames: [],
    is_primary: true,
  };
}

const NNI_HEARTBEAT_RECORDS_PAGE_SIZE = 10;
const NNI_HEARTBEAT_ERRORS_PAGE_SIZE = 10;
const FACTORY_RESET_CONFIRM_WORD = "RESET";
const FACTORY_RESET_COUNTDOWN_SECONDS = 10;

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
  const [skillsLoading, setSkillsLoading] = useState(false);
  const [skillsError, setSkillsError] = useState<string | null>(null);
  const [skillsData, setSkillsData] = useState<SkillsResponse | null>(null);
  const [skillsConfigLoading, setSkillsConfigLoading] = useState(false);
  const [skillsConfigError, setSkillsConfigError] = useState<string | null>(null);
  const [skillsConfigData, setSkillsConfigData] = useState<SkillsConfigResponse | null>(null);
  const [skillSwitchDraft, setSkillSwitchDraft] = useState<Record<string, boolean>>({});
  const [wechatConfigLoading, setWechatConfigLoading] = useState(false);
  const [wechatConfigError, setWechatConfigError] = useState<string | null>(null);
  const [wechatConfigData, setWechatConfigData] = useState<WechatConfigResponse | null>(null);
  const [wechatConfigDraft, setWechatConfigDraft] = useState<WechatConfigResponse | null>(null);
  const [wechatConfigSaving, setWechatConfigSaving] = useState(false);
  const [wechatConfigSaveMessage, setWechatConfigSaveMessage] = useState<string | null>(null);
  const [feishuConfigLoading, setFeishuConfigLoading] = useState(false);
  const [feishuConfigError, setFeishuConfigError] = useState<string | null>(null);
  const [feishuConfigData, setFeishuConfigData] = useState<FeishuConfigResponse | null>(null);
  const [telegramConfigLoading, setTelegramConfigLoading] = useState(false);
  const [telegramConfigError, setTelegramConfigError] = useState<string | null>(null);
  const [telegramConfigData, setTelegramConfigData] = useState<TelegramConfigResponse | null>(null);
  const [telegramConfigDraft, setTelegramConfigDraft] = useState<TelegramConfigResponse | null>(null);
  const [telegramConfigSaving, setTelegramConfigSaving] = useState(false);
  const [telegramConfigSaveMessage, setTelegramConfigSaveMessage] = useState<string | null>(null);
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
  const workspaceUpdateSilentFailuresRef = useRef(0);
  const [llmConfigLoading, setLlmConfigLoading] = useState(false);
  const [llmConfigError, setLlmConfigError] = useState<string | null>(null);
  const [llmConfigData, setLlmConfigData] = useState<LlmConfigResponse | null>(null);
  const [llmDraftVendor, setLlmDraftVendor] = useState("");
  const [llmDraftModel, setLlmDraftModel] = useState("");
  const [llmConfigSaving, setLlmConfigSaving] = useState(false);
  const [llmConfigSaveMessage, setLlmConfigSaveMessage] = useState<string | null>(null);
  const [llmDraftBaseUrl, setLlmDraftBaseUrl] = useState("");
  const [llmDraftApiKey, setLlmDraftApiKey] = useState("");
  const [llmDraftApiFormat, setLlmDraftApiFormat] = useState("openai_compat");
  const [llmTestLoading, setLlmTestLoading] = useState(false);
  const [llmTestMessage, setLlmTestMessage] = useState<string | null>(null);
  const [llmTestError, setLlmTestError] = useState<string | null>(null);
  const [nniStatus, setNniStatus] = useState<NniDeviceStatusResponse | null>(null);
  const [nniStatusLoading, setNniStatusLoading] = useState(false);
  const [nniStatusError, setNniStatusError] = useState<string | null>(null);
  const [nniActionLoading, setNniActionLoading] = useState<string | null>(null);
  const [nniActionResult, setNniActionResult] = useState<NniDeviceActionResponse | null>(null);
  const [nniActionError, setNniActionError] = useState<string | null>(null);
  const [nniActionMessage, setNniActionMessage] = useState<string | null>(null);
  const [nniJoined, setNniJoined] = useState(false);
  const [nniRemoteNodes, setNniRemoteNodes] = useState("");
  const [nniHeartbeatRequestCount, setNniHeartbeatRequestCount] = useState(0);
  const [nniHeartbeatRetryLimit, setNniHeartbeatRetryLimit] = useState(3);
  const [nniLastHeartbeatAtTs, setNniLastHeartbeatAtTs] = useState<number | null>(null);
  const [nniLastHeartbeatNetworkFailures, setNniLastHeartbeatNetworkFailures] = useState(0);
  const [nniHeartbeatRecords, setNniHeartbeatRecords] = useState<NniHeartbeatRecord[]>([]);
  const [nniHeartbeatRecordsPage, setNniHeartbeatRecordsPage] = useState(1);
  const [nniHeartbeatRecordsTotal, setNniHeartbeatRecordsTotal] = useState(0);
  const [nniHeartbeatRecordsTotalPages, setNniHeartbeatRecordsTotalPages] = useState(1);
  const [nniHeartbeatRecordsLoading, setNniHeartbeatRecordsLoading] = useState(false);
  const [nniHeartbeatRecordsClearing, setNniHeartbeatRecordsClearing] = useState(false);
  const [nniHeartbeatRecordsError, setNniHeartbeatRecordsError] = useState<string | null>(null);
  const [nniHeartbeatRecordsMessage, setNniHeartbeatRecordsMessage] = useState<string | null>(null);
  const [nniHeartbeatErrors, setNniHeartbeatErrors] = useState<NniHeartbeatErrorRecord[]>([]);
  const [nniHeartbeatErrorsPage, setNniHeartbeatErrorsPage] = useState(1);
  const [nniHeartbeatErrorsTotal, setNniHeartbeatErrorsTotal] = useState(0);
  const [nniHeartbeatErrorsTotalPages, setNniHeartbeatErrorsTotalPages] = useState(1);
  const [nniHeartbeatErrorsLoading, setNniHeartbeatErrorsLoading] = useState(false);
  const [nniHeartbeatErrorsClearing, setNniHeartbeatErrorsClearing] = useState(false);
  const [nniHeartbeatErrorsError, setNniHeartbeatErrorsError] = useState<string | null>(null);
  const [nniHeartbeatErrorsMessage, setNniHeartbeatErrorsMessage] = useState<string | null>(null);
  const [nniConfigLoading, setNniConfigLoading] = useState(false);
  const [nniConfigSaving, setNniConfigSaving] = useState(false);
  const [nniConfigError, setNniConfigError] = useState<string | null>(null);
  const [nniConfigMessage, setNniConfigMessage] = useState<string | null>(null);
  const [multimodalConfigData, setMultimodalConfigData] = useState<ModelConfigResponse | null>(null);
  const [multimodalConfigLoading, setMultimodalConfigLoading] = useState(false);
  const [multimodalConfigError, setMultimodalConfigError] = useState<string | null>(null);
  const [multimodalDraft, setMultimodalDraft] = useState<Record<string, ModelConfigItem>>({});
  const [multimodalConfigSaving, setMultimodalConfigSaving] = useState(false);
  const [multimodalConfigSaveMessage, setMultimodalConfigSaveMessage] = useState<string | null>(null);
  const [modelsAdvancedOpen, setModelsAdvancedOpen] = useState(false);
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
  const [serviceActionLoading, setServiceActionLoading] = useState<Record<string, boolean>>({});
  const [serviceActionMessage, setServiceActionMessage] = useState<ServiceActionNotice | null>(null);
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
  const [authKeyCopyingTarget, setAuthKeyCopyingTarget] = useState<number | "new" | null>(null);
  const [authKeyCopiedTarget, setAuthKeyCopiedTarget] = useState<number | "new" | null>(null);
  const [authKeyActionError, setAuthKeyActionError] = useState<string | null>(null);
  const [newlyCreatedKey, setNewlyCreatedKey] = useState<string | null>(null);
  const [webdLoginEditorKeyId, setWebdLoginEditorKeyId] = useState<number | null>(null);
  const [webdLoginUsernameDraft, setWebdLoginUsernameDraft] = useState("");
  const [webdLoginPasswordDraft, setWebdLoginPasswordDraft] = useState("");
  const [factoryResetDialogOpen, setFactoryResetDialogOpen] = useState(false);
  const [factoryResetCountdown, setFactoryResetCountdown] = useState(FACTORY_RESET_COUNTDOWN_SECONDS);
  const [factoryResetConfirmText, setFactoryResetConfirmText] = useState("");
  const [factoryResetLoading, setFactoryResetLoading] = useState(false);
  const [factoryResetError, setFactoryResetError] = useState<string | null>(null);
  const [factoryResetResult, setFactoryResetResult] = useState<FactoryResetResponse | null>(null);
  const [diagnosticsRefreshing, setDiagnosticsRefreshing] = useState(false);
  const [selectedLogFile, setSelectedLogFile] = useState("clawd.log");
  const [logTailLines, setLogTailLines] = useState(200);
  const [logLoading, setLogLoading] = useState(false);
  const [logError, setLogError] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  const [logLastUpdated, setLogLastUpdated] = useState<number | null>(null);
  const [logFollowTail, setLogFollowTail] = useState(true);
  const [memoryOverview, setMemoryOverview] = useState<MemoryOverviewResponse | null>(null);
  const [memoryPreferences, setMemoryPreferences] = useState<MemoryPreferenceItem[]>([]);
  const [memoryFacts, setMemoryFacts] = useState<MemoryFactItem[]>([]);
  const [memoryRecent, setMemoryRecent] = useState<MemoryRecentItem[]>([]);
  const [memoryLoading, setMemoryLoading] = useState(false);
  const [memoryError, setMemoryError] = useState<string | null>(null);
  const [memoryMessage, setMemoryMessage] = useState<string | null>(null);
  const [memoryActionLoading, setMemoryActionLoading] = useState<string | null>(null);
  const [memorySettingsSaving, setMemorySettingsSaving] = useState(false);
  const [memoryClearScope, setMemoryClearScope] = useState<"recent" | "preferences" | "facts" | "all">("recent");
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

  useEffect(() => {
    if (!factoryResetDialogOpen || factoryResetResult || factoryResetCountdown <= 0) return;
    const timer = window.setTimeout(() => {
      setFactoryResetCountdown((value) => Math.max(0, value - 1));
    }, 1000);
    return () => window.clearTimeout(timer);
  }, [factoryResetCountdown, factoryResetDialogOpen, factoryResetResult]);

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
  const activeUserKey = authMode === "key" && uiKey.trim() ? uiKey.trim() : "";
  const activeIdentityIds =
    activeUserKey || interactionUserId == null || interactionChatId == null
      ? {}
      : { user_id: interactionUserId, chat_id: interactionChatId };

  const readApiBody = async <T,>(res: Response, label: string): Promise<T> => {
    const body = (await res.json()) as ApiResponse<T>;
    if (!res.ok || !body.ok || body.data === undefined) {
      throw new Error(body.error || `${label} 请求失败 (${res.status})`);
    }
    return body.data;
  };

  const fetchMemoryData = async (silent = false) => {
    if (!silent) {
      setMemoryLoading(true);
      setMemoryError(null);
    }
    try {
      const [overviewRes, preferencesRes, factsRes, recentRes] = await Promise.all([
        apiFetch("/v1/memory"),
        apiFetch("/v1/memory/preferences"),
        apiFetch("/v1/memory/facts"),
        apiFetch("/v1/memory/recent"),
      ]);
      const [overview, preferences, facts, recent] = await Promise.all([
        readApiBody<MemoryOverviewResponse>(overviewRes, "memory overview"),
        readApiBody<MemoryPreferenceItem[]>(preferencesRes, "memory preferences"),
        readApiBody<MemoryFactItem[]>(factsRes, "memory facts"),
        readApiBody<MemoryRecentItem[]>(recentRes, "memory recent"),
      ]);
      setMemoryOverview(overview);
      setMemoryPreferences(preferences);
      setMemoryFacts(facts);
      setMemoryRecent(recent);
      setMemoryError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setMemoryError(message);
    } finally {
      if (!silent) {
        setMemoryLoading(false);
      }
    }
  };

  const deleteMemoryItem = async (id: string) => {
    const confirmed = window.confirm(
      t("确定删除这条记忆吗？删除后不会再用于后续回复。", "Delete this memory item? It will no longer be used in future replies."),
    );
    if (!confirmed) return;
    setMemoryActionLoading(`delete:${id}`);
    setMemoryError(null);
    setMemoryMessage(null);
    try {
      const res = await apiFetch(`/v1/memory/${encodeURIComponent(id)}`, { method: "DELETE" });
      const data = await readApiBody<MemoryDeleteResult>(res, "delete memory");
      setMemoryMessage(
        data.deleted
          ? t("已删除这条记忆。", "Memory item deleted.")
          : t("没有找到这条记忆，可能已经被删除。", "Memory item was not found. It may already be deleted."),
      );
      await fetchMemoryData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setMemoryError(message);
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const expireMemoryItem = async (id: string) => {
    const confirmed = window.confirm(
      t("确定把这条记忆标记为过期吗？过期后不会再主动用于回复。", "Mark this memory item as expired? Expired items will not be actively used in replies."),
    );
    if (!confirmed) return;
    setMemoryActionLoading(`expire:${id}`);
    setMemoryError(null);
    setMemoryMessage(null);
    try {
      const res = await apiFetch(`/v1/memory/${encodeURIComponent(id)}/expire`, { method: "POST" });
      const data = await readApiBody<MemoryExpireResult>(res, "expire memory");
      setMemoryMessage(
        data.expired
          ? t("已把这条记忆标记为过期。", "Memory item marked as expired.")
          : t("没有找到这条记忆，可能已经处理过。", "Memory item was not found. It may already have been handled."),
      );
      await fetchMemoryData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setMemoryError(message);
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const clearMemoryScope = async () => {
    const labelMap: Record<typeof memoryClearScope, string> = {
      recent: t("近期记录", "recent memories"),
      preferences: t("偏好", "preferences"),
      facts: t("事实卡片", "fact cards"),
      all: t("全部记忆", "all memory data"),
    };
    const confirmed = window.confirm(
      t(
        `确定清空${labelMap[memoryClearScope]}吗？这个操作会影响后续回复使用的记忆。`,
        `Clear ${labelMap[memoryClearScope]}? This affects which memories are used in future replies.`,
      ),
    );
    if (!confirmed) return;
    setMemoryActionLoading(`clear:${memoryClearScope}`);
    setMemoryError(null);
    setMemoryMessage(null);
    try {
      const res = await apiFetch("/v1/memory/clear", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ scope: memoryClearScope }),
      });
      const data = await readApiBody<MemoryClearResult>(res, "clear memory");
      setMemoryMessage(
        t(
          `清理完成：近期 ${data.recent_deleted} 条，偏好 ${data.preferences_deleted} 条，事实 ${data.facts_deleted} 条。`,
          `Cleared: ${data.recent_deleted} recent, ${data.preferences_deleted} preferences, ${data.facts_deleted} facts.`,
        ),
      );
      await fetchMemoryData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setMemoryError(message);
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const updateMemoryLongTermEnabled = async (enabled: boolean) => {
    setMemorySettingsSaving(true);
    setMemoryError(null);
    setMemoryMessage(null);
    try {
      const res = await apiFetch("/v1/memory/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ long_term_enabled: enabled }),
      });
      const data = await readApiBody<MemorySettingsResult>(res, "memory settings");
      setMemoryOverview((prev) => (prev ? { ...prev, long_term_enabled: data.long_term_enabled } : prev));
      setMemoryMessage(
        data.restart_required
          ? t("记忆设置已保存。重启 RustClaw 后生效。", "Memory setting saved. Restart RustClaw for it to take effect.")
          : t("记忆设置没有变化。", "Memory setting is unchanged."),
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setMemoryError(message);
    } finally {
      setMemorySettingsSaving(false);
    }
  };

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

  const controlService = async (
    serviceName: "telegramd" | "whatsappd" | "whatsapp_webd" | "wechatd" | "feishud" | "larkd",
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
        setServiceActionMessage({
          tone: "error",
          text: formatServiceActionError(serviceName, action, serviceActionErrorCode(body), t),
        });
        return;
      }
      setServiceActionMessage(
        {
          tone: "success",
          text: serviceActionSuccessMessage(serviceName, action, t),
        },
      );
      await sleep(800);
      await fetchHealth();
    } catch {
      setServiceActionMessage({
        tone: "error",
        text: formatServiceActionError(serviceName, action, "service_action_request_failed", t),
      });
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
        ...(activeUserKey ? { user_key: activeUserKey } : {}),
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

  const createAuthKey = async (role = "user") => {
    setAuthKeyCreateLoading(true);
    setAuthKeyCreateError(null);
    setNewlyCreatedKey(null);
    setAuthKeyCopiedTarget(null);
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

  const fetchFullAuthKey = async (keyId: number) => {
    const res = await apiFetch(`/v1/admin/auth-keys/${keyId}/full`);
    const body = (await res.json()) as ApiResponse<{ user_key: string }>;
    if (!res.ok || !body.ok || !body.data?.user_key) {
      throw new Error(body.error || `完整 Key 获取失败 (${res.status})`);
    }
    return body.data.user_key;
  };

  const copyAuthKey = async (options: { target: number | "new"; keyId?: number; plaintextKey?: string | null }) => {
    setAuthKeyActionError(null);
    setAuthKeyCopyingTarget(options.target);
    try {
      await copyAuthKeyValue({
        keyId: options.keyId,
        plaintextKey: options.plaintextKey,
        fetchFullAuthKey,
        writeClipboard: async (value) => {
          await writeTextToClipboard(value);
        },
      });
      setAuthKeyCopiedTarget(options.target);
    } catch (err) {
      setAuthKeyActionError(err instanceof Error ? err.message : "未知错误");
    } finally {
      setAuthKeyCopyingTarget(null);
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
    setFeishuConfigError(null);
    setFeishuBindError(null);
    try {
      const res = await apiFetch(`/v1/admin/feishu/reset`, { method: "POST" });
      const body = (await res.json()) as ApiResponse<FeishuConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `飞书重置失败 (${res.status})`);
      }
      setFeishuConfigData(body.data);
      setFeishuBindSession(null);
      setFeishuBindQrDataUrl(null);
      await fetchHealth();
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setFeishuBindError(message);
    } finally {
      setFeishuResetLoading(false);
    }
  };

  const updateAuthKey = async (keyId: number, patch: { role?: string; enabled?: boolean }) => {
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

  const openWebdLoginEditor = (row: AuthKeyListItem) => {
    setAuthKeyActionError(null);
    setWebdLoginEditorKeyId(row.key_id);
    setWebdLoginUsernameDraft(row.webd_username ?? "");
    setWebdLoginPasswordDraft("");
  };

  const closeWebdLoginEditor = () => {
    setWebdLoginEditorKeyId(null);
    setWebdLoginUsernameDraft("");
    setWebdLoginPasswordDraft("");
  };

  const saveWebdLoginEditor = async (row: AuthKeyListItem) => {
    const normalizedUsername = webdLoginUsernameDraft.trim();
    const normalizedPassword = webdLoginPasswordDraft.trim();
    if (!normalizedUsername) {
      setAuthKeyActionError(t("用户名不能为空", "Username is required"));
      return;
    }
    if (!normalizedPassword) {
      setAuthKeyActionError(t("密码不能为空", "Password is required"));
      return;
    }

    setAuthKeyActionLoading(row.key_id);
    setAuthKeyActionError(null);
    try {
      const res = await apiFetch("/v1/admin/webd-accounts", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          username: normalizedUsername,
          password: normalizedPassword,
          key_id: row.key_id,
        }),
      });
      const body = (await res.json()) as ApiResponse<{ updated: boolean }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `保存登录名/密码失败 (${res.status})`);
      }
      await fetchAuthKeys();
      closeWebdLoginEditor();
    } catch (err) {
      setAuthKeyActionError(err instanceof Error ? err.message : "未知错误");
    } finally {
      setAuthKeyActionLoading(null);
    }
  };

  const deleteAuthKey = async (row: AuthKeyListItem) => {
    const ok = window.confirm(
      t(
        `确认删除 ${row.user_key}？删除后将移除该 Key、关联绑定，以及它对应的用户名密码登录。`,
        `Delete ${row.user_key}? This will remove the key, related bindings, and its username/password login.`,
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

  const openFactoryResetDialog = () => {
    setFactoryResetDialogOpen(true);
    setFactoryResetCountdown(FACTORY_RESET_COUNTDOWN_SECONDS);
    setFactoryResetConfirmText("");
    setFactoryResetError(null);
    setFactoryResetResult(null);
  };

  const clearLocalAuthAfterFactoryReset = (result: FactoryResetResponse) => {
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
    setAuthKeysList([]);
  };

  const closeFactoryResetDialog = () => {
    if (factoryResetLoading) return;
    setFactoryResetDialogOpen(false);
    setFactoryResetConfirmText("");
    setFactoryResetError(null);
    setFactoryResetCountdown(FACTORY_RESET_COUNTDOWN_SECONDS);
  };

  const runFactoryReset = async () => {
    if (factoryResetCountdown > 0 || factoryResetConfirmText.trim() !== FACTORY_RESET_CONFIRM_WORD) {
      setFactoryResetError(
        t(
          `请等待倒计时结束，并输入 ${FACTORY_RESET_CONFIRM_WORD} 后再继续。`,
          `Wait for the countdown to finish and type ${FACTORY_RESET_CONFIRM_WORD} before continuing.`,
        ),
      );
      return;
    }
    setFactoryResetLoading(true);
    setFactoryResetError(null);
    try {
      const res = await apiFetch("/v1/admin/factory-reset", { method: "POST" });
      const data = await readApiBody<FactoryResetResponse>(res, "factory reset");
      setFactoryResetResult(data);
      clearLocalAuthAfterFactoryReset(data);
    } catch (err) {
      setFactoryResetError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
    } finally {
      setFactoryResetLoading(false);
    }
  };

  const promptCreateCustomAuthKey = async () => {
    const role = window.prompt(
      t("请输入自定义角色名称，例如 operator / reviewer / finance", "Enter a custom role, such as operator / reviewer / finance"),
      "",
    );
    const normalized = role?.trim();
    if (!normalized) return;
    await createAuthKey(normalized);
  };
  const promptUpdateAuthKeyRole = async (row: AuthKeyListItem) => {
    const role = window.prompt(
      t("请输入新的角色名称。内置推荐：admin / user / guest，也支持自定义。", "Enter a new role. Suggested built-ins: admin / user / guest, but custom values are also allowed."),
      row.role,
    );
    const normalized = role?.trim();
    if (!normalized || normalized === row.role) return;
    await updateAuthKey(row.key_id, { role: normalized });
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
      const nextSwitchDraft = { ...(body.data.skill_switches || {}) };
      (body.data.locked_skill_names || body.data.core_skill_names || []).forEach((name) => {
        if (nextSwitchDraft[name] === false) nextSwitchDraft[name] = true;
      });
      setSkillSwitchDraft(nextSwitchDraft);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setSkillsConfigError(message);
    } finally {
      setSkillsConfigLoading(false);
    }
  };

  const fetchWechatConfig = async () => {
    setWechatConfigLoading(true);
    setWechatConfigError(null);
    try {
      const res = await apiFetch(`/v1/wechat/config`);
      const body = (await res.json()) as ApiResponse<WechatConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `微信配置获取失败 (${res.status})`);
      }
      setWechatConfigData(body.data);
      setWechatConfigDraft(body.data);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setWechatConfigError(message);
    } finally {
      setWechatConfigLoading(false);
    }
  };

  const fetchFeishuConfig = async () => {
    setFeishuConfigLoading(true);
    setFeishuConfigError(null);
    try {
      const res = await apiFetch(`/v1/feishu/config`);
      const body = (await res.json()) as ApiResponse<FeishuConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `飞书配置获取失败 (${res.status})`);
      }
      setFeishuConfigData(body.data);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setFeishuConfigError(message);
    } finally {
      setFeishuConfigLoading(false);
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
      setTelegramConfigDraft(body.data);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setTelegramConfigError(message);
    } finally {
      setTelegramConfigLoading(false);
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
      setLlmDraftApiKey(selectedVendor?.api_key || "");
      setLlmDraftApiFormat(llmVendorSupportsApiFormat(selectedVendor?.name) ? (selectedVendor?.api_format || "openai_compat") : "");
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setLlmConfigError(message);
    } finally {
      setLlmConfigLoading(false);
    }
  };

  const fetchNniDeviceStatus = async (silent = false) => {
    if (!silent) {
      setNniStatusLoading(true);
      setNniStatusError(null);
    }
    try {
      const res = await apiFetch(`/v1/nni/device/status`);
      const body = (await res.json()) as ApiResponse<NniDeviceStatusResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `NNI 状态获取失败 (${res.status})`);
      }
      setNniStatus(body.data);
      setNniStatusError(null);
      if (!body.data.signature_chip_present) {
        await setNniJoinedPersisted(false);
      }
      return body.data;
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniStatusError(message);
      return null;
    } finally {
      if (!silent) {
        setNniStatusLoading(false);
      }
    }
  };

  const runNniDeviceAction = async (action: string, options?: { challenge?: string }) => {
    setNniActionLoading(action);
    setNniActionError(null);
    setNniActionMessage(null);
    try {
      const res = await apiFetch(`/v1/nni/device/action`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action, challenge: options?.challenge }),
      });
      const body = (await res.json()) as ApiResponse<NniDeviceActionResponse>;
      if (!res.ok || !body.ok || !body.data) {
        const actionData = body.data;
        if (actionData?.signature_chip_present === false) {
          setNniStatus((prev) =>
            prev
              ? {
                  ...prev,
                  signature_chip_present: false,
                  status: "signature_chip_missing",
                  message: t(
                    "未检测到设备签名芯片。此设备仍可使用 RustClaw，NNI 的设备签名能力暂不可用。",
                    "No device signature chip was detected. RustClaw can still run, but NNI device signing is unavailable.",
                  ),
                }
              : prev,
          );
        }
        throw new Error(body.error || `NNI 操作失败 (${res.status})`);
      }
      setNniActionResult(body.data);
      setNniActionMessage(body.data.message || t("NNI 操作已完成。", "NNI action completed."));
      if (body.data.payload?.pubkey) {
        setNniStatus((prev) =>
          prev
            ? {
                ...prev,
                signature_chip_present: true,
                status: "ready",
                pubkey: body.data.payload?.pubkey,
                pubkey_preview: shortenHex(body.data.payload?.pubkey, 12, 12),
              }
            : prev,
        );
      }
      return body.data;
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniActionError(message);
      await setNniJoinedPersisted(false);
      return null;
    } finally {
      setNniActionLoading(null);
    }
  };

  const requestNniJoinTask = async (): Promise<NniJoinTaskResponse | null> => {
    const nodeUrls = nniRemoteNodeUrls();
    if (nodeUrls.length === 0) {
      throw new Error(t("请先填写至少一个远程 NNI 节点地址。", "Enter at least one remote NNI node URL first."));
    }
    const res = await apiFetch(`/v1/nni/join/request`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ node_urls: nodeUrls }),
    });
    const body = (await res.json()) as ApiResponse<NniJoinTaskResponse>;
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(nniJoinErrorMessage(body.error, body.data, `NNI join request failed (${res.status})`, lang));
    }
    return body.data;
  };

  const verifyNniJoinTask = async (taskId: string, nodeUrl: string, signature: string): Promise<NniJoinVerifyResponse | null> => {
    const res = await apiFetch(`/v1/nni/join/verify`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ task_id: taskId, node_url: nodeUrl, signature }),
    });
    const body = (await res.json()) as ApiResponse<NniJoinVerifyResponse>;
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(nniJoinErrorMessage(body.error, body.data, `NNI join verify failed (${res.status})`, lang));
    }
    return body.data;
  };

  const nniRemoteNodeUrls = () => parseNniRemoteNodeUrls(nniRemoteNodes);

  const applyNniConfigResponse = (config: NniConfigResponse) => {
    setNniJoined(config.joined);
    setNniRemoteNodes(config.remote_nodes.join("\n"));
    setNniHeartbeatRequestCount(config.heartbeat_request_count ?? 0);
    setNniHeartbeatRetryLimit(config.heartbeat_network_retry_limit ?? 3);
    setNniLastHeartbeatAtTs(config.last_heartbeat_at_ts ?? null);
    setNniLastHeartbeatNetworkFailures(config.last_heartbeat_network_failures ?? 0);
  };

  const setNniJoinedPersisted = async (joined: boolean, options?: { persistRemoteNodes?: boolean }) => {
    setNniJoined(joined);
    try {
      const payload: { joined: boolean; remote_nodes?: string[] } = { joined };
      if (options?.persistRemoteNodes) {
        payload.remote_nodes = nniRemoteNodeUrls();
      }
      const res = await apiFetch(`/v1/nni/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const body = (await res.json()) as ApiResponse<NniConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `NNI config update failed (${res.status})`);
      }
      applyNniConfigResponse(body.data);
      setNniConfigError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniConfigError(message);
    }
  };

  const fetchNniConfig = async (silent = false) => {
    if (!silent) setNniConfigLoading(true);
    setNniConfigError(null);
    try {
      const res = await apiFetch(`/v1/nni/config`);
      const body = (await res.json()) as ApiResponse<NniConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `NNI config load failed (${res.status})`);
      }
      applyNniConfigResponse(body.data);
      if (!silent) setNniConfigMessage(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniConfigError(message);
    } finally {
      if (!silent) setNniConfigLoading(false);
    }
  };

  const fetchNniHeartbeatRecords = async (page = nniHeartbeatRecordsPage, silent = false) => {
    const safePage = Math.max(1, page);
    if (!silent) {
      setNniHeartbeatRecordsLoading(true);
      setNniHeartbeatRecordsError(null);
      setNniHeartbeatRecordsMessage(null);
    }
    try {
      const params = new URLSearchParams({
        page: String(safePage),
        per_page: String(NNI_HEARTBEAT_RECORDS_PAGE_SIZE),
      });
      const res = await apiFetch(`/v1/nni/records?${params.toString()}`);
      const body = (await res.json()) as ApiResponse<NniHeartbeatRecordsResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `NNI request records load failed (${res.status})`);
      }
      setNniHeartbeatRecords(body.data.records ?? []);
      setNniHeartbeatRecordsPage(body.data.page || safePage);
      setNniHeartbeatRecordsTotal(body.data.total ?? 0);
      setNniHeartbeatRecordsTotalPages(Math.max(1, body.data.total_pages ?? 1));
      setNniHeartbeatRecordsError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      if (!silent) setNniHeartbeatRecordsError(message);
    } finally {
      if (!silent) setNniHeartbeatRecordsLoading(false);
    }
  };

  const clearNniHeartbeatRecords = async () => {
    const confirmed = window.confirm(
      t(
        "确定清理本机 NNI 请求记录吗？这只会清理本机保存的加入和心跳历史，不会修改远程 NNI 服务端记录。",
        "Clear local NNI request records? This only clears Join and Heartbeat history saved on this device and will not change remote NNI server records.",
      ),
    );
    if (!confirmed) return;
    setNniHeartbeatRecordsClearing(true);
    setNniHeartbeatRecordsError(null);
    setNniHeartbeatRecordsMessage(null);
    try {
      const res = await apiFetch(`/v1/nni/records/clear`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({}),
      });
      const rawText = await res.text();
      let body: ApiResponse<{ deleted_records?: number }>;
      try {
        body = JSON.parse(rawText) as ApiResponse<{ deleted_records?: number }>;
      } catch {
        throw new Error(t("NNI 请求记录清理接口返回了非 JSON 内容。", "The NNI request records clear endpoint returned non-JSON content."));
      }
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `NNI request records clear failed (${res.status})`);
      }
      const deletedRecords = body.data?.deleted_records ?? 0;
      setNniHeartbeatRecords([]);
      setNniHeartbeatRecordsPage(1);
      setNniHeartbeatRecordsTotal(0);
      setNniHeartbeatRecordsTotalPages(1);
      setNniHeartbeatRecordsMessage(
        t(
          `已清理 ${deletedRecords} 条本机 NNI 请求记录。`,
          `${deletedRecords} local NNI request records cleared.`,
        ),
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniHeartbeatRecordsError(message);
    } finally {
      setNniHeartbeatRecordsClearing(false);
    }
  };

  const fetchNniHeartbeatErrors = async (page = nniHeartbeatErrorsPage, silent = false) => {
    const safePage = Math.max(1, page);
    if (!silent) {
      setNniHeartbeatErrorsLoading(true);
      setNniHeartbeatErrorsError(null);
      setNniHeartbeatErrorsMessage(null);
    }
    try {
      const params = new URLSearchParams({
        page: String(safePage),
        per_page: String(NNI_HEARTBEAT_ERRORS_PAGE_SIZE),
      });
      const res = await apiFetch(`/v1/nni/heartbeat/errors?${params.toString()}`);
      const rawText = await res.text();
      let body: ApiResponse<NniHeartbeatErrorsResponse>;
      try {
        body = JSON.parse(rawText) as ApiResponse<NniHeartbeatErrorsResponse>;
      } catch {
        const trimmed = rawText.trim().toLowerCase();
        if (trimmed.startsWith("<!doctype") || trimmed.startsWith("<html")) {
          throw new Error(
            t(
              "后端心跳错误接口还不可用，通常是 clawd 还没更新或正在重启。请等待编译重启完成后再刷新。",
              "The backend heartbeat error endpoint is not available yet. clawd is usually still updating or restarting; refresh after the build restart completes.",
            ),
          );
        }
        throw new Error(t("NNI 心跳错误接口返回了非 JSON 内容。", "The NNI heartbeat error endpoint returned non-JSON content."));
      }
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `NNI heartbeat errors load failed (${res.status})`);
      }
      setNniHeartbeatErrors(body.data.records ?? []);
      setNniHeartbeatErrorsPage(body.data.page || safePage);
      setNniHeartbeatErrorsTotal(body.data.total ?? 0);
      setNniHeartbeatErrorsTotalPages(Math.max(1, body.data.total_pages ?? 1));
      setNniHeartbeatErrorsError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      if (!silent) setNniHeartbeatErrorsError(message);
    } finally {
      if (!silent) setNniHeartbeatErrorsLoading(false);
    }
  };

  const clearNniHeartbeatErrors = async () => {
    const confirmed = window.confirm(
      t(
        "确定清理本机心跳错误记录吗？这只会清理本机页面里的错误历史，不会修改远程 NNI 服务端请求记录。",
        "Clear local heartbeat error history? This only clears the local error history shown here and will not change remote NNI server request records.",
      ),
    );
    if (!confirmed) return;
    setNniHeartbeatErrorsClearing(true);
    setNniHeartbeatErrorsError(null);
    setNniHeartbeatErrorsMessage(null);
    try {
      const res = await apiFetch(`/v1/nni/heartbeat/errors/clear`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({}),
      });
      const rawText = await res.text();
      let body: ApiResponse<{ deleted_records?: number }>;
      try {
        body = JSON.parse(rawText) as ApiResponse<{ deleted_records?: number }>;
      } catch {
        throw new Error(t("NNI 心跳错误清理接口返回了非 JSON 内容。", "The NNI heartbeat error clear endpoint returned non-JSON content."));
      }
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `NNI heartbeat errors clear failed (${res.status})`);
      }
      const deletedRecords = body.data?.deleted_records ?? 0;
      setNniHeartbeatErrors([]);
      setNniHeartbeatErrorsPage(1);
      setNniHeartbeatErrorsTotal(0);
      setNniHeartbeatErrorsTotalPages(1);
      setNniHeartbeatErrorsMessage(
        t(
          `已清理 ${deletedRecords} 条本机心跳错误记录。`,
          `${deletedRecords} local heartbeat error records cleared.`,
        ),
      );
      await fetchNniConfig(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniHeartbeatErrorsError(message);
    } finally {
      setNniHeartbeatErrorsClearing(false);
    }
  };

  const saveNniConfig = async () => {
    setNniConfigSaving(true);
    setNniConfigError(null);
    setNniConfigMessage(null);
    try {
      const res = await apiFetch(`/v1/nni/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ remote_nodes: nniRemoteNodeUrls() }),
      });
      const body = (await res.json()) as ApiResponse<NniConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `NNI config save failed (${res.status})`);
      }
      applyNniConfigResponse(body.data);
      setNniConfigMessage(t("远程 NNI 节点已保存到配置文件。", "Remote NNI nodes were saved to the config file."));
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniConfigError(message);
    } finally {
      setNniConfigSaving(false);
    }
  };

  const testJoinNni = async () => {
    const status = nniStatus ?? (await fetchNniDeviceStatus(false));
    if (!status?.signature_chip_present) {
      setNniActionError(
        status?.message ||
          t(
            "未检测到设备签名芯片，暂时不能执行 NNI 测试加入。",
            "No device signature chip was detected, so this device cannot run the NNI test join yet.",
          ),
      );
      await setNniJoinedPersisted(false);
      return;
    }
    const result = await runNniDeviceAction("sign_timestamp");
    if (result?.payload?.signature) {
      setNniActionMessage(
        t(
          "测试签名已完成：本机已生成时间戳签名。只有点击加入并通过服务端验签后，才会开启运行状态。",
          "Test signature completed: this device generated a timestamp signature. The runtime starts only after Join passes server verification.",
        ),
      );
    }
  };

  const joinNni = async () => {
    setNniActionLoading("join_nni");
    setNniActionError(null);
    setNniActionMessage(null);
    const status = nniStatus ?? (await fetchNniDeviceStatus(false));
    if (!status?.signature_chip_present) {
      setNniActionError(
        status?.message ||
          t(
            "未检测到设备签名芯片，暂时不能加入需要设备签名的 NNI。",
            "No device signature chip was detected, so this device cannot join signed NNI yet.",
          ),
      );
      await setNniJoinedPersisted(false);
      setNniActionLoading(null);
      return;
    }
    try {
      const task = await requestNniJoinTask();
      if (!task?.challenge) {
        throw new Error("nni_join_challenge_missing");
      }
      const signatureResult = await runNniDeviceAction("sign_challenge", { challenge: task.challenge });
      const signature = signatureResult?.payload?.signature;
      if (!signature) {
        throw new Error("nni_join_signature_missing");
      }
      setNniActionLoading("join_nni");
      const verified = await verifyNniJoinTask(task.task_id, task.node_url, signature);
      if (!verified?.joined || !verified.compliant) {
        throw new Error("nni_join_verify_rejected");
      }
      await setNniJoinedPersisted(true, { persistRemoteNodes: true });
      setNniActionMessage(
        t(
          "设备签名已通过服务端验证，NNI 已开始运行。",
          "The device signature was verified by the server, and NNI is now running.",
        ),
      );
      await fetchNniHeartbeatRecords(1, true);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniActionError(message);
      await setNniJoinedPersisted(false);
    } finally {
      setNniActionLoading(null);
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

  const setWechatConfigDraftField = <K extends keyof WechatConfigResponse>(key: K, value: WechatConfigResponse[K]) => {
    setWechatConfigDraft((prev) => (prev ? { ...prev, [key]: value } : prev));
  };

  const setTelegramPrimaryBotDraftField = (key: keyof TelegramBotConfigItem, value: TelegramBotConfigItem[keyof TelegramBotConfigItem]) => {
    setTelegramConfigDraft((prev) => {
      if (!prev) return prev;
      const bots = prev.bots.length > 0 ? [...prev.bots] : [buildDefaultTelegramBot()];
      const primaryIndex = bots.findIndex((bot) => bot.is_primary);
      const targetIndex = primaryIndex >= 0 ? primaryIndex : 0;
      bots[targetIndex] = {
        ...(bots[targetIndex] ?? buildDefaultTelegramBot()),
        [key]: value,
        is_primary: true,
      };
      return { ...prev, bots };
    });
  };

  const saveWechatConfig = async () => {
    if (!wechatConfigDraft) return;
    setWechatConfigSaving(true);
    setWechatConfigSaveMessage(null);
    setWechatConfigError(null);
    try {
      const res = await apiFetch(`/v1/wechat/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          enabled: wechatConfigDraft.enabled,
          listen: wechatConfigDraft.listen,
          clawd_base_url: wechatConfigDraft.clawd_base_url,
          api_base_url: wechatConfigDraft.api_base_url,
          wechat_uin_base64: wechatConfigDraft.wechat_uin_base64,
          request_timeout_seconds: wechatConfigDraft.request_timeout_seconds,
          longpoll_timeout_ms: wechatConfigDraft.longpoll_timeout_ms,
          text_chunk_chars: wechatConfigDraft.text_chunk_chars,
        }),
      });
      const body = (await res.json()) as ApiResponse<WechatConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `微信配置保存失败 (${res.status})`);
      }
      setWechatConfigData(body.data);
      setWechatConfigDraft(body.data);
      setWechatConfigSaveMessage(
        t(
          "微信配置已保存。请到 Services 页重启 wechatd，让新配置生效。",
          "WeChat config was saved. Restart wechatd from the Services page to apply it.",
        ),
      );
      await fetchWechatLoginStatus(true);
      await fetchHealth();
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setWechatConfigError(message);
    } finally {
      setWechatConfigSaving(false);
    }
  };

  const saveTelegramConfig = async () => {
    if (!telegramConfigDraft) return;
    setTelegramConfigSaving(true);
    setTelegramConfigSaveMessage(null);
    setTelegramConfigError(null);
    try {
      const bots = telegramConfigDraft.bots.length > 0 ? telegramConfigDraft.bots : [buildDefaultTelegramBot()];
      const normalizedBots = bots.map((bot, index) => ({
        ...bot,
        name: bot.name?.trim() || (index === 0 ? "primary" : `bot-${index + 1}`),
        bot_token: bot.bot_token?.trim() || "",
        agent_id: bot.agent_id?.trim() || "main",
        is_primary: index === 0 ? true : bot.is_primary,
      }));
      const res = await apiFetch(`/v1/telegram/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          bots: normalizedBots,
          agents: telegramConfigDraft.agents ?? [],
        }),
      });
      const body = (await res.json()) as ApiResponse<TelegramConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `Telegram 配置保存失败 (${res.status})`);
      }
      setTelegramConfigData(body.data);
      setTelegramConfigDraft(body.data);
      setTelegramConfigSaveMessage(
        t(
          "Telegram 配置已保存。下一步请启动 telegramd，然后发一条测试消息。",
          "Telegram config was saved. Next, start telegramd and send a test message.",
        ),
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setTelegramConfigError(message);
    } finally {
      setTelegramConfigSaving(false);
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
          vendor_api_format: llmVendorSupportsApiFormat(llmDraftVendor) ? llmDraftApiFormat : undefined,
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

  const testLlmConfig = async () => {
    if (!llmDraftVendor || !llmDraftModel || !llmDraftBaseUrl.trim()) {
      setLlmTestMessage(null);
      setLlmTestError(
        t(
          "请先补齐厂商、模型和 Base URL，再测试连接。",
          "Please fill in vendor, model, and base URL before testing the connection.",
        ),
      );
      return;
    }
    setLlmTestLoading(true);
    setLlmTestMessage(null);
    setLlmTestError(null);
    try {
      const res = await apiFetch(`/v1/llm/test`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          selected_vendor: llmDraftVendor,
          selected_model: llmDraftModel,
          vendor_base_url: llmDraftBaseUrl,
          vendor_api_key: llmDraftApiKey.trim(),
          vendor_api_format: llmVendorSupportsApiFormat(llmDraftVendor) ? llmDraftApiFormat : undefined,
        }),
      });
      const body = (await res.json()) as ApiResponse<LlmTestResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `模型连接测试失败 (${res.status})`);
      }
      const message = hasUnsavedLlmChanges
        ? `${body.data.message}${t(
            " 这是页面里的临时草稿；确认没问题后，再点“保存模型设置”。",
            " This used the current draft values; save the settings once you're happy with them.",
          )}`
        : body.data.message;
      setLlmTestMessage(message);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setLlmTestError(message);
    } finally {
      setLlmTestLoading(false);
    }
  };

  const fetchMultimodalConfig = async () => {
    setMultimodalConfigLoading(true);
    setMultimodalConfigError(null);
    try {
      const res = await apiFetch("/v1/admin/model-config");
      const body = (await res.json()) as ApiResponse<ModelConfigResponse>;
      if (!res.ok || !body.ok || !body.data) throw new Error(body.error || "fetch failed");
      setMultimodalConfigData(body.data);
      setMultimodalDraft(buildMultimodalDraft(body.data));
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
      const payload = buildMultimodalSavePayload(multimodalDraft);
      const res = await apiFetch("/v1/admin/model-config", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const body = (await res.json()) as ApiResponse<{ restart_required?: boolean }>;
      if (!res.ok || !body.ok) throw new Error(body.error || "save failed");
      setMultimodalConfigSaveMessage(t("多模态模块配置已保存，需重启 clawd 生效。", "Multimodal config saved. Restart clawd to apply."));
      await fetchMultimodalConfig();
    } catch (err) {
      setMultimodalConfigError(err instanceof Error ? err.message : "Unknown");
    } finally {
      setMultimodalConfigSaving(false);
    }
  };

  const setMultimodalDraftKey = (key: MultimodalKey, field: keyof ModelConfigItem, value: string) => {
    setMultimodalDraft((prev) => updateMultimodalDraftField(prev, key, field, value));
  };

  const hasUnsavedMultimodalChanges = useMemo(() => {
    return hasUnsavedMultimodalDraftChanges(multimodalConfigData, multimodalDraft);
  }, [multimodalConfigData, multimodalDraft]);

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

  useEffect(() => {
    setLlmTestMessage(null);
    setLlmTestError(null);
  }, [llmDraftApiFormat, llmDraftApiKey, llmDraftBaseUrl, llmDraftModel, llmDraftVendor]);

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
  const sortedAuthKeysList = useMemo(
    () =>
      [...authKeysList].sort((a, b) => {
        const aPriority = a.role === "admin" ? 0 : 1;
        const bPriority = b.role === "admin" ? 0 : 1;
        if (aPriority !== bPriority) return aPriority - bPriority;
        return b.created_at.localeCompare(a.created_at);
      }),
    [authKeysList],
  );
  const selectedChannelPreset = useMemo(() => channelPresets[channelBindingChannel], [channelBindingChannel, channelPresets]);
  const hasUnsavedWechatConfigChanges = useMemo(() => {
    if (!wechatConfigData || !wechatConfigDraft) return false;
    return JSON.stringify({
      enabled: wechatConfigData.enabled,
      listen: wechatConfigData.listen,
      clawd_base_url: wechatConfigData.clawd_base_url,
      api_base_url: wechatConfigData.api_base_url,
      wechat_uin_base64: wechatConfigData.wechat_uin_base64,
      request_timeout_seconds: wechatConfigData.request_timeout_seconds,
      longpoll_timeout_ms: wechatConfigData.longpoll_timeout_ms,
      text_chunk_chars: wechatConfigData.text_chunk_chars,
    }) !== JSON.stringify({
      enabled: wechatConfigDraft.enabled,
      listen: wechatConfigDraft.listen,
      clawd_base_url: wechatConfigDraft.clawd_base_url,
      api_base_url: wechatConfigDraft.api_base_url,
      wechat_uin_base64: wechatConfigDraft.wechat_uin_base64,
      request_timeout_seconds: wechatConfigDraft.request_timeout_seconds,
      longpoll_timeout_ms: wechatConfigDraft.longpoll_timeout_ms,
      text_chunk_chars: wechatConfigDraft.text_chunk_chars,
    });
  }, [wechatConfigData, wechatConfigDraft]);
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
  const selectedLlmVendorInfo = useMemo(
    () => llmConfigData?.vendors.find((vendor) => vendor.name === llmDraftVendor) ?? null,
    [llmConfigData, llmDraftVendor],
  );
  const hasCustomLlmVendor = useMemo(
    () => (llmConfigData?.vendors ?? []).some((vendor) => vendor.name === "custom"),
    [llmConfigData],
  );
  const hasUnsavedLlmChanges = useMemo(() => {
    return hasUnsavedLlmDraftChanges(
      llmConfigData
        ? {
            selectedVendor: llmConfigData.selected_vendor || "",
            selectedModel: llmConfigData.selected_model || "",
            vendors: llmConfigData.vendors,
            draftVendor: llmDraftVendor,
            draftModel: llmDraftModel,
            draftBaseUrl: llmDraftBaseUrl,
            draftApiKey: llmDraftApiKey,
            draftApiFormat: llmDraftApiFormat,
          }
        : null,
    );
  }, [llmConfigData, llmDraftApiFormat, llmDraftApiKey, llmDraftBaseUrl, llmDraftModel, llmDraftVendor]);
  const llmRestartPending = useMemo(() => {
    if (!llmConfigData) return false;
    const runtimeVendor = llmConfigData.runtime?.vendor?.trim() || "";
    const runtimeModel = llmConfigData.runtime?.model?.trim() || "";
    const savedVendor = llmConfigData.selected_vendor?.trim() || "";
    const savedModel = llmConfigData.selected_model?.trim() || "";
    return llmConfigData.restart_required || runtimeVendor !== savedVendor || runtimeModel !== savedModel;
  }, [llmConfigData]);
  const savedLlmVendorInfo = useMemo(
    () => llmConfigData?.vendors.find((vendor) => vendor.name === llmConfigData.selected_vendor) ?? null,
    [llmConfigData],
  );
  const llmConfigured = useMemo(() => {
    if (!llmConfigData?.selected_vendor || !llmConfigData.selected_model) return false;
    if (!savedLlmVendorInfo) return false;
    return savedLlmVendorInfo.api_key_configured;
  }, [llmConfigData, savedLlmVendorInfo]);
  const llmStepStatus = useMemo<"done" | "attention" | "todo">(() => {
    if (!llmConfigured) return "todo";
    return llmRestartPending ? "attention" : "done";
  }, [llmConfigured, llmRestartPending]);
  const primaryTelegramBot = useMemo(() => {
    const bots = telegramConfigDraft?.bots ?? telegramConfigData?.bots ?? [];
    return bots.find((bot) => bot.is_primary) ?? bots[0] ?? buildDefaultTelegramBot();
  }, [telegramConfigData, telegramConfigDraft]);
  const telegramBotTokenConfigured = useMemo(() => {
    const token = primaryTelegramBot.bot_token?.trim() || "";
    return (token.length > 0 && token !== "REPLACE_ME") || primaryTelegramBot.bot_token_configured === true;
  }, [primaryTelegramBot]);
  const hasUnsavedTelegramConfigChanges = useMemo(() => {
    if (!telegramConfigData || !telegramConfigDraft) return false;
    return JSON.stringify(telegramConfigData) !== JSON.stringify(telegramConfigDraft);
  }, [telegramConfigData, telegramConfigDraft]);
  const healthStatusLoading = health == null && error == null;
  const wechatStatusLoading = healthStatusLoading || (wechatConfigData == null && wechatConfigError == null);
  const telegramStatusLoading = healthStatusLoading || (telegramConfigData == null && telegramConfigError == null);
  const feishuStatusLoading = healthStatusLoading || (feishuConfigData == null && feishuConfigError == null);
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
  const normalizedSkillsSearchQuery = useMemo(() => normalizeSkillSearchQuery(skillsSearchQuery), [skillsSearchQuery]);
  const filteredManagedSkills = useMemo(
    () => filterSkillNamesBySearch(managedSkills, normalizedSkillsSearchQuery),
    [managedSkills, normalizedSkillsSearchQuery],
  );

  /** 能力分组：工具 / 图片 / 语音 / 基础 / 其他 */
  const skillGroups = useMemo(
    () => groupSkillNames(managedSkills, baseSkillNamesSet, toolSkillNamesSet),
    [managedSkills, baseSkillNamesSet, toolSkillNamesSet],
  );
  const filteredSkillsTool = useMemo(() => filterSkillNamesBySearch(skillGroups.tool, normalizedSkillsSearchQuery), [skillGroups.tool, normalizedSkillsSearchQuery]);
  const filteredSkillsImage = useMemo(() => filterSkillNamesBySearch(skillGroups.image, normalizedSkillsSearchQuery), [skillGroups.image, normalizedSkillsSearchQuery]);
  const filteredSkillsAudio = useMemo(() => filterSkillNamesBySearch(skillGroups.audio, normalizedSkillsSearchQuery), [skillGroups.audio, normalizedSkillsSearchQuery]);
  const filteredSkillsBase = useMemo(() => filterSkillNamesBySearch(skillGroups.base, normalizedSkillsSearchQuery), [skillGroups.base, normalizedSkillsSearchQuery]);
  const filteredSkillsOther = useMemo(() => filterSkillNamesBySearch(skillGroups.other, normalizedSkillsSearchQuery), [skillGroups.other, normalizedSkillsSearchQuery]);
  useEffect(() => {
    if (!skillImportPreview) return;
    if (managedSkills.includes(skillImportPreview.skill_name)) return;
    setSkillImportPreview(null);
    if (recentImportedSkillName === skillImportPreview.skill_name) {
      setRecentImportedSkillName(null);
    }
  }, [managedSkills, recentImportedSkillName, skillImportPreview]);
  const visibleRuntimeSkills = useMemo(
    () => visibleSkillNames(skillsData?.skills),
    [skillsData],
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
  const describeSkill = (name: string) => {
    return skillDescription(name, lang, skillItemsByName.get(name)?.description);
  };
  const applyLlmVendorDraft = (nextVendor: string) => {
    const vendorInfo = llmConfigData?.vendors.find((vendor) => vendor.name === nextVendor);
    setLlmDraftVendor(nextVendor);
    if (!vendorInfo) {
      setLlmDraftModel("");
      setLlmDraftBaseUrl("");
      setLlmDraftApiKey("");
      setLlmDraftApiFormat("");
      return;
    }
    const nextModel = vendorInfo.default_model || vendorInfo.models[0] || "";
    setLlmDraftModel(nextModel);
    setLlmDraftBaseUrl(vendorInfo.base_url || "");
    setLlmDraftApiKey(vendorInfo.api_key || "");
    setLlmDraftApiFormat(llmVendorSupportsApiFormat(vendorInfo.name) ? (vendorInfo.api_format || "openai_compat") : "");
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
  const nniActionLabel = (action: string) => formatNniActionLabel(action, lang);
  const nniChipPresent = nniStatus?.signature_chip_present === true;
  const nniChipMissing = nniStatus?.signature_chip_present === false;
  const nniPrimaryHex = nniPayloadHexField(nniActionResult?.payload);
  const nniRemoteNodeCount = nniRemoteNodeUrls().length;
  const nniHeartbeatRecordsCanPrev = nniHeartbeatRecordsPage > 1;
  const nniHeartbeatRecordsCanNext = nniHeartbeatRecordsPage < nniHeartbeatRecordsTotalPages;
  const nniHeartbeatErrorsCanPrev = nniHeartbeatErrorsPage > 1;
  const nniHeartbeatErrorsCanNext = nniHeartbeatErrorsPage < nniHeartbeatErrorsTotalPages;
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
  const isDashboardPage = currentPage === "dashboard";
  const factoryResetCanConfirm =
    factoryResetCountdown <= 0 &&
    factoryResetConfirmText.trim() === FACTORY_RESET_CONFIRM_WORD &&
    !factoryResetLoading &&
    !factoryResetResult;
  const factoryResetDeletedTotal = factoryResetResult?.database
    ? Object.values(factoryResetResult.database).reduce((sum, value) => sum + (Number.isFinite(value) ? value : 0), 0)
    : 0;
  const factoryResetModal = factoryResetDialogOpen ? (
    <div className="fixed inset-0 z-[90] flex items-center justify-center bg-black/70 px-3 py-6 backdrop-blur-sm">
      <div className="w-full max-w-2xl rounded-2xl border border-red-400/30 bg-[#151923] p-5 text-white shadow-2xl sm:p-6">
        <div className="flex items-start gap-3">
          <div className="rounded-2xl border border-red-400/35 bg-red-500/15 p-3 text-red-100">
            <ShieldAlert className="h-6 w-6" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[11px] uppercase tracking-[0.24em] text-red-200/80">
              {t("危险操作", "Danger Zone")}
            </p>
            <h3 className="mt-2 text-lg font-semibold tracking-tight sm:text-2xl">
              {factoryResetResult ? t("恢复出厂设置已完成", "Factory Reset Complete") : t("恢复出厂设置", "Factory Reset")}
            </h3>
            <p className="mt-2 text-sm leading-7 text-white/70">
              {factoryResetResult
                ? t(
                    "旧登录凭据已经失效。请使用下面的新凭据重新进入控制台。",
                    "The old credentials are no longer valid. Use the credentials below to sign in again.",
                  )
                : t(
                    "这会删除所有记忆、所有日志、配置文件里的密钥字段、其它用户 key 与通道绑定，并重置管理员登录。",
                    "This deletes all memories, all logs, secret fields in config files, other user keys, and channel bindings, then resets the admin login.",
                  )}
            </p>
          </div>
        </div>

        {factoryResetResult ? (
          <div className="mt-5 space-y-4">
            <div className="rounded-xl border border-emerald-400/25 bg-emerald-400/10 px-4 py-3 text-sm text-emerald-100">
              {t("新的管理员凭据已生成。", "New admin credentials have been generated.")}
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="rounded-xl border border-white/10 bg-black/25 px-4 py-3">
                <p className="text-[11px] uppercase tracking-widest text-white/45">Admin Key</p>
                <p className="mt-2 break-all font-mono text-sm text-white/90">{factoryResetResult.admin_user_key}</p>
                <button
                  type="button"
                  onClick={() => void writeTextToClipboard(factoryResetResult.admin_user_key)}
                  className="mt-3 inline-flex items-center gap-2 rounded-lg border border-white/15 bg-white/5 px-2.5 py-1.5 text-xs text-white/80 hover:bg-white/10"
                >
                  <Copy className="h-3.5 w-3.5" />
                  {t("复制", "Copy")}
                </button>
              </div>
              <div className="rounded-xl border border-white/10 bg-black/25 px-4 py-3">
                <p className="text-[11px] uppercase tracking-widest text-white/45">{t("Web 登录", "Web Login")}</p>
                <p className="mt-2 text-sm text-white/75">
                  {t("用户名", "Username")}: <span className="font-mono text-white">{factoryResetResult.webd_username}</span>
                </p>
                <p className="mt-1 text-sm text-white/75">
                  {t("密码", "Password")}: <span className="font-mono text-white">{factoryResetResult.webd_password}</span>
                </p>
              </div>
            </div>
            <div className="grid gap-3 text-xs text-white/55 sm:grid-cols-3">
              <div className="rounded-xl border border-white/10 bg-white/5 px-3 py-2">
                {t("数据库删除记录", "Database rows deleted")}: <span className="text-white/85">{factoryResetDeletedTotal}</span>
              </div>
              <div className="rounded-xl border border-white/10 bg-white/5 px-3 py-2">
                {t("配置字段清空", "Config fields cleared")}: <span className="text-white/85">{factoryResetResult.config?.fields_cleared ?? 0}</span>
              </div>
              <div className="rounded-xl border border-white/10 bg-white/5 px-3 py-2">
                {t("日志文件删除", "Log files deleted")}: <span className="text-white/85">{factoryResetResult.logs?.files_deleted ?? 0}</span>
              </div>
            </div>
            {(factoryResetResult.warnings?.length ?? 0) > 0 ? (
              <div className="max-h-28 overflow-auto rounded-xl border border-amber-400/25 bg-amber-400/10 px-3 py-2 text-xs leading-5 text-amber-100">
                {factoryResetResult.warnings?.map((warning) => <p key={warning}>{warning}</p>)}
              </div>
            ) : null}
          </div>
        ) : (
          <div className="mt-5 space-y-4">
            <div className="grid gap-2 text-sm text-white/72 sm:grid-cols-2">
              {[
                t("删除所有记忆和长期事实", "Delete all memories and long-term facts"),
                t("删除 logs 文件夹下所有日志", "Delete every log under the logs folder"),
                t("清空配置里的 key/token/secret/password", "Clear key/token/secret/password fields in configs"),
                t("删除其它用户 key 与绑定", "Delete other user keys and bindings"),
                t("重置 admin key", "Reset the admin key"),
                t("用户名重置为 rustclaw，密码重置为 123456", "Reset username to rustclaw and password to 123456"),
              ].map((item) => (
                <div key={item} className="flex items-start gap-2 rounded-xl border border-white/10 bg-white/5 px-3 py-2">
                  <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-red-200" />
                  <span>{item}</span>
                </div>
              ))}
            </div>

            <div className="rounded-xl border border-red-400/25 bg-red-500/10 px-4 py-3">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <p className="text-sm font-medium text-red-100">
                  {factoryResetCountdown > 0
                    ? t(`请等待 ${factoryResetCountdown} 秒`, `Wait ${factoryResetCountdown}s`)
                    : t("倒计时结束，可以继续确认。", "Countdown complete. You can continue.")}
                </p>
                <span className="rounded-full border border-red-300/25 bg-black/20 px-3 py-1 font-mono text-sm text-red-100">
                  {factoryResetCountdown}s
                </span>
              </div>
              <label className="mt-3 block space-y-2">
                <span className="text-xs text-red-100/75">
                  {t(`输入 ${FACTORY_RESET_CONFIRM_WORD} 确认执行`, `Type ${FACTORY_RESET_CONFIRM_WORD} to confirm`)}
                </span>
                <input
                  className="theme-input"
                  value={factoryResetConfirmText}
                  onChange={(e) => setFactoryResetConfirmText(e.target.value)}
                  disabled={factoryResetLoading}
                  autoComplete="off"
                />
              </label>
            </div>
          </div>
        )}

        {factoryResetError ? (
          <p className="mt-4 rounded-xl border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-100">
            {factoryResetError}
          </p>
        ) : null}

        <div className="mt-5 flex flex-wrap items-center justify-end gap-3">
          <button
            type="button"
            onClick={closeFactoryResetDialog}
            disabled={factoryResetLoading}
            className="theme-secondary-btn px-4 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-50"
          >
            {factoryResetResult ? t("返回登录", "Back to Sign In") : t("取消", "Cancel")}
          </button>
          {!factoryResetResult ? (
            <button
              type="button"
              onClick={() => void runFactoryReset()}
              disabled={!factoryResetCanConfirm}
              className="inline-flex items-center gap-2 rounded-xl border border-red-400/35 bg-red-500/20 px-4 py-2 text-sm font-semibold text-red-50 transition hover:bg-red-500/25 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {factoryResetLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <Trash2 className="h-4 w-4" />}
              {factoryResetLoading ? t("正在恢复", "Resetting") : t("确认恢复出厂设置", "Confirm Factory Reset")}
            </button>
          ) : null}
        </div>
      </div>
    </div>
  ) : null;

  if (!uiAuthReady) {
    return (
      <Fragment>
        <div className="theme-shell min-h-screen px-4 py-8">
          <div className="mx-auto grid max-w-5xl gap-6 lg:grid-cols-[1.1fr_0.9fr]">
            <div className="theme-panel p-6 sm:p-8">
              <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">{t("欢迎", "Welcome")}</p>
              <h1 className="mt-4 flex items-center gap-2 text-2xl font-bold sm:text-3xl">
                <img className="rustclaw-logo rustclaw-logo-hero" src="/rustclaw-logo.svg" alt="" />
                <span>{t("进入 RustClaw 控制台", "Enter RustClaw Console")}</span>
              </h1>
              <p className="mt-4 max-w-xl text-sm leading-7 text-white/70 sm:text-base">
                {t(
                  "这是给普通用户准备的可视化面板。你不需要先懂命令行，只要填好服务地址、用户名和密码，就能查看状态、绑定账号、测试消息。",
                  "This is a visual panel designed for everyday users. You do not need the command line first; enter the service address, username, and password to check status, bind accounts, and test messages.",
                )}
              </p>

              <div className="mt-6 rounded-2xl border border-white/10 bg-black/20 p-4">
                <p className="text-sm font-semibold text-white">{t("登录前你需要什么？", "What do you need before signing in?")}</p>
                <ol className="mt-3 list-decimal space-y-2 pl-5 text-sm text-white/65">
                  <li>{t("一个已经启动的 RustClaw 服务地址。", "A running RustClaw service address.")}</li>
                  <li>{t("你的网页登录用户名和密码。", "Your web login username and password.")}</li>
                  <li>{t("如果不知道接下来该做什么，登录后先看首页。", "If you are not sure what to do next, start with Home after signing in.")}</li>
                </ol>
              </div>
            </div>

            <div className="theme-panel p-6">
            <div className="mb-6">
              <h2 className="text-xl font-bold">{t("登录", "Sign in")}</h2>
              <p className="mt-2 text-sm text-white/60">
                {loginTab === "key"
                  ? t("使用 Access Key 验证后进入控制台。", "Verify with an access key to enter the console.")
                  : t("默认使用用户名和密码登录。", "Username and password sign-in is the default.")}
              </p>
            </div>

            <div className="space-y-4">
              {loginTab === "key" ? (
                <>
                  <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-2 text-xs leading-relaxed text-white/55">
                    {t(
                      "Key 登录适合管理员或已经拿到 user_key 的用户。普通用户建议返回用户名密码登录。",
                      "Access key sign-in is for admins or users who already have a user_key. Most users should use username and password sign-in.",
                    )}
                  </div>
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
                      {t("直连 clawd 或经 webd 代理时均可；请与浏览器能访问到的 API 地址一致。", "Use the API URL your browser can reach (direct clawd or via webd).")}
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
                </>
              ) : (
                <>
                  <p className="text-xs leading-relaxed text-white/55">
                    {t(
                      "可填写 webd 地址端口（例如 http://127.0.0.1:8788）；留空则默认走当前页面地址（常见于 nginx 反代）。",
                      "You can enter a webd URL/port (for example http://127.0.0.1:8788); if left empty, current page origin is used (common with nginx reverse proxy).",
                    )}
                  </p>
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">
                      {t("Webd 地址（可选）", "Webd URL (optional)")}
                    </span>
                    <input
                      className="theme-input"
                      value={webdBaseUrlDraft}
                      onChange={(e) => setWebdBaseUrlDraft(e.target.value)}
                      placeholder="http://127.0.0.1:8788"
                    />
                  </label>
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">{t("用户名", "Username")}</span>
                    <input
                      className="theme-input"
                      autoComplete="username"
                      value={webdUsername}
                      onChange={(e) => setWebdUsername(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          e.preventDefault();
                          void loginWebd();
                        }
                      }}
                    />
                  </label>
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">{t("密码", "Password")}</span>
                    <input
                      className="theme-input"
                      type="password"
                      autoComplete="current-password"
                      value={webdPassword}
                      onChange={(e) => setWebdPassword(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          e.preventDefault();
                          void loginWebd();
                        }
                      }}
                    />
                  </label>
                </>
              )}

              {uiAuthError ? (
                <p className="rounded-xl border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                  {uiAuthError}
                </p>
              ) : null}

              <div className="flex flex-wrap items-center gap-3">
                {loginTab === "key" ? (
                  <>
                    <button
                      type="button"
                      onClick={() => void verifyUiKey(uiKeyDraft)}
                      disabled={uiAuthLoading}
                      className="theme-accent-btn"
                    >
                      {uiAuthLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                      {t("进入控制台", "Enter Console")}
                    </button>
                    {uiKey ? (
                      <button
                        type="button"
                        onClick={() => void verifyUiKey(uiKey)}
                        disabled={uiAuthLoading}
                        className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-4 py-2 text-sm font-medium text-white transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-60"
                      >
                        {t("使用已保存 Key", "Use saved key")}
                      </button>
                    ) : null}
                    <button
                      type="button"
                      onClick={() => {
                        setLoginTab("webd");
                        setUiAuthError(null);
                      }}
                      disabled={uiAuthLoading}
                      className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-4 py-2 text-sm font-medium text-white transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-60"
                    >
                      {t("返回用户名密码登录", "Back to username sign-in")}
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      onClick={() => void loginWebd()}
                      disabled={uiAuthLoading}
                      className="theme-accent-btn"
                    >
                      {uiAuthLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                      {t("进入控制台", "Enter Console")}
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setLoginTab("key");
                        setUiAuthError(null);
                      }}
                      disabled={uiAuthLoading}
                      className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-4 py-2 text-sm font-medium text-white transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-60"
                    >
                      <KeyRound className="h-4 w-4" />
                      {t("使用 Key 登录", "Use access key")}
                    </button>
                  </>
                )}
                <button
                  type="button"
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
        {factoryResetModal}
      </Fragment>
    );
  }

  return (
    <div className="theme-shell min-h-screen">
      {factoryResetModal}
      <header className="theme-header sticky top-0 z-40 border-b border-white/10 px-3 sm:px-6">
        <div className="theme-header-inner mx-auto flex min-h-16 w-full max-w-7xl items-center justify-between gap-3 py-2">
          <div className="min-w-0">
            <button
              type="button"
              onClick={() => setCurrentPage("dashboard")}
              className="theme-brand-link inline-flex items-center gap-2 truncate text-left text-lg font-bold tracking-tight transition hover:text-white/85 sm:text-2xl"
            >
              <img className="rustclaw-logo rustclaw-logo-header" src="/rustclaw-logo.svg" alt="" />
              <span>RustClaw</span>
            </button>
          </div>

          <div className="theme-header-actions flex flex-wrap items-center justify-end gap-2">
            {/* 小屏下拉导航，仅在 lg 以下显示 */}
            <div ref={navDropdownRef} className="relative flex items-center lg:hidden">
              <button
                type="button"
                onClick={() => setNavDropdownOpen((v) => !v)}
                className="theme-topbar-nav-btn"
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
                  {isAdminIdentity ? (
                    <div className="mt-1 border-t border-white/10 pt-1">
                      <button
                        type="button"
                        onClick={() => {
                          setNavDropdownOpen(false);
                          openFactoryResetDialog();
                        }}
                        className="flex w-full items-center gap-2 px-3 py-2.5 text-left text-sm text-red-100 transition hover:bg-red-500/10"
                      >
                        <ShieldAlert className="h-4 w-4" />
                        <span>{t("恢复出厂设置", "Factory Reset")}</span>
                      </button>
                    </div>
                  ) : null}
	                </div>
	              )}
            </div>
            <div className="theme-toolbar-shell">
              <button
                type="button"
                onClick={() => setLang((v) => (v === "zh" ? "en" : "zh"))}
                className="theme-toolbar-segment"
                title={t("切换界面语言", "Switch interface language")}
              >
                {lang === "zh" ? "中文" : "English"}
              </button>
              <span className="theme-toolbar-divider" aria-hidden="true" />
              <button
                type="button"
                onClick={() => void logout()}
                className="theme-toolbar-segment theme-toolbar-segment-danger"
                title={
                  authMode === "webd"
                    ? t("退出登录并清除 Web 会话", "Log out and clear web session")
                    : t("退出登录，需重新输入 key", "Log out; key required to sign in again")
                }
              >
                {t("退出", "Log out")}
              </button>
            </div>
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

            {isAdminIdentity ? (
              <button
                type="button"
                onClick={openFactoryResetDialog}
                className="mt-3 flex w-full items-center gap-2 rounded-2xl border border-red-500/25 bg-red-500/10 px-3 py-2.5 text-left text-sm font-medium text-red-100 transition hover:bg-red-500/15"
              >
                <ShieldAlert className="h-4 w-4" />
                <span>{t("恢复出厂设置", "Factory Reset")}</span>
              </button>
            ) : null}

	            <div className="theme-panel-soft mt-3 p-3.5 text-sm text-white/70">
              <p className="font-medium text-white">{t("当前登录身份", "Current identity")}</p>
              {authMode === "webd" ? (
                <div className="mt-2 space-y-1 text-xs text-white/55">
                  <p>{t("Web 会话（由 webd 注入访问凭证，浏览器不保存明文 key）", "Web session (webd injects credentials; no plaintext key in browser)")}</p>
                  <p>
                    {t("角色", "Role")}: <span className="text-white/75">{authIdentity?.role || "--"}</span>
                  </p>
                  <p className="break-all font-mono">
                    {t("Key", "Key")}: <span className="text-white/75">{maskedIdentityKey || "--"}</span>
                  </p>
                </div>
              ) : (
                <p className="mt-2 break-all font-mono text-xs text-white/55">{maskedSavedUiKey || "--"}</p>
              )}
            </div>
          </div>
        </aside>

        <main className="mx-auto min-w-0 max-w-7xl space-y-4">
          {isDashboardPage ? (
            <>
              <section className="theme-panel setup-hero p-5 sm:p-6">
                <div className="max-w-3xl">
                  <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">{t("首次使用", "First run")}</p>
                  <h3 className="mt-2 text-xl font-semibold tracking-tight sm:text-3xl">
                    {t("开始使用 RustClaw", "Start using RustClaw")}
                  </h3>
                  <p className="mt-3 text-sm leading-7 text-white/70 sm:text-base">
                    {t(
                      "请先完成大模型配置和消息测试；如需通过微信使用 RustClaw，再继续完成微信接入。Telegram 仅在你需要时再补充配置。",
                      "Please complete the model setup and a test message first. If you want to use RustClaw through WeChat, continue with the WeChat setup. Add Telegram later only if you need it.",
                    )}
                  </p>
                </div>

                <div className="mt-6 grid gap-3 xl:grid-cols-3">
                  {onboardingSteps.map((step, index) => (
                    <button
                      key={step.key}
                      type="button"
                      onClick={() => setCurrentPage(step.page)}
                      className="setup-step-card setup-step-card-compact text-left"
                    >
                      <span className="setup-step-index setup-step-index-floating">{index + 1}</span>
                      {step.key !== "chat" ? (
                        <span
                          className={
                            step.status === "done"
                              ? "setup-status setup-step-status setup-status-done"
                              : step.status === "attention"
                              ? "setup-status setup-step-status setup-status-attention"
                              : "setup-status setup-step-status setup-status-todo"
                          }
                        >
                          {step.status === "done"
                            ? t("已完成", "Done")
                            : step.status === "attention"
                              ? t("待完成", "Needs attention")
                              : t("未开始", "Not started")}
                        </span>
                      ) : null}
                      <div className="setup-step-card-body">
                        <h4 className="text-base font-semibold text-white">{step.title}</h4>
                        <p className="mt-2 text-sm leading-7 text-white/65">{step.description}</p>
                      </div>
                    </button>
                  ))}
                </div>
              </section>

              <section className="theme-panel-soft rounded-[22px] border border-white/10 px-4 py-3 sm:px-5">
                <div className="grid gap-3 md:grid-cols-3">
                  {dashboardOverviewItems.map((item, index) => (
                    <div
                      key={item.key}
                      className={`py-2 ${
                        index > 0 ? "md:border-l md:border-white/8 md:pl-5" : ""
                      }`}
                    >
                      <p className="text-[11px] tracking-[0.16em] text-white/42">{item.label}</p>
                      <p
                        className={`mt-2 text-base font-semibold ${
                          item.tone === "good"
                            ? "text-emerald-200"
                            : item.tone === "warning"
                            ? "text-amber-200"
                            : "text-white/92"
                        }`}
                      >
                        {item.value}
                      </p>
                    </div>
                  ))}
                </div>
              </section>

              <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="max-w-2xl">
                    <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">
                      {t("系统更新", "System Update")}
                    </p>
                    <h3 className="mt-2 text-base font-semibold text-white">
                      {t("拉取更新或分项编译", "Pull updates or build parts")}
                    </h3>
                    <p className="mt-2 text-sm leading-7 text-white/65">
                      {t(
                        "完整编译会先尝试正常拉取远端版本，再编译并重启 clawd；也可以直接下载 GitHub Release 预编译包部署，避免在设备上长时间编译。",
                        "A full build pulls the remote version first, then builds and restarts clawd. You can also deploy a prebuilt GitHub Release package to avoid long on-device builds.",
                      )}
                    </p>
                  </div>
                  {isAdminIdentity ? (
                    <div className="flex flex-wrap items-center gap-2">
                      <button
                        type="button"
                        onClick={() => void fetchWorkspaceUpdateStatus(false)}
                        disabled={workspaceUpdateLoading || systemRestarting}
                        className="theme-topbar-btn px-3 py-2 text-sm"
                      >
                        {workspaceUpdateLoading && !workspaceUpdateRunning ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <RefreshCw className="h-4 w-4" />
                        )}
                        {t("检查远端版本", "Check remote")}
                      </button>
                      <button
                        type="button"
                        onClick={() => void startWorkspaceUpdate("full")}
                        disabled={workspaceUpdateLoading || workspaceUpdateRunning || systemRestarting}
                        className="theme-accent-btn"
                      >
                        {workspaceUpdateRunning ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <RefreshCw className="h-4 w-4" />
                        )}
                        {workspaceUpdateRunning
                          ? t("编译进行中", "Building")
                          : workspaceUpdateHasRemoteDiff
                            ? t("拉取并编译", "Pull and Build")
                            : t("完整编译", "Build All")}
                      </button>
                      <button
                        type="button"
                        onClick={() => void startWorkspaceUpdate("ui_only")}
                        disabled={workspaceUpdateLoading || workspaceUpdateRunning || systemRestarting}
                        className="theme-secondary-btn px-3 py-2 text-sm"
                      >
                        <LayoutDashboard className="h-4 w-4" />
                        {t("只编译 UI", "Build UI")}
                      </button>
                      <button
                        type="button"
                        onClick={() => void startWorkspaceUpdate("clawd_only")}
                        disabled={workspaceUpdateLoading || workspaceUpdateRunning || systemRestarting}
                        className="theme-secondary-btn px-3 py-2 text-sm"
                      >
                        <Cpu className="h-4 w-4" />
                        {t("只编译 clawd", "Build clawd")}
                      </button>
                      <button
                        type="button"
                        onClick={() => void startWorkspaceUpdate("release_deploy")}
                        disabled={workspaceUpdateLoading || workspaceUpdateRunning || systemRestarting}
                        className="theme-secondary-btn px-3 py-2 text-sm"
                      >
                        <Download className="h-4 w-4" />
                        {t("下载 Release 部署", "Deploy Release")}
                      </button>
                      {workspaceUpdateStatus?.status === "running" ? (
                        <button
                          type="button"
                          onClick={() => void cancelWorkspaceUpdate()}
                          disabled={workspaceUpdateCanceling || systemRestarting}
                          className="theme-secondary-btn px-3 py-2 text-sm text-red-100 hover:border-red-400/35 hover:bg-red-500/10"
                        >
                          {workspaceUpdateCanceling ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <X className="h-4 w-4" />
                          )}
                          {workspaceUpdateCanceling
                            ? t("停止中", "Stopping")
                            : workspaceUpdateStatus.mode === "release_deploy"
                              ? t("停止部署", "Stop Deploy")
                              : t("停止编译", "Stop Build")}
                        </button>
                      ) : null}
                      <button
                        type="button"
                        onClick={() => {
                          const confirmed = window.confirm(
                            t(
                              "现在重启 RustClaw？重启期间页面会短暂断开，稍后会自动恢复。",
                              "Restart RustClaw now? The page may disconnect briefly and then recover.",
                            ),
                          );
                          if (confirmed) void restartSystem();
                        }}
                        disabled={workspaceUpdateLoading || workspaceUpdateStatus?.status === "running" || systemRestarting}
                        className="theme-secondary-btn px-3 py-2 text-sm"
                      >
                        {systemRestarting ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <RefreshCw className="h-4 w-4" />
                        )}
                        {systemRestarting ? t("重启中", "Restarting") : t("重启 RustClaw", "Restart RustClaw")}
                      </button>
                      {piAppStatus?.available ? (
                        <button
                          type="button"
                          onClick={() => {
                            const confirmed = window.confirm(
                              t(
                                "现在重启 Pi App 小程序？小屏界面会短暂关闭后重新打开。",
                                "Restart the Pi App now? The small-screen app will close briefly and reopen.",
                              ),
                            );
                            if (confirmed) void restartPiApp();
                          }}
                          disabled={piAppRestarting || systemRestarting}
                          className="theme-secondary-btn px-3 py-2 text-sm"
                          title={piAppStatus.model || undefined}
                        >
                          {piAppRestarting ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <RefreshCw className="h-4 w-4" />
                          )}
                          {piAppRestarting ? t("重启中", "Restarting") : t("重启 Pi App", "Restart Pi App")}
                        </button>
                      ) : null}
                    </div>
                  ) : (
                    <span className="rounded-full border border-white/10 bg-white/5 px-3 py-2 text-xs text-white/55">
                      {t("仅管理员可更新", "Admin only")}
                    </span>
                  )}
                </div>

                {workspaceUpdateMessage ? (
                  <p className="mt-4 rounded-xl border border-sky-400/25 bg-sky-400/10 px-3 py-2 text-sm text-sky-100">
                    {workspaceUpdateMessage}
                  </p>
                ) : null}
                {systemRestartMessage ? (
                  <p className="mt-3 rounded-xl border border-emerald-400/25 bg-emerald-400/10 px-3 py-2 text-sm text-emerald-100">
                    {systemRestartMessage}
                  </p>
                ) : null}
                {piAppRestartMessage ? (
                  <p className="mt-3 rounded-xl border border-emerald-400/25 bg-emerald-400/10 px-3 py-2 text-sm text-emerald-100">
                    {piAppRestartMessage}
                  </p>
                ) : null}

                <div className="mt-4 rounded-xl border border-white/8 bg-black/20 px-3 py-3">
                  <div className="flex items-center justify-between gap-3">
                    <p className="text-sm font-medium text-white/85">{t("编译进度", "Build Progress")}</p>
                    <span className="font-mono text-xs text-white/55">{workspaceUpdateProgressPercent}%</span>
                  </div>
                  <div className="mt-3 h-2 overflow-hidden rounded-full bg-white/10">
                    <div
                      className={`workspace-build-progress-bar h-full rounded-full transition-all duration-500 ${
                        workspaceUpdateProgressActive ? "workspace-build-progress-bar-active" : ""
                      } ${
                        workspaceUpdateDisplayStatus === "failed"
                          ? "bg-red-300"
                          : workspaceUpdateDisplayStatus === "canceled"
                            ? "bg-amber-300"
                            : workspaceUpdateDisplayStatus === "up_to_date" || workspaceUpdateRestarting
                              ? "bg-emerald-300"
                              : "bg-sky-300"
                      }`}
                      style={{ width: `${workspaceUpdateProgressPercent}%` }}
                    />
                  </div>
                  <p className="mt-2 text-xs leading-5 text-white/50">{workspaceUpdateProgressLabel}</p>
                </div>

                <div className="mt-4 grid gap-3 md:grid-cols-4">
                  <div className="rounded-xl border border-white/8 bg-black/20 px-3 py-3">
                    <p className="text-[11px] tracking-[0.14em] text-white/45">{t("状态", "Status")}</p>
                    <p
                      className={`mt-2 text-sm font-semibold ${
                        workspaceUpdateDisplayStatus === "failed"
                          ? "text-red-200"
                          : workspaceUpdateDisplayStatus === "up_to_date"
                            ? "text-emerald-200"
                            : workspaceUpdateRunning
                            ? "text-sky-200"
                            : "text-white/90"
                      }`}
                    >
                      {workspaceUpdateStatusLabel(workspaceUpdateDisplayStatus)}
                    </p>
                  </div>
                  <div className="rounded-xl border border-white/8 bg-black/20 px-3 py-3">
                    <p className="text-[11px] tracking-[0.14em] text-white/45">{t("当前步骤", "Current step")}</p>
                    <p className="mt-2 text-sm font-semibold text-white/90">
                      {workspaceUpdateStepLabel(workspaceUpdateStatus?.step)}
                    </p>
                  </div>
                  <div className="rounded-xl border border-white/8 bg-black/20 px-3 py-3">
                    <p className="text-[11px] tracking-[0.14em] text-white/45">{t("本地版本", "Local version")}</p>
                    <p className="mt-2 text-sm font-semibold text-white/90">
                      {workspaceUpdateStatus?.old_commit || "--"}
                      {workspaceUpdateStatus?.new_commit && workspaceUpdateStatus.new_commit !== workspaceUpdateStatus.old_commit
                        ? ` → ${workspaceUpdateStatus.new_commit}`
                        : ""}
                    </p>
                    <p className="mt-1 text-xs text-white/50">
                      {t("远端最新", "Remote latest")}: {workspaceUpdateStatus?.remote_commit || "--"}
                    </p>
                  </div>
                  <div className="rounded-xl border border-white/8 bg-black/20 px-3 py-3">
                    <p className="text-[11px] tracking-[0.14em] text-white/45">{t("开始时间", "Started")}</p>
                    <p className="mt-2 text-sm font-semibold text-white/90">
                      {workspaceUpdateTimeLabel(workspaceUpdateStatus?.started_ts)}
                    </p>
                  </div>
                </div>

                {workspaceUpdateNotice ? (
                  <div
                    className={`mt-4 rounded-xl border px-3 py-3 text-sm ${
                      workspaceUpdateNotice.tone === "error"
                        ? "border-red-500/30 bg-red-500/10 text-red-100"
                        : workspaceUpdateNotice.tone === "success"
                          ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-100"
                          : "border-sky-400/25 bg-sky-400/10 text-sky-100"
                    }`}
                  >
                    <p className="font-semibold">{workspaceUpdateNotice.title}</p>
                    <p className="mt-1 opacity-80">{workspaceUpdateNotice.detail}</p>
                  </div>
                ) : null}

                {workspaceUpdateLogPreview ? (
                  <details className="mt-4 rounded-xl border border-white/10 bg-black/20 p-3">
                    <summary className="cursor-pointer text-sm font-medium text-white/75">
                      {workspaceUpdateRunning
                        ? t("查看实时编译日志", "View live build logs")
                        : t("查看最近日志摘要", "View recent log summary")}
                    </summary>
                    <pre className="mt-3 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-black/30 p-3 text-xs leading-5 text-white/65">
                      {workspaceUpdateLogPreview}
                    </pre>
                  </details>
                ) : null}
              </section>

              {dashboardCommunicationRows.length > 0 ? (
                <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div>
                      <h3 className="text-base font-semibold">{t("已启动的通信端", "Running communication services")}</h3>
                      <p className="mt-2 text-sm text-white/65">
                        {t(
                          "首页只显示当前已经启动的通信端，并展示它们的运行状态、进程数量和内存占用。",
                          "Home only shows communication services that are currently running, together with their runtime status, process count, and memory usage.",
                        )}
                      </p>
                    </div>
                    <button type="button" onClick={() => setCurrentPage("services")} className="theme-topbar-btn px-3 py-2 text-sm">
                      {t("去通信接入", "Open Communication Setup")}
                    </button>
                  </div>

                  <div className="mt-4 grid gap-3 xl:grid-cols-2">
                    {dashboardCommunicationRows.map((row) => (
                      <div key={row.key} className="rounded-2xl border border-white/10 bg-black/20 p-4">
                        <div className="flex items-start justify-between gap-3">
                          <div>
                            <p className="text-sm font-semibold text-white">{row.label}</p>
                            <p className="mt-1 text-xs text-white/55">{row.statusLabel}</p>
                          </div>
                          <span
                            className={
                              row.category === "ready"
                                ? "setup-status setup-status-done"
                                : row.category === "attention"
                                  ? "setup-status setup-status-attention"
                                  : row.category === "stopped"
                                    ? "setup-status setup-status-todo"
                                    : "setup-status"
                            }
                          >
                            {row.category === "ready"
                              ? t("运行中", "Running")
                              : row.category === "attention"
                                ? t("待处理", "Needs attention")
                                : row.category === "stopped"
                                  ? t("未运行", "Stopped")
                                  : t("未知", "Unknown")}
                          </span>
                        </div>

                        <p className="mt-3 text-sm leading-6 text-white/68">{row.detail}</p>

                        <div className="mt-4 grid gap-3 sm:grid-cols-2">
                          <div className="rounded-xl border border-white/8 bg-white/5 px-3 py-3">
                            <p className="text-[11px] tracking-[0.14em] text-white/45">{t("内存占用", "Memory usage")}</p>
                            <p className="mt-2 text-sm font-semibold text-white/92">{row.memoryLabel}</p>
                            <p className="mt-1 text-xs text-white/50">
                              {row.usesSharedGatewayMemory
                                ? t("当前显示的是共享 channel-gateway 内存。", "Currently showing shared channel-gateway memory.")
                                : t("当前显示的是该通信端进程内存。", "Currently showing memory for this service process.")}
                            </p>
                          </div>
                          <div className="rounded-xl border border-white/8 bg-white/5 px-3 py-3">
                            <p className="text-[11px] tracking-[0.14em] text-white/45">{t("进程数量", "Process count")}</p>
                            <p className="mt-2 text-sm font-semibold text-white/92">{row.processCount ?? "--"}</p>
                            <p className="mt-1 text-xs text-white/50">{row.statusLabel}</p>
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                </section>
              ) : null}

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

            </>
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
            <div className="space-y-4">
              <section className="theme-panel p-5 sm:p-6">
                <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
                  <div className="max-w-3xl">
                    <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">Network Native Intelligence</p>
                    <h3 className="mt-2 flex items-center gap-2 text-xl font-semibold tracking-tight sm:text-2xl">
                      <Network className="h-6 w-6 theme-icon-accent" />
                      <span>{t("NNI 网络原生智能", "NNI Network-Native Intelligence")}</span>
                    </h3>
                    <p className="mt-3 text-sm leading-7 text-white/70">
                      {t(
                        "这里管理 Pi App 里的 NNI 入口和设备签名能力。普通设备可以只查看状态；带安全芯片的设备可以读取公钥、生成时间戳签名，并查看 TNG 证书链。加入时，本机公钥必须是白名单合规公钥。",
                        "This page manages the NNI entry from the Pi App and device signing. Regular devices can simply check status; devices with a secure chip can read the public key, create timestamp signatures, and inspect the TNG certificate chain. To join, the local public key must be compliant with the whitelist.",
                      )}
                    </p>
                  </div>

                  <div className="flex flex-wrap gap-2">
                    <button
                      type="button"
                      onClick={() => void fetchNniDeviceStatus()}
                      disabled={nniStatusLoading}
                      className="theme-secondary-btn px-3 py-2 text-sm"
                    >
                      {nniStatusLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                      {t("刷新状态", "Refresh status")}
                    </button>
                    <button
                      type="button"
                      onClick={() => (nniJoined ? void setNniJoinedPersisted(false) : void joinNni())}
                      disabled={Boolean(nniActionLoading) || nniStatusLoading || nniChipMissing || (!nniJoined && nniRemoteNodeCount === 0)}
                      className={nniJoined ? "theme-secondary-btn px-3 py-2 text-sm" : "theme-accent-btn px-3 py-2 text-sm"}
                      title={
                        nniChipMissing
                          ? t("当前设备缺少签名芯片，不能加入需要设备签名的 NNI。", "This device has no signature chip, so it cannot join signed NNI.")
                          : nniRemoteNodeCount === 0
                            ? t("请先填写远程 NNI 节点地址。", "Enter a remote NNI node URL first.")
                          : undefined
                      }
                    >
                      {["join_nni", "sign_challenge"].includes(nniActionLoading || "") ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <KeyRound className="h-4 w-4" />
                      )}
                      {nniJoined ? t("停止", "Stop") : t("加入", "Join")}
                    </button>
                    {!nniJoined ? (
                      <button
                        type="button"
                        onClick={() => void testJoinNni()}
                        disabled={Boolean(nniActionLoading) || nniStatusLoading || nniChipMissing}
                        className="theme-secondary-btn px-3 py-2 text-sm"
                        title={t(
                          "测试加入只做本机时间戳签名，不请求远程 NNI 服务端。",
                          "Test join only signs a local timestamp and does not contact the remote NNI server.",
                        )}
                      >
                        {nniActionLoading === "sign_timestamp" ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <KeyRound className="h-4 w-4" />
                        )}
                        {t("测试加入", "Test Join")}
                      </button>
                    ) : null}
                  </div>
                </div>
              </section>

              {nniStatusError ? (
                <p className="rounded-2xl border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-100">
                  {nniStatusError}
                </p>
              ) : null}

              <section className="grid gap-4 xl:grid-cols-[0.95fr_1.05fr]">
                <div className="theme-panel-soft p-5">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">{t("设备状态", "Device status")}</p>
                      <h4 className="mt-2 text-lg font-semibold">{t("设备签名芯片", "Device signature chip")}</h4>
                    </div>
                    <span
                      className={
                        nniStatusLoading
                          ? "setup-status"
                          : nniStatus == null
                            ? "setup-status setup-status-todo"
                            : nniChipPresent
                            ? "setup-status setup-status-done"
                            : "setup-status setup-status-attention"
                      }
                    >
                      {nniStatusLoading ? (
                        <>
                          <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          {t("检测中", "Checking")}
                        </>
                      ) : nniChipPresent ? (
                        <>
                          <ShieldCheck className="h-3.5 w-3.5" />
                          {t("可用", "Ready")}
                        </>
                      ) : nniStatus == null ? (
                        t("未检测", "Not checked")
                      ) : (
                        <>
                          <ShieldAlert className="h-3.5 w-3.5" />
                          {t("缺失签名芯片", "Signature chip missing")}
                        </>
                      )}
                    </span>
                  </div>

                  <div
                    className={
                      nniChipPresent
                        ? "mt-4 rounded-xl border border-emerald-500/25 bg-emerald-500/10 px-3 py-3 text-sm text-emerald-100"
                        : "mt-4 rounded-xl border border-amber-500/30 bg-amber-500/10 px-3 py-3 text-sm text-amber-100"
                    }
                  >
                    <p className="font-medium">
                      {nniStatus?.message ||
                        (nniStatusLoading
                          ? t("正在读取签名芯片状态。", "Reading signature chip status.")
                          : t("还没有读取状态。点击刷新状态开始检测。", "Status has not been loaded yet. Click Refresh status to check."))}
                    </p>
                    {nniStatus?.next_step ? <p className="mt-1 text-sm opacity-80">{nniStatus.next_step}</p> : null}
                    {nniStatus?.error ? <p className="mt-2 break-words font-mono text-xs opacity-75">{nniStatus.error}</p> : null}
                  </div>

                  <div className="mt-4 grid gap-3 sm:grid-cols-2">
                    <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-3">
                      <p className="text-[11px] tracking-[0.14em] text-white/45">slot</p>
                      <p className="mt-2 text-sm font-semibold text-white/90">{nniStatus?.meta?.slot ?? "--"}</p>
                    </div>
                    <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-3">
                      <p className="text-[11px] tracking-[0.14em] text-white/45">I2C</p>
                      <p className="mt-2 text-sm font-semibold text-white/90">
                        {nniStatus?.meta?.i2c_address || "--"}
                        {nniStatus?.meta?.i2c_bus != null ? ` / bus ${nniStatus?.meta?.i2c_bus}` : ""}
                      </p>
                    </div>
                    <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-3 sm:col-span-2">
                      <p className="text-[11px] tracking-[0.14em] text-white/45">{t("公钥指纹", "Public key fingerprint")}</p>
                      <p className="mt-2 break-all font-mono text-sm font-semibold text-white/90">
                        {nniStatus?.pubkey_fingerprint || nniStatus?.pubkey_preview || "--"}
                      </p>
                    </div>
                  </div>
                </div>

                <div className="theme-panel-soft p-5">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">{t("加入状态", "Join state")}</p>
                      <h4 className="mt-2 text-lg font-semibold">{t("NNI 运行入口", "NNI runtime entry")}</h4>
                    </div>
                    <span className={nniJoined ? "setup-status setup-status-done" : "setup-status setup-status-todo"}>
                      {nniJoined ? t("心跳挑战中", "Heartbeat active") : t("未加入", "Not joined")}
                    </span>
                  </div>

                  <div className="mt-4 rounded-2xl border border-white/10 bg-black/20 p-3">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <label className="text-[11px] font-semibold tracking-[0.16em] text-white/55">
                        {t("远程 NNI 节点", "Remote NNI nodes")}
                      </label>
                      <div className="flex flex-wrap items-center gap-2">
                        <button
                          type="button"
                          onClick={() => void fetchNniConfig()}
                          disabled={nniConfigLoading || nniConfigSaving}
                          className="theme-secondary-btn px-3 py-1.5 text-xs"
                        >
                          {nniConfigLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                          {t("重新载入", "Reload")}
                        </button>
                        <button
                          type="button"
                          onClick={() => void saveNniConfig()}
                          disabled={nniConfigLoading || nniConfigSaving}
                          className="theme-accent-btn px-3 py-1.5 text-xs"
                        >
                          {nniConfigSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                          {t("保存节点", "Save nodes")}
                        </button>
                      </div>
                    </div>
                    <textarea
                      className="theme-input mt-2 min-h-20 resize-y font-mono text-xs"
                      placeholder={t(
                        "例如：https://nni-node.example.com\n多个节点可以一行一个，系统会按顺序尝试。",
                        "Example: https://nni-node.example.com\nUse one node per line. The system will try them in order.",
                      )}
                      value={nniRemoteNodes}
                      onChange={(event) => {
                        setNniRemoteNodes(event.target.value);
                        setNniConfigMessage(null);
                        setNniConfigError(null);
                      }}
                    />
                    <p className="mt-2 text-xs leading-5 text-white/50">
                      {t(
                        "远程节点会保存到 configs/config.toml；加入成功后运行状态也会保存，clawd 重启或页面重开后会自动载入。远程节点负责下发 challenge、验签并记录合规请求。本机公钥必须是白名单合规公钥。",
                        "Remote nodes are saved to configs/config.toml. After Join succeeds, the runtime state is saved too and loads automatically after clawd restarts or the page reopens. Remote nodes issue challenges, verify signatures, and record compliant requests. The local public key must be compliant with the whitelist.",
                      )}
                    </p>
                    {nniConfigMessage ? <p className="mt-2 text-xs text-emerald-200">{nniConfigMessage}</p> : null}
                    {nniConfigError ? <p className="mt-2 break-words text-xs text-red-200">{nniConfigError}</p> : null}
                  </div>

                  <div
                    className={`nni-runtime-board mt-4 min-h-[180px] rounded-2xl border p-4 ${
                      nniJoined ? "nni-runtime-board-active" : "nni-runtime-board-idle"
                    }`}
                  >
                    <div className="grid h-full min-h-[148px] grid-cols-6 gap-2 sm:grid-cols-8">
                      {NNI_RUNTIME_TILES.map((tile, index) => (
                        <div
                          key={index}
                          className={`nni-runtime-tile rounded-lg border ${
                            nniJoined ? "nni-runtime-tile-active" : "nni-runtime-tile-idle"
                          }`}
                          style={{
                            animationDelay: `${tile.delay}s`,
                            animationDuration: `${tile.duration}s`,
                            opacity: nniJoined ? undefined : tile.idleOpacity,
                          }}
                        />
                      ))}
                    </div>
                  </div>

                  <div className="mt-4 grid gap-3 border-t border-white/10 pt-4 sm:grid-cols-3">
                    <div>
                      <p className="text-[11px] font-semibold tracking-[0.16em] text-white/45">
                        {t("心跳请求次数", "Heartbeat requests")}
                      </p>
                      <p className="mt-1 text-xl font-semibold text-white/90">{nniHeartbeatRequestCount}</p>
                    </div>
                    <div>
                      <p className="text-[11px] font-semibold tracking-[0.16em] text-white/45">
                        {t("最近请求", "Latest request")}
                      </p>
                      <p className="mt-1 text-sm font-medium text-white/75">{formatUnixDateTime(nniLastHeartbeatAtTs)}</p>
                    </div>
                    <div>
                      <p className="text-[11px] font-semibold tracking-[0.16em] text-white/45">
                        {t("最近网络重试", "Latest network retries")}
                      </p>
                      <p className="mt-1 text-sm font-medium text-white/75">
                        {nniLastHeartbeatNetworkFailures > 0
                          ? `${nniLastHeartbeatNetworkFailures} / ${nniHeartbeatRetryLimit}`
                          : `0 / ${nniHeartbeatRetryLimit}`}
                      </p>
                    </div>
                  </div>

                  <p className="mt-4 text-sm leading-7 text-white/65">
                    {nniChipMissing
                      ? t(
                          "当前设备缺少签名芯片，因此不会显示为已加入。你仍可以继续使用 RustClaw 的其它功能。",
                          "This device has no signature chip, so it will not be marked as joined. Other RustClaw features remain available.",
                        )
                      : nniJoined
                        ? t(
                            "服务端已验证设备签名，NNI 运行入口已开启。clawd 会每 15 分钟向服务器发送一次硬件签名心跳。",
                            "The server verified the device signature, and the NNI runtime entry is active. clawd will send a hardware-signed heartbeat to the server every 15 minutes.",
                          )
                        : t(
                            "点击加入会向远程服务端请求一次随机挑战。本机公钥必须是白名单合规公钥，验签通过后开启运行入口；测试加入只做本机时间戳签名，不请求远程服务端。",
                            "Click Join to request a random challenge from the remote server. The local public key must be compliant with the whitelist, and the runtime is enabled after verification. Test Join only signs a local timestamp and does not contact the remote server.",
                          )}
                  </p>
                </div>
              </section>

              <section className="theme-panel-soft p-5">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">
                      {t("NNI 心跳错误", "NNI heartbeat errors")}
                    </p>
                    <h4 className="mt-2 text-lg font-semibold">{t("本机心跳错误记录", "Local heartbeat error history")}</h4>
                    <p className="mt-2 text-sm leading-6 text-white/60">
                      {t(
                        `共 ${nniHeartbeatErrorsTotal} 条错误记录，每页 ${NNI_HEARTBEAT_ERRORS_PAGE_SIZE} 条。`,
                        `${nniHeartbeatErrorsTotal} error records total, ${NNI_HEARTBEAT_ERRORS_PAGE_SIZE} per page.`,
                      )}
                    </p>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <button
                      type="button"
                      onClick={() => void fetchNniHeartbeatErrors(nniHeartbeatErrorsPage)}
                      disabled={nniHeartbeatErrorsLoading || nniHeartbeatErrorsClearing}
                      className="theme-secondary-btn px-3 py-2 text-xs"
                    >
                      {nniHeartbeatErrorsLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                      {t("刷新错误", "Refresh errors")}
                    </button>
                    <button
                      type="button"
                      onClick={() => void clearNniHeartbeatErrors()}
                      disabled={nniHeartbeatErrorsLoading || nniHeartbeatErrorsClearing || nniHeartbeatErrorsTotal === 0}
                      className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                      title={t(
                        "只清理本机保存的心跳错误历史，不会修改远程服务端请求记录。",
                        "Only clears local heartbeat error history. Remote server request records are not changed.",
                      )}
                    >
                      {nniHeartbeatErrorsClearing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}
                      {t("清理错误", "Clear errors")}
                    </button>
                  </div>
                </div>

                {nniHeartbeatErrorsError ? (
                  <p className="mt-3 break-words rounded-xl border border-amber-300/20 bg-amber-300/10 px-3 py-2 text-xs leading-5 text-amber-100">
                    {t("NNI 心跳错误暂时无法载入：", "NNI heartbeat errors could not be loaded: ")}
                    {nniHeartbeatErrorsError}
                  </p>
                ) : null}
                {nniHeartbeatErrorsMessage ? (
                  <p className="mt-3 rounded-xl border border-emerald-500/25 bg-emerald-500/10 px-3 py-2 text-xs leading-5 text-emerald-100">
                    {nniHeartbeatErrorsMessage}
                  </p>
                ) : null}

                <div className="mt-4 overflow-hidden rounded-2xl border border-white/10 bg-black/20">
                  {nniHeartbeatErrors.length === 0 ? (
                    <p className="px-4 py-5 text-sm text-white/55">
                      {nniHeartbeatErrorsLoading
                        ? t("正在载入 NNI 心跳错误...", "Loading NNI heartbeat errors...")
                        : t("当前没有本机心跳错误记录。后续自动心跳失败时会出现在这里。", "There are no local heartbeat error records. Future automatic heartbeat failures will appear here.")}
                    </p>
                  ) : (
                    nniHeartbeatErrors.map((record) => (
                      <div key={`${record.id}-${record.created_at_ts ?? 0}`} className="border-t border-white/10 px-4 py-3 first:border-t-0">
                        <div className="flex flex-wrap items-center justify-between gap-2">
                          <div className="flex flex-wrap items-center gap-2">
                            <span className="setup-status setup-status-attention">{t("心跳失败", "Heartbeat failed")}</span>
                            <span className="rounded-full border border-white/10 bg-white/[0.04] px-2 py-0.5 text-[11px] font-semibold text-white/55">
                              {record.network ? t("网络错误", "Network") : t("服务端返回", "Server response")}
                            </span>
                            <span className="font-mono text-xs text-white/45">#{record.id}</span>
                          </div>
                          <span className="text-xs text-white/50">{formatUnixDateTime(record.created_at_ts)}</span>
                        </div>
                        <p className="mt-3 break-words rounded-xl border border-white/10 bg-black/25 px-3 py-2 font-mono text-xs leading-5 text-white/75">
                          {record.error}
                        </p>
                      </div>
                    ))
                  )}
                </div>

                <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
                  <p className="text-xs text-white/50">
                    {t(
                      `第 ${nniHeartbeatErrorsPage} / ${nniHeartbeatErrorsTotalPages} 页`,
                      `Page ${nniHeartbeatErrorsPage} of ${nniHeartbeatErrorsTotalPages}`,
                    )}
                  </p>
                  <div className="flex items-center gap-2">
                    <button
                      type="button"
                      onClick={() => void fetchNniHeartbeatErrors(nniHeartbeatErrorsPage - 1)}
                      disabled={!nniHeartbeatErrorsCanPrev || nniHeartbeatErrorsLoading}
                      className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      <ChevronLeft className="h-3.5 w-3.5" />
                      {t("上一页", "Previous")}
                    </button>
                    <button
                      type="button"
                      onClick={() => void fetchNniHeartbeatErrors(nniHeartbeatErrorsPage + 1)}
                      disabled={!nniHeartbeatErrorsCanNext || nniHeartbeatErrorsLoading}
                      className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {t("下一页", "Next")}
                      <ChevronRight className="h-3.5 w-3.5" />
                    </button>
                  </div>
                </div>
              </section>

              <section className="theme-panel-soft p-5">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">
                      {t("NNI 请求记录", "NNI request records")}
                    </p>
                    <h4 className="mt-2 text-lg font-semibold">{t("本机请求记录", "Local request records")}</h4>
                    <p className="mt-2 text-sm leading-6 text-white/60">
                      {t(
                        `共 ${nniHeartbeatRecordsTotal} 条记录，每页 ${NNI_HEARTBEAT_RECORDS_PAGE_SIZE} 条。`,
                        `${nniHeartbeatRecordsTotal} records total, ${NNI_HEARTBEAT_RECORDS_PAGE_SIZE} per page.`,
                      )}
                    </p>
                    <p className="mt-1 text-xs leading-5 text-white/45">
                      {t(
                        "这里保存本机看到的手动加入和自动心跳结果；远端服务端记录不再从 UI 拉取。",
                        "This stores manual Join and automatic Heartbeat results seen by this device. Remote server records are no longer fetched in the UI.",
                      )}
                    </p>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <button
                      type="button"
                      onClick={() => void clearNniHeartbeatRecords()}
                      disabled={nniHeartbeatRecordsTotal === 0 || nniHeartbeatRecordsLoading || nniHeartbeatRecordsClearing}
                      className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {nniHeartbeatRecordsClearing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}
                      {t("清理记录", "Clear records")}
                    </button>
                    <button
                      type="button"
                      onClick={() => void fetchNniHeartbeatRecords(nniHeartbeatRecordsPage)}
                      disabled={nniHeartbeatRecordsLoading}
                      className="theme-secondary-btn px-3 py-2 text-xs"
                    >
                      {nniHeartbeatRecordsLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                      {t("刷新记录", "Refresh records")}
                    </button>
                  </div>
                </div>

                {nniHeartbeatRecordsError ? (
                  <p className="mt-3 break-words rounded-xl border border-amber-300/20 bg-amber-300/10 px-3 py-2 text-xs leading-5 text-amber-100">
                    {t("NNI 请求记录暂时无法载入：", "NNI request records could not be loaded: ")}
                    {nniHeartbeatRecordsError}
                  </p>
                ) : null}
                {nniHeartbeatRecordsMessage ? (
                  <p className="mt-3 rounded-xl border border-emerald-300/20 bg-emerald-300/10 px-3 py-2 text-xs leading-5 text-emerald-100">
                    {nniHeartbeatRecordsMessage}
                  </p>
                ) : null}

                <div className="mt-4 overflow-hidden rounded-2xl border border-white/10 bg-black/20">
                  {nniHeartbeatRecords.length === 0 ? (
                    <p className="px-4 py-5 text-sm text-white/55">
                      {nniHeartbeatRecordsLoading
                        ? t("正在载入 NNI 请求记录...", "Loading NNI request records...")
                        : t(
                            "本机还没有 NNI 请求记录。手动加入和自动心跳的成功或失败结果都会保存在这里。",
                            "This device has no NNI request records yet. Manual Join and automatic Heartbeat successes or failures will be saved here.",
                          )}
                    </p>
                  ) : (
                    nniHeartbeatRecords.map((record) => {
                      const complianceKnown = typeof record.compliant === "boolean";
                      const accepted = record.status === "accepted" && record.compliant !== false;
                      const attention = ["blocked", "rejected", "expired", "failed"].includes(record.status) || record.compliant === false;
                      const statusClass = accepted
                        ? "setup-status setup-status-done"
                        : attention
                          ? "setup-status setup-status-attention"
                          : "setup-status setup-status-todo";
                      const statusLabel =
                        record.status === "accepted"
                          ? t("已通过", "Accepted")
                          : record.status === "blocked"
                            ? t("已拦截", "Blocked")
                            : record.status === "rejected"
                              ? t("已拒绝", "Rejected")
                              : record.status === "expired"
                                ? t("已过期", "Expired")
                                : record.status === "challenge_created"
                                  ? t("挑战已创建", "Challenge created")
                                  : record.status === "failed"
                                    ? t("失败", "Failed")
                                    : record.status || t("未知", "Unknown");
                      const kindLabel =
                        record.request_kind === "nni_join"
                          ? t("加入", "Join")
                          : record.request_kind === "nni_heartbeat"
                            ? t("心跳", "Heartbeat")
                            : record.request_kind || t("请求", "Request");
                      const resultLabel =
                        record.error_code ||
                        (record.compliant === true
                          ? t("合规", "Compliant")
                          : record.compliant === false
                            ? t("未合规", "Not compliant")
                            : record.status === "challenge_created"
                              ? t("等待签名验证", "Waiting for signature verification")
                              : record.status === "failed"
                                ? t("失败", "Failed")
                            : record.status === "accepted"
                              ? t("已通过", "Accepted")
                              : t("未返回", "Not reported"));
                      return (
                        <div
                          key={`${record.id ?? record.task_id ?? "heartbeat"}-${record.created_at_ts ?? 0}`}
                          className="border-t border-white/10 px-4 py-3 first:border-t-0"
                        >
                          <div className="flex flex-wrap items-center justify-between gap-2">
                            <div className="flex flex-wrap items-center gap-2">
                              <span className={statusClass}>{statusLabel}</span>
                              <span className="rounded-full border border-white/10 bg-white/[0.04] px-2 py-0.5 text-[11px] font-semibold text-white/55">
                                {kindLabel}
                              </span>
                              <span className="font-mono text-xs text-white/45">#{record.id ?? "--"}</span>
                            </div>
                            <span className="text-xs text-white/50">{formatUnixDateTime(record.created_at_ts)}</span>
                          </div>
                          <div className="mt-3 grid gap-3 text-xs sm:grid-cols-3">
                            <div>
                              <p className="font-semibold tracking-[0.12em] text-white/35">{t("公钥", "Public key")}</p>
                              <p className="mt-1 break-all font-mono text-white/75" title={record.device_pubkey || ""}>
                                {shortNniValue(record.device_pubkey)}
                              </p>
                            </div>
                            <div>
                              <p className="font-semibold tracking-[0.12em] text-white/35">{t("任务", "Task")}</p>
                              <p className="mt-1 break-all font-mono text-white/75" title={record.task_id || ""}>
                                {shortNniValue(record.task_id)}
                              </p>
                            </div>
                            <div>
                              <p className="font-semibold tracking-[0.12em] text-white/35">{t("结果", "Result")}</p>
                              <p className="mt-1 break-words text-white/75">
                                {resultLabel}
                              </p>
                            </div>
                          </div>
                          {!complianceKnown && record.status !== "accepted" && !record.error_code ? (
                            <p className="mt-2 text-xs leading-5 text-white/40">
                              {t(
                                "这条记录没有合规结果；请以状态标签和错误码为准。",
                                "This record has no compliance result; use the status label and error code.",
                              )}
                            </p>
                          ) : null}
                          <p className="mt-2 text-xs leading-5 text-white/40">
                            {t("签名", "Signature")}: {record.signature_present ? t("已记录", "Recorded") : t("无", "None")} ·{" "}
                            {t("挑战", "Challenge")}: {record.challenge_present ? t("已记录", "Recorded") : t("无", "None")} ·{" "}
                            {t("节点", "Node")}: <span className="font-mono">{shortNniValue(record.node_url)}</span> ·{" "}
                            {t("用户", "User")}: <span className="font-mono">{shortNniValue(record.user_key)}</span>
                          </p>
                        </div>
                      );
                    })
                  )}
                </div>

                <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
                  <p className="text-xs text-white/50">
                    {t(
                      `第 ${nniHeartbeatRecordsPage} / ${nniHeartbeatRecordsTotalPages} 页`,
                      `Page ${nniHeartbeatRecordsPage} of ${nniHeartbeatRecordsTotalPages}`,
                    )}
                  </p>
                  <div className="flex items-center gap-2">
                    <button
                      type="button"
                      onClick={() => void fetchNniHeartbeatRecords(nniHeartbeatRecordsPage - 1)}
                      disabled={!nniHeartbeatRecordsCanPrev || nniHeartbeatRecordsLoading}
                      className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      <ChevronLeft className="h-3.5 w-3.5" />
                      {t("上一页", "Previous")}
                    </button>
                    <button
                      type="button"
                      onClick={() => void fetchNniHeartbeatRecords(nniHeartbeatRecordsPage + 1)}
                      disabled={!nniHeartbeatRecordsCanNext || nniHeartbeatRecordsLoading}
                      className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {t("下一页", "Next")}
                      <ChevronRight className="h-3.5 w-3.5" />
                    </button>
                  </div>
                </div>
              </section>

              <section className="grid gap-4 xl:grid-cols-[0.9fr_1.1fr]">
                <div className="theme-panel-soft p-5">
                  <div className="flex items-start gap-3">
                    <Fingerprint className="mt-0.5 h-5 w-5 shrink-0 theme-icon-soft" />
                    <div>
                      <h4 className="text-lg font-semibold">{t("设备签名操作", "Device signing actions")}</h4>
                      <p className="mt-2 text-sm leading-7 text-white/65">
                        {t(
                          "这些操作对应 Pi App 已预埋的 helper：slot 0 公钥、时间戳签名、TNG 设备公钥和证书链。",
                          "These actions map to the helper already built into the Pi App: Slot 0 public key, timestamp signing, TNG device public key, and certificate chain.",
                        )}
                      </p>
                    </div>
                  </div>

                  <div className="mt-4 grid gap-2">
                    {["pubkey", "sign_timestamp", "tng_device_pubkey", "tng_device_cert", "tng_signer_cert", "tng_root_cert"].map((action) => (
                      <button
                        key={action}
                        type="button"
                        onClick={() => void runNniDeviceAction(action)}
                        disabled={Boolean(nniActionLoading) || nniStatusLoading || nniChipMissing}
                        className="theme-topbar-btn justify-between px-3 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-50"
                        title={
                          nniChipMissing
                            ? t("当前设备缺少签名芯片，不能执行该操作。", "This device has no signature chip, so this action cannot run.")
                            : undefined
                        }
                      >
                        <span className="inline-flex items-center gap-2">
                          {nniActionLoading === action ? <Loader2 className="h-4 w-4 animate-spin" /> : <Cpu className="h-4 w-4" />}
                          {nniActionLabel(action)}
                        </span>
                        <span className="font-mono text-xs text-white/45">{action}</span>
                      </button>
                    ))}
                  </div>
                </div>

                <div className="theme-panel-soft p-5">
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div>
                      <h4 className="text-lg font-semibold">{t("最近一次结果", "Latest result")}</h4>
                      <p className="mt-2 text-sm text-white/60">
                        {nniActionResult
                          ? nniActionLabel(nniActionResult.action)
                          : t("执行一个设备签名操作后，这里会显示返回值。", "Run a device signing action to show its result here.")}
                      </p>
                    </div>
                    {nniPrimaryHex ? (
                      <button
                        type="button"
                        onClick={() => {
                          void writeTextToClipboard(nniPrimaryHex.value)
                            .then(() => setNniActionMessage(t("已复制结果。", "Result copied.")))
                            .catch((err) => setNniActionError(err instanceof Error ? err.message : "复制失败"));
                        }}
                        className="theme-secondary-btn px-3 py-2 text-xs"
                      >
                        <Copy className="h-4 w-4" />
                        {t("复制", "Copy")}
                      </button>
                    ) : null}
                  </div>

                  {nniActionMessage ? (
                    <p className="mt-4 rounded-xl border border-emerald-500/25 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-100">
                      {nniActionMessage}
                    </p>
                  ) : null}
                  {nniActionError ? (
                    <p className="mt-4 rounded-xl border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-100">
                      {nniActionError}
                    </p>
                  ) : null}

                  {nniPrimaryHex ? (
                    <div className="mt-4 rounded-xl border border-white/10 bg-black/20 p-3">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <p className="text-xs font-semibold text-white/75">{nniPrimaryHex.label}</p>
                        {nniPrimaryHex.size != null ? (
                          <span className="rounded-full border border-white/10 bg-white/5 px-2 py-1 text-[11px] text-white/55">
                            {nniPrimaryHex.size} bytes
                          </span>
                        ) : null}
                      </div>
                      <p className="mt-3 break-all font-mono text-xs leading-6 text-white/75">
                        {shortenHex(nniPrimaryHex.value, 48, 48)}
                      </p>
                    </div>
                  ) : null}

                  {nniActionResult?.payload?.timestamp ? (
                    <div className="mt-3 rounded-xl border border-white/10 bg-black/20 px-3 py-3">
                      <p className="text-[11px] tracking-[0.14em] text-white/45">{t("签名时间", "Signed timestamp")}</p>
                      <p className="mt-2 font-mono text-sm text-white/85">{nniActionResult.payload.timestamp}</p>
                    </div>
                  ) : null}

                  {nniActionResult ? (
                    <details className="mt-4 rounded-xl border border-white/10 bg-black/20 p-3">
                      <summary className="cursor-pointer text-sm font-medium text-white/75">
                        {t("查看原始 JSON", "View raw JSON")}
                      </summary>
                      <pre className="mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-black/30 p-3 text-xs leading-5 text-white/65">
                        {JSON.stringify(nniActionResult.payload ?? nniActionResult, null, 2)}
                      </pre>
                    </details>
                  ) : null}
                </div>
              </section>
            </div>
          ) : null}

          {currentPage === "services" ? (
            <div className="space-y-5">
              {serviceActionMessage ? (
                <p
                  className={
                    serviceActionMessage.tone === "error"
                      ? "rounded-2xl border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-100"
                      : "rounded-2xl border border-emerald-500/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-100"
                  }
                >
                  {serviceActionMessage.text}
                </p>
              ) : null}

              <section className="theme-panel-soft channel-setup-hero p-5">
                <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
                  <div className="max-w-2xl">
                    <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">{t("通信接入", "Communication setup")}</p>
                    <h3 className="mt-2 text-xl font-semibold tracking-tight">
                      {t("微信、Telegram 和飞书都可以在这里接入。", "WeChat, Telegram, and Feishu can all be connected here.")}
                    </h3>
                    <p className="mt-3 text-sm leading-7 text-white/70">
                      {t(
                        "按你要使用的通信方式完成配置即可。微信支持扫码登录，Telegram 支持 Bot Token 接入，飞书支持扫码打开机器人后发送绑定码完成绑定。",
                        "Configure only the communication method you plan to use. WeChat supports QR sign-in, Telegram uses a bot token, and Feishu lets you scan to open the bot and then send a bind code to finish binding.",
                      )}
                    </p>
                  </div>
                </div>

                <div className="mt-5 grid items-start gap-4 xl:grid-cols-3">
                  <div className="setup-channel-card channel-setup-card flex self-start flex-col">
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <h4 className="text-lg font-semibold text-white">{t("微信", "WeChat")}</h4>
                        <p className="mt-2 text-sm leading-7 text-white/65">
                          {t(
                            "可以直接在当前卡片里完成设置、启动服务和扫码登录。",
                            "Complete configuration, start the service, and sign in with a QR code directly in this card.",
                          )}
                        </p>
                      </div>
                      <span className={wechatStatusLoading ? "setup-status" : wechatStepStatus === "done" ? "setup-status setup-status-done" : wechatStepStatus === "attention" ? "setup-status setup-status-attention" : "setup-status setup-status-todo"}>
                        {wechatStatusLoading ? (
                          <>
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            {t("载入中", "Loading")}
                          </>
                        ) : wechatStepStatus === "done" ? t("已可用", "Ready") : wechatStepStatus === "attention" ? t("还差一步", "In progress") : t("还没开始", "Not started")}
                      </span>
                    </div>

                    <p className="mt-4 text-sm leading-7 text-white/65">{wechatStatusSummary}</p>

                    <div className="mt-4 flex flex-1 flex-col gap-4 border-t border-white/10 pt-4">
                      {wechatQrStarting || wechatLoginStatus?.qr_status === "generating" || (wechatQrPreviewRequested && wechatLoginStatus?.qrcode_url) ? (
                        <div className="wechat-login-visual space-y-3">
                          {wechatQrStarting || wechatLoginStatus?.qr_status === "generating" ? (
                            <div className="wechat-login-stage flex min-h-[20rem] items-center justify-center rounded-[24px] border border-dashed border-sky-500/25 bg-sky-500/6 p-5">
                              <div className="flex flex-col items-center gap-3 text-center">
                                <Loader2 className="h-8 w-8 animate-spin text-sky-200" />
                                <p className="text-sm font-medium text-sky-100">{t("正在生成二维码", "Generating QR code")}</p>
                                <p className="max-w-sm text-xs leading-6 text-sky-100/70">
                                  {t("生成完成后，这里会自动切换成可扫码的二维码。", "This panel will switch to a scannable QR code automatically once generation finishes.")}
                                </p>
                              </div>
                            </div>
                          ) : wechatQrPreviewRequested && wechatLoginStatus?.qrcode_url ? (
                            <div className="space-y-3">
                              <div className="inline-block rounded-[24px] border border-white/12 bg-white p-4 shadow-[0_24px_70px_rgba(6,10,18,0.22)]">
                                <img src={wechatLoginStatus.qrcode_url} alt="WeChat QR" className="wechat-login-qr-image h-72 w-72" />
                              </div>
                              <p className="text-xs text-white/52">
                                {t("二维码有效期较短，过期后点击“刷新二维码”即可。", "The QR code expires quickly. Click Refresh QR if it expires.")}
                              </p>
                            </div>
                          ) : null}
                        </div>
                      ) : null}

                      <div className="flex flex-1 flex-col gap-4">
                          {wechatLoginStatus?.connected ? (
                            <div className="rounded-xl border border-emerald-500/20 bg-emerald-500/8 px-3 py-2 text-sm text-emerald-100/85">
                              {t("当前登录状态可继续使用；如果要更换登录，也可以重新生成二维码。", "The current login is active. If you want to switch accounts, you can also regenerate the QR code.")}
                            </div>
                          ) : null}

                          {wechatLoginStatus?.last_error ? (
                            <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
                              {wechatLoginStatus.last_error}
                            </p>
                          ) : null}
                          {wechatLoginError ? (
                            <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                              {wechatLoginError}
                            </p>
                          ) : null}

                          <div className="mt-auto flex flex-wrap gap-2">
                            <button
                              type="button"
                              onClick={() => void controlService("wechatd", health?.wechatd_healthy === true ? "restart" : "start")}
                              disabled={Boolean(serviceActionLoading.wechatd) || !wechatConfigDraft?.enabled}
                              className="theme-secondary-btn px-4 py-2.5 text-sm"
                            >
                              {serviceActionLoading.wechatd ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                              {health?.wechatd_healthy === true ? t("重启微信服务", "Restart the WeChat service") : t("启动微信服务", "Start the WeChat service")}
                            </button>
                            <button
                              type="button"
                              onClick={() => void startWechatQrLogin(true)}
                              disabled={Boolean(serviceActionLoading.wechatd) || wechatQrStarting || health?.wechatd_healthy !== true}
                              className="theme-accent-btn px-4 py-2.5 text-sm"
                            >
                              {wechatQrStarting ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                              {wechatLoginStatus?.connected
                                ? t("重新生成二维码", "Regenerate QR")
                                : wechatQrPreviewRequested && wechatLoginStatus?.qrcode_url
                                  ? t("刷新二维码", "Refresh QR")
                                  : t("生成二维码", "Generate QR")}
                            </button>
                          </div>
                      </div>
                    </div>
                  </div>

                  <div className="setup-channel-card channel-setup-card flex self-start flex-col">
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <h4 className="text-lg font-semibold text-white">Telegram</h4>
                        <p className="mt-2 text-sm leading-7 text-white/65">
                          {t(
                            "如果你要用 Telegram，只需要填好 Bot Token，然后保存并启动服务。",
                            "If you plan to use Telegram, just enter the bot token, save it, and start the service.",
                          )}
                        </p>
                      </div>
                      <span className={telegramStatusLoading ? "setup-status" : telegramStepStatus === "done" ? "setup-status setup-status-done" : telegramStepStatus === "attention" ? "setup-status setup-status-attention" : "setup-status setup-status-todo"}>
                        {telegramStatusLoading ? (
                          <>
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            {t("载入中", "Loading")}
                          </>
                        ) : telegramStepStatus === "done" ? t("已可用", "Ready") : telegramStepStatus === "attention" ? t("还差一步", "In progress") : t("还没开始", "Not started")}
                      </span>
                    </div>

                    <p className="mt-4 text-sm leading-7 text-white/65">{telegramStatusSummary}</p>

                    <div className="channel-setup-form mt-4 grid gap-3">
                      <label className="block space-y-2">
                        <span className="text-xs uppercase tracking-widest text-white/50">{t("Bot Token", "Bot Token")}</span>
                        <input
                          className="theme-input"
                          value={primaryTelegramBot.bot_token}
                          onChange={(e) => setTelegramPrimaryBotDraftField("bot_token", e.target.value)}
                        />
                        <p className="text-xs text-white/45">
                          {t("这里只填 Bot Token 就够了。更复杂的设置以后再说。", "Only the Bot Token is needed here. More advanced settings can wait until later.")}
                        </p>
                        {primaryTelegramBot.bot_token_masked ? (
                          <p className="rounded-lg border border-white/10 bg-black/20 px-3 py-2 text-xs text-white/65">
                            {t("当前正在使用：", "Currently in use: ")}
                            <span className="ml-1 font-mono text-white/88">{primaryTelegramBot.bot_token_masked}</span>
                          </p>
                        ) : null}
                        <p className="text-xs text-white/35">
                          {telegramBotTokenConfigured
                            ? t("出于安全考虑，当前已保存的 Bot Token 不会回显到输入框。", "For safety, the currently saved bot token is not echoed back into the input.")
                            : t("这里不会回显已保存的 Token。需要更新时，直接输入新的 Bot Token 即可。", "Saved tokens are not echoed here. To update it, just enter a new bot token.")}
                        </p>
                      </label>
                    </div>

                    {telegramConfigError ? (
                      <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{telegramConfigError}</p>
                    ) : null}
                    {telegramConfigSaveMessage ? (
                      <p className="mt-3 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">{telegramConfigSaveMessage}</p>
                    ) : null}
                    <div className="channel-setup-actions mt-auto flex flex-wrap gap-2 pt-5">
                      <button
                        type="button"
                        onClick={() => void saveTelegramConfig()}
                        disabled={telegramConfigSaving || telegramConfigLoading || !hasUnsavedTelegramConfigChanges}
                        className="theme-accent-btn theme-key-create-btn px-3 py-2 text-sm"
                      >
                        {telegramConfigSaving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Database className="h-4 w-4" />}
                        {t("保存 Telegram", "Save Telegram")}
                      </button>
                      <button
                        type="button"
                        onClick={() => void controlService("telegramd", health?.telegramd_healthy === true ? "restart" : "start")}
                        disabled={Boolean(serviceActionLoading.telegramd) || !telegramBotTokenConfigured}
                        className="theme-secondary-btn theme-key-create-btn px-3 py-2 text-sm"
                      >
                        {serviceActionLoading.telegramd ? <Loader2 className="h-4 w-4 animate-spin" /> : <Server className="h-4 w-4" />}
                        {health?.telegramd_healthy === true ? t("重启 Telegram 服务", "Restart the Telegram service") : t("启动 Telegram 服务", "Start the Telegram service")}
                      </button>
                    </div>
                  </div>

                  <div className="setup-channel-card channel-setup-card flex self-start flex-col">
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <h4 className="text-lg font-semibold text-white">{t("飞书", "Feishu")}</h4>
                        <p className="mt-2 text-sm leading-7 text-white/65">
                          {t(
                            "开始后会生成二维码，扫码打开机器人，再发送绑定码完成绑定。",
                            "Start to generate a QR code, then scan to open the bot and send the bind code to finish binding.",
                          )}
                        </p>
                      </div>
                      <span className={feishuStatusLoading ? "setup-status" : feishuStepStatus === "done" ? "setup-status setup-status-done" : feishuStepStatus === "attention" ? "setup-status setup-status-attention" : "setup-status setup-status-todo"}>
                        {feishuStatusLoading ? (
                          <>
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            {t("载入中", "Loading")}
                          </>
                        ) : feishuStepStatus === "done" ? t("已可用", "Ready") : feishuStepStatus === "attention" ? t("进行中", "In progress") : t("还没开始", "Not started")}
                      </span>
                    </div>

                    <p className="mt-4 text-sm leading-7 text-white/65">{feishuStatusSummary}</p>

                    {feishuConfigError ? (
                      <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{feishuConfigError}</p>
                    ) : null}
                    <p className="mt-3 text-sm text-white/55">
                      {lang === "zh" ? feishuSetupGuidance.zhHint : feishuSetupGuidance.enHint}
                    </p>

                    {!feishuCurrentKeyBound && feishuBindQrDataUrl ? (
                      <div className="mt-4 rounded-2xl border border-white/10 bg-black/18 p-4">
                        <div className="flex items-start justify-between gap-3">
                          <div>
                            <p className="text-sm font-medium text-white/92">{lang === "zh" ? feishuBindStatusCopy.zhLabel : feishuBindStatusCopy.enLabel}</p>
                            <p className="mt-2 text-xs leading-6 text-white/58">
                              {lang === "zh" ? feishuBindStatusCopy.zhDescription : feishuBindStatusCopy.enDescription}
                            </p>
                          </div>
                        </div>

                        <div className="mt-4 flex min-h-52 items-center justify-center rounded-[24px] border border-dashed border-white/12 bg-white/4">
                          <div className="inline-block rounded-[24px] border border-white/12 bg-white p-4 shadow-[0_24px_70px_rgba(6,10,18,0.22)]">
                            <img src={feishuBindQrDataUrl} alt="Feishu QR" className="h-52 w-52" />
                          </div>
                        </div>
                        {feishuBindSession && !isFeishuBindTerminalStatus(feishuBindSession.status) ? (
                          <div className="mt-4 rounded-2xl border border-sky-400/20 bg-sky-500/10 p-4">
                            <p className="text-xs font-medium uppercase tracking-[0.22em] text-sky-100/70">
                              {t("绑定码", "Bind code")}
                            </p>
                            <p className="mt-3 break-all rounded-xl bg-black/25 px-3 py-3 font-mono text-sm text-sky-50">
                              {feishuBindSession.bind_token}
                            </p>
                            <p className="mt-3 text-xs leading-6 text-sky-100/80">
                              {t(
                                "1. 扫码打开 RustClaw 飞书机器人。2. 把这串绑定码原样发给机器人。3. 页面会自动刷新为绑定成功。",
                                "1. Scan to open the RustClaw Feishu bot. 2. Send this bind code to the bot exactly as shown. 3. The page will refresh when binding succeeds.",
                              )}
                            </p>
                          </div>
                        ) : null}
                        {feishuBindSession && !feishuBindSession.entry_url ? (
                          <div className="mt-4 rounded-xl border border-amber-400/20 bg-amber-500/10 p-3 text-xs leading-6 text-amber-100/85">
                            {t(
                              "这次飞书接入还没有拿到可用二维码。稍等 1 到 2 秒后重试；如果还是不行，再去日志页面看 feishud.log。",
                              "This Feishu setup did not get a usable QR code yet. Wait 1-2 seconds and try again. If it still fails, check feishud.log on the logs page.",
                            )}
                          </div>
                        ) : null}
                      </div>
                    ) : null}

                    {feishuBindError ? (
                      <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{feishuBindError}</p>
                    ) : null}

                    <div className="channel-setup-actions mt-auto flex flex-wrap gap-2 pt-5">
                      <button
                        type="button"
                        onClick={() => void beginFeishuBind()}
                        disabled={feishuBindLoading || feishuResetLoading || !isAdminIdentity || !feishuSetupGuidance.canStartBind}
                        className="theme-accent-btn px-3 py-2 text-sm"
                      >
                        {feishuBindLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                        {feishuBindSession ? t("重新生成二维码", "Refresh QR") : t("开始飞书接入", "Start Feishu setup")}
                      </button>
                      {feishuSetupGuidance.canStartService || health?.feishud_healthy === true ? (
                        <button
                          type="button"
                          onClick={() => void controlService("feishud", health?.feishud_healthy === true ? "restart" : "start")}
                          disabled={Boolean(serviceActionLoading.feishud) || !canControlFeishuService}
                          className="theme-secondary-btn px-3 py-2 text-sm"
                        >
                          {serviceActionLoading.feishud ? <Loader2 className="h-4 w-4 animate-spin" /> : <Server className="h-4 w-4" />}
                          {health?.feishud_healthy === true
                            ? t("重启飞书服务", "Restart Feishu service")
                            : t("启动飞书服务", "Start Feishu service")}
                        </button>
                      ) : null}
                      <button
                        type="button"
                        onClick={() => void resetFeishuSetup()}
                        disabled={feishuResetLoading || feishuBindLoading || !isAdminIdentity}
                        className="theme-secondary-btn px-3 py-2 text-sm"
                      >
                        {feishuResetLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                        {t("重置飞书", "Reset Feishu")}
                      </button>
                    </div>
                  </div>
                </div>
              </section>

            </div>
          ) : null}

          {currentPage === "channels" ? (
            <div className="space-y-4">
              <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
                <div className="flex items-start justify-between gap-3">
                  <div>
                    <h3 className="text-base font-semibold">{t("账号绑定与 Key 管理", "Account binding and key management")}</h3>
                    <p className="mt-2 text-sm text-white/65">
                      {t("微信、Telegram 和飞书的快捷接入已经移到通信接入页。这里现在只保留账号绑定、访问 Key 生成与管理。", "Quick WeChat, Telegram, and Feishu setup moved to Communication Setup. This page now keeps account bindings plus access key generation and management.")}
                    </p>
                  </div>
                </div>
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
                  {isAdminIdentity ? (
                    <>
                      <button
                        type="button"
                        onClick={() => void createAuthKey("user")}
                        disabled={authKeyCreateLoading}
                        className="theme-accent-btn px-3 py-2 text-sm"
                      >
                        {authKeyCreateLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                        {t("生成新 Key（user）", "Generate new key (user)")}
                      </button>
                      <button
                        type="button"
                        onClick={() => void createAuthKey("guest")}
                        disabled={authKeyCreateLoading}
                        className="theme-secondary-btn px-3 py-2 text-sm"
                      >
                        {authKeyCreateLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                        {t("生成新 Key（guest）", "Generate new key (guest)")}
                      </button>
                      <button
                        type="button"
                        onClick={() => void promptCreateCustomAuthKey()}
                        disabled={authKeyCreateLoading}
                        className="theme-topbar-btn theme-key-create-btn px-3 py-2 text-sm"
                      >
                        {authKeyCreateLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                        {t("生成新 Key（自定义角色）", "Generate new key (custom role)")}
                      </button>
                    </>
                  ) : null}
                </div>
                {isAdminIdentity ? (
                  <p className="mt-3 rounded-lg border border-sky-400/25 bg-sky-500/10 px-3 py-2 text-sm text-sky-100">
                    {t("系统现在只允许 1 个 admin key。为保护记忆和绑定关系，key 一旦生成后不能修改；非 admin 登录后只会看到自己的 key。", "The system now allows only one admin key. To preserve memories and bindings, keys cannot be modified after creation; non-admin users only see their own key.")}
                  </p>
                ) : null}
                {!isAdminIdentity ? (
                  <p className="mt-3 rounded-lg border border-amber-400/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
                    {t("当前不是 admin：这里只显示你自己的 key；你不能新增、禁用、删除，也不能修改当前 key。", "Current key is not admin: only your own key is shown here; you cannot create, disable, delete, or modify the current key.")}
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
                    <p className="text-sm font-medium text-emerald-200">{t("新 Key 已生成，请复制保存", "New key generated. Copy and save it.")}</p>
                    <p className="mt-2 break-all font-mono text-sm text-white/90">{newlyCreatedKey}</p>
                    <div className="mt-3 flex flex-wrap items-center gap-2">
                      <button
                        type="button"
                        onClick={() => void copyAuthKey({ target: "new", plaintextKey: newlyCreatedKey })}
                        disabled={authKeyCopyingTarget === "new"}
                        className="theme-secondary-btn px-3 py-2 text-xs"
                      >
                        {authKeyCopiedTarget === "new" ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                        {authKeyCopyingTarget === "new"
                          ? t("复制中...", "Copying...")
                          : authKeyCopiedTarget === "new"
                            ? t("已复制", "Copied")
                            : t("复制 Key", "Copy key")}
                      </button>
                      <button
                        type="button"
                        onClick={() => setNewlyCreatedKey(null)}
                        className="text-xs text-white/70 underline"
                      >
                        {t("关闭", "Dismiss")}
                      </button>
                    </div>
                  </div>
                ) : null}
                <div className="mt-4 rounded-xl border border-white/10 bg-black/20 overflow-hidden">
                  <table className="w-full text-left text-sm">
                    <thead>
                      <tr className="border-b border-white/10 bg-white/5">
                        <th className="px-4 py-3 font-medium text-white/80">{t("Key", "Key")}</th>
                        <th className="px-4 py-3 font-medium text-white/80">role</th>
                        <th className="px-4 py-3 font-medium text-white/80">{t("网页登录", "Web login")}</th>
                        <th className="px-4 py-3 font-medium text-white/80">{t("启用", "Enabled")}</th>
                        <th className="px-4 py-3 font-medium text-white/80">{t("创建时间", "Created")}</th>
                        <th className="px-4 py-3 font-medium text-white/80">{t("最后使用", "Last used")}</th>
                        <th className="px-4 py-3 font-medium text-white/80">{t("操作", "Actions")}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {authKeysList.length === 0 && !authKeysLoading ? (
                        <tr>
                          <td colSpan={7} className="px-4 py-6 text-center text-white/50">
                            {isAdminIdentity
                              ? t("暂无数据，点击「刷新列表」或「生成新 Key」", "No keys yet. Click Refresh list or Generate new key.")
                              : t("暂无可显示的 key，请点击「刷新列表」", "No visible key yet. Click Refresh list.")}
                          </td>
                        </tr>
                      ) : (
                        sortedAuthKeysList.map((row) => {
                          const editingWebdLogin = webdLoginEditorKeyId === row.key_id;
                          return (
                            <Fragment key={row.key_id}>
                              <tr className="border-b border-white/5">
                                <td className="px-4 py-2 font-mono text-white/85">{row.user_key}</td>
                                <td className="px-4 py-2 text-white/75">{row.role}</td>
                                <td className="px-4 py-2 text-white/75">{row.webd_username || "--"}</td>
                                <td className="px-4 py-2">{row.enabled ? t("是", "Yes") : t("否", "No")}</td>
                                <td className="px-4 py-2 text-white/65">{formatDateOnlyHuman(row.created_at, lang === "zh" ? "zh-CN" : "en-US")}</td>
                                <td className="px-4 py-2 text-white/65">{formatDateTimeHuman(row.last_used_at)}</td>
                                <td className="px-4 py-2">
                                  {isAdminIdentity ? (
                                    <div className="flex flex-wrap items-center gap-2">
                                      <button
                                        type="button"
                                        disabled={authKeyCopyingTarget === row.key_id}
                                        className="theme-secondary-btn px-2 py-1 text-xs"
                                        onClick={() => void copyAuthKey({ target: row.key_id, keyId: row.key_id })}
                                      >
                                        {authKeyCopiedTarget === row.key_id ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                                        {authKeyCopyingTarget === row.key_id
                                          ? t("复制中...", "Copying...")
                                          : authKeyCopiedTarget === row.key_id
                                            ? t("已复制", "Copied")
                                            : t("复制 Key", "Copy key")}
                                      </button>
                                      {row.current_key ? (
                                        <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/55">
                                          {t("当前 key 不可修改", "Current key cannot be modified")}
                                        </span>
                                      ) : (
                                        <button
                                          type="button"
                                          disabled={authKeyActionLoading === row.key_id}
                                          className="theme-topbar-btn px-2 py-1 text-xs"
                                          onClick={() => void updateAuthKey(row.key_id, { enabled: !row.enabled })}
                                        >
                                          {row.enabled ? t("禁用", "Disable") : t("启用", "Enable")}
                                        </button>
                                      )}
                                      <button
                                        type="button"
                                        disabled={authKeyActionLoading === row.key_id || row.role === "admin"}
                                        className="theme-secondary-btn px-2 py-1 text-xs"
                                        onClick={() => void promptUpdateAuthKeyRole(row)}
                                      >
                                        {t("修改角色", "Change role")}
                                      </button>
                                      <button
                                        type="button"
                                        disabled={authKeyActionLoading === row.key_id}
                                        className="theme-secondary-btn px-2 py-1 text-xs"
                                        onClick={() => (editingWebdLogin ? closeWebdLoginEditor() : openWebdLoginEditor(row))}
                                      >
                                        {row.webd_username
                                          ? t("修改登录名/密码", "Update username/password")
                                          : t("设置登录名/密码", "Set username/password")}
                                      </button>
                                      <button
                                        type="button"
                                        disabled={authKeyActionLoading === row.key_id || row.role === "admin"}
                                        className="rounded-md border border-red-500/30 bg-red-500/10 px-2 py-1 text-xs text-red-200 transition hover:bg-red-500/20 disabled:opacity-50"
                                        onClick={() => void deleteAuthKey(row)}
                                      >
                                        {t("删除", "Delete")}
                                      </button>
                                    </div>
                                  ) : row.current_key ? (
                                    <span className="text-xs text-white/45">{t("当前 key 不可修改", "Current key cannot be modified")}</span>
                                  ) : (
                                    <span className="text-xs text-white/45">--</span>
                                  )}
                                </td>
                              </tr>
                              {isAdminIdentity && editingWebdLogin ? (
                                <tr className="border-b border-white/5 bg-white/[0.03]">
                                  <td colSpan={7} className="px-4 py-4">
                                    <div className="rounded-xl border border-white/10 bg-black/15 p-4">
                                      <div className="flex flex-wrap items-center justify-between gap-3">
                                        <div>
                                          <p className="text-sm font-medium text-white/90">
                                            {t("修改登录名/密码", "Update username/password")}
                                          </p>
                                          <p className="mt-1 text-xs text-white/55">
                                            {t(
                                              "为这个 Key 设置网页登录用户名和新密码。用户名会自动转成小写。",
                                              "Set the web login username and a new password for this key. The username will be normalized to lowercase.",
                                            )}
                                          </p>
                                        </div>
                                        <p className="font-mono text-xs text-white/45">{row.user_key}</p>
                                      </div>
                                      <div className="mt-4 grid gap-3 md:grid-cols-2">
                                        <label className="space-y-2">
                                          <span className="text-xs uppercase tracking-widest text-white/50">{t("登录名", "Username")}</span>
                                          <input
                                            value={webdLoginUsernameDraft}
                                            onChange={(e) => setWebdLoginUsernameDraft(e.target.value)}
                                            className="theme-input"
                                            placeholder={t("例如 rustclaw_admin", "For example rustclaw_admin")}
                                          />
                                        </label>
                                        <label className="space-y-2">
                                          <span className="text-xs uppercase tracking-widest text-white/50">{t("新密码", "New password")}</span>
                                          <input
                                            type="password"
                                            value={webdLoginPasswordDraft}
                                            onChange={(e) => setWebdLoginPasswordDraft(e.target.value)}
                                            className="theme-input"
                                            placeholder={t("输入新的登录密码", "Enter a new login password")}
                                          />
                                        </label>
                                      </div>
                                      <div className="mt-4 flex flex-wrap items-center gap-2">
                                        <button
                                          type="button"
                                          disabled={authKeyActionLoading === row.key_id}
                                          className="theme-accent-btn px-3 py-2 text-sm"
                                          onClick={() => void saveWebdLoginEditor(row)}
                                        >
                                          {authKeyActionLoading === row.key_id ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                                          {t("保存登录名/密码", "Save username/password")}
                                        </button>
                                        <button
                                          type="button"
                                          disabled={authKeyActionLoading === row.key_id}
                                          className="theme-topbar-btn px-3 py-2 text-sm"
                                          onClick={() => closeWebdLoginEditor()}
                                        >
                                          {t("取消", "Cancel")}
                                        </button>
                                      </div>
                                    </div>
                                  </td>
                                </tr>
                              ) : null}
                            </Fragment>
                          );
                        })
                      )}
                    </tbody>
                  </table>
                </div>
              </section>
            </div>
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
            <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
              <SkillImportPanel
                t={t}
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
              />

              <div className="rounded-xl border border-white/10 bg-black/20 p-4">
                <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
                  <h4 className="text-sm font-semibold">{t("工具与技能开关", "Tool and Skill Switches")}</h4>
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
                    "工具能力固定开启；技能按图片、语音、基础能力与其它分组展示。按钮只是先选择；点击“保存开关”后会提示重启，确认后系统会自动帮你重启并生效。",
                    "Tool capabilities stay always on. Skills are grouped by image, audio, core capabilities, and others. Buttons only stage your choice; after Save Switches you will be prompted to restart.",
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
                    const skillItem = skillItemsByName.get(name);
                    const runtimeIssue = skillRuntimeIssue(skillItem, lang);
                    const visiblePlannerCapabilities = (skillItem?.planner_capabilities ?? []).slice(0, 3);
                    const visibleCapabilities = (skillItem?.capabilities ?? []).slice(0, 3);
                    const configuredEnabled = configuredEnabledSkills.has(name);
                    const persistedSwitchValue = skillsConfigData?.skill_switches?.[name];
                    const draftSwitchValue = skillSwitchDraft[name];
                    const pendingApply = persistedSwitchValue !== draftSwitchValue;
                    const isRecentImport = recentImportedSkillName === name;
                    const isExternalSkill = externalSkillNamesSet.has(name);
                    const isLockedSkill = lockedSkillNamesSet.has(name);
                    const isToolSkill = toolSkillNamesSet.has(name);
                    const isUninstalling = skillUninstallingName === name;
                    const statusMeta = [
                      isToolSkill ? t("系统工具", "Tool") : null,
                      baseSkillNamesSet.has(name) && !isToolSkill ? t("系统基础能力", "Core capability") : null,
                      isLockedSkill ? t("固定开启", "Always on") : null,
                      isExternalSkill ? t("外部导入", "Imported") : null,
                      skillItem?.group ? `${t("分组", "Group")}: ${skillItem.group}` : null,
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
                          <span className="mt-1 flex flex-wrap gap-1">
                            <span className="rounded border border-white/10 bg-white/5 px-1.5 py-0.5 text-[10px] text-white/45">
                              {skillRiskLabel(skillItem?.risk_level, lang)}
                            </span>
                            {skillItem?.requires_confirmation ? (
                              <span className="rounded border border-amber-500/25 bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-100">
                                {t("操作前确认", "Confirms first")}
                              </span>
                            ) : null}
                            {skillItem?.side_effect ? (
                              <span className="rounded border border-sky-500/25 bg-sky-500/10 px-1.5 py-0.5 text-[10px] text-sky-100">
                                {t("会改变状态", "Changes state")}
                              </span>
                            ) : null}
                            {visiblePlannerCapabilities.map((capability) => (
                              <span
                                key={`planner-${capability}`}
                                className="rounded border border-cyan-500/20 bg-cyan-500/10 px-1.5 py-0.5 text-[10px] text-cyan-100"
                              >
                                {skillPlannerCapabilityLabel(capability, lang)}
                              </span>
                            ))}
                            {visibleCapabilities.map((capability) => (
                              <span key={`runtime-${capability}`} className="rounded border border-white/10 bg-white/5 px-1.5 py-0.5 text-[10px] text-white/45">
                                {skillCapabilityLabel(capability, lang)}
                              </span>
                            ))}
                          </span>
                          {runtimeIssue ? (
                            <span className="mt-1 flex items-start gap-1 text-[10px] leading-4 text-amber-200/90">
                              <AlertCircle className="mt-0.5 h-3 w-3 shrink-0" />
                              <span>{runtimeIssue}</span>
                            </span>
                          ) : skillItem?.missing_optional_bins?.length ? (
                            <span className="mt-1 block text-[10px] leading-4 text-white/35">
                              {t("可选工具未找到", "Optional tools missing")}: {skillItem.missing_optional_bins.join(", ")}
                            </span>
                          ) : null}
                        </span>
                        <span className="mt-1 flex shrink-0 flex-wrap items-center gap-1.5 sm:mt-0">
                          {skillItem?.runtime_available === false ? (
                            <span className="inline-flex items-center gap-1 rounded-full border border-amber-500/35 bg-amber-500/12 px-2 py-0.5 text-[10px] font-medium text-amber-200">
                              <Wrench className="h-3 w-3" />
                              {t("需配置", "Needs setup")}
                            </span>
                          ) : null}
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
                            disabled={isLockedSkill}
                            className={
                              isLockedSkill
                                ? "cursor-not-allowed rounded border border-emerald-500/25 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] text-emerald-100/80"
                                : "rounded border border-white/20 bg-white/5 px-1.5 py-0.5 text-[10px] text-white/80 hover:bg-white/10"
                            }
                            title={
                              isLockedSkill
                                ? isToolSkill
                                  ? t("这是底层工具能力，UI 中不能关闭。", "This is a low-level tool capability and cannot be disabled in the UI.")
                                  : t("这是系统基础能力，UI 中不能关闭。", "This is a core system capability and cannot be disabled in the UI.")
                                : configuredEnabled
                                  ? t("先设为关闭，保存后才会真正关闭", "Choose Disable first. It only turns off after you save.")
                                  : t("先设为开启，保存后才会真正开启", "Choose Enable first. It only turns on after you save.")
                            }
                          >
                            {isLockedSkill ? t("固定", "Fixed") : configuredEnabled ? t("关", "Off") : isRecentImport ? t("启用", "Enable") : t("开", "On")}
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
                          <h5 className="text-sm font-semibold text-white">{t("工具与技能分组", "Tools and skills by group")}</h5>
                          <span className="theme-meta-pill !rounded-xl !px-2.5 !py-1 text-[11px]">
                            {filteredManagedSkills.length}/{managedSkills.length}
                          </span>
                        </div>
                        <p className="mt-1 text-xs leading-5 text-white/50">
                          {t(
                            "工具固定开启；图片、语音、基础能力与其它技能可以按需管理。新导入的技能会出现在对应分组。",
                            "Tools stay always on; image, audio, core capabilities, and other skills can be managed as needed. Newly imported skills appear in the matching group.",
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
                        {renderSkillGroup(t("固定开启的工具", "Always-on tools"), filteredSkillsTool)}
                        {renderSkillGroup(t("固定开启的基础技能", "Always-on core skills"), filteredSkillsBase)}
                        {renderSkillGroup(t("图片技能", "Image skills"), filteredSkillsImage)}
                        {renderSkillGroup(t("语音技能", "Voice / Audio skills"), filteredSkillsAudio)}
                        {renderSkillGroup(t("其他", "Others"), filteredSkillsOther)}
                      </div>
                      {normalizedSkillsSearchQuery &&
                        filteredSkillsTool.length === 0 &&
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
            <>
              <ActiveTasksPanel
                lang={lang}
                t={t}
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
              />

              <ManualTaskSubmitPanel
                t={t}
                tSlash={tSlash}
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
              />

              <TaskResultPanel
                lang={lang}
                t={t}
                tSlash={tSlash}
                taskId={taskId}
                taskLoading={taskLoading}
                taskError={taskError}
                taskResult={taskResult}
                resumeDrafts={resumeDrafts}
                resumeSubmittingTaskId={resumeSubmittingTaskId}
                onTaskIdChange={setTaskId}
                onQueryTask={queryTask}
                onResumeDraftChange={(taskIdToResume, value) =>
                  setResumeDrafts((prev) => ({
                    ...prev,
                    [taskIdToResume]: value,
                  }))
                }
                onSubmitResume={submitResumeForTask}
              />
            </>
          ) : null}
        </main>
      </div>

    </div>
  );
}
