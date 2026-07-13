import { useEffect, useMemo, useRef, useState } from "react";
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
import { maskStoredKey } from "./lib/auth-keys";
import { formatDuration, toLocalTime } from "./lib/display-format";
import {
  formatDateTimeHuman as formatDateTimeHumanValue,
  formatUnixDateTime as formatUnixDateTimeValue,
} from "./lib/date-format";
import {
  MULTIMODAL_KEYS,
  buildMultimodalMetaView,
  type MultimodalKey,
} from "./lib/model-config";
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
import { useServiceActionsRuntime } from "./hooks/useServiceActionsRuntime";
import { useWhatsappWebRuntime } from "./hooks/useWhatsappWebRuntime";
import { useWechatRuntime } from "./hooks/useWechatRuntime";
import { useFeishuBindRuntime } from "./hooks/useFeishuBindRuntime";
import { useChatRuntime } from "./hooks/useChatRuntime";
import { useTaskRuntime } from "./hooks/useTaskRuntime";
import { useSystemRuntime } from "./hooks/useSystemRuntime";
import { useConsoleProjections } from "./hooks/useConsoleProjections";

import type {
  ApiResponse,
  HealthResponse,
  LocalInteractionContextResponse,
  AuthIdentityResponse,
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

  const [interactionUserId, setInteractionUserId] = useState<number | null>(null);
  const [interactionChatId, setInteractionChatId] = useState<number | null>(null);
  const [interactionRole, setInteractionRole] = useState<string>("-");
  const [localContextLoading, setLocalContextLoading] = useState(false);
  const [localContextError, setLocalContextError] = useState<string | null>(null);
  const [diagnosticsRefreshing, setDiagnosticsRefreshing] = useState(false);
  const [currentPage, setCurrentPage] = useState<ConsolePage>(() => {
    const saved = window.localStorage.getItem(STORAGE_KEYS.currentPage);
    return saved && CONSOLE_PAGES.includes(saved as ConsolePage) ? (saved as ConsolePage) : "dashboard";
  });
  const logContainerRef = useRef<HTMLPreElement | null>(null);

  const t = (zh: string, en: string) => (lang === "zh" ? zh : en);
  const isAdminIdentity = authIdentity?.role?.toLowerCase() === "admin";
  const tSlash = (mixed: string) => {
    const [zh, en] = mixed.split(" / ");
    return lang === "zh" ? zh : en ?? zh;
  };
  const dateLocale = lang === "zh" ? "zh-CN" : "en-US";
  const formatDateTimeHuman = (raw: string | null | undefined) => {
    return formatDateTimeHumanValue(raw, dateLocale);
  };
  const formatUnixDateTime = (ts: number | null | undefined) => {
    return formatUnixDateTimeValue(ts, dateLocale);
  };

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
    onBeforeSaveLlm: () => {},
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
  const {
    taskId,
    setTaskId,
    taskLoading,
    taskResult,
    taskError,
    taskLlmDebug,
    taskLlmDebugLoading,
    taskLlmDebugError,
    trackingTaskId,
    activeTasks,
    activeTasksLoading,
    activeTasksError,
    activeTasksLastUpdated,
    resumeDrafts,
    resumeSubmittingTaskId,
    resumeTaskMessage,
    resumeTaskError,
    cancelingTaskIndex,
    cancelTaskMessage,
    cancelTaskError,
    taskControlSubmittingId,
    taskControlMessage,
    taskControlError,
    interactionKind,
    setInteractionKind,
    interactionChannel,
    setInteractionChannel,
    interactionExternalUserId,
    setInteractionExternalUserId,
    interactionExternalChatId,
    setInteractionExternalChatId,
    interactionAdapter,
    setInteractionAdapter,
    interactionAskText,
    setInteractionAskText,
    interactionAgentMode,
    setInteractionAgentMode,
    interactionSkillName,
    setInteractionSkillName,
    interactionSkillArgs,
    setInteractionSkillArgs,
    interactionLoading,
    interactionError,
    interactionSubmittedTaskId,
    fetchTaskById,
    fetchActiveTasks,
    queryTask,
    queryTaskLlmDebug,
    viewTask,
    setResumeDraftValue,
    submitResumeForTask,
    cancelActiveTask,
    controlTaskById,
    submitInteractionTask,
    markTaskSubmitted,
    recordTaskResult,
  } = useTaskRuntime({
    apiFetch,
    t,
    apiBase,
    uiAuthReady,
    currentPage,
    interactionUserId,
    interactionChatId,
    activeUserKey,
    activeIdentityIds,
  });
  const {
    chatMessages,
    chatThreads,
    activeChatThreadId,
    chatInput,
    chatAttachments,
    chatAgentMode,
    chatTeachingMode,
    chatTeachingTaskResult,
    chatTeachingLlmDebug,
    chatTeachingLlmDebugLoading,
    chatTeachingLlmDebugError,
    chatSending,
    chatRecording,
    chatVoiceRecordingSupported,
    chatError,
    chatAttachmentInputRef,
    setChatAgentMode,
    setChatTeachingMode,
    createNewChatThread,
    selectChatThread,
    deleteChatThread,
    clearChatMessages,
    setChatInput,
    handleChatInputKeyDown,
    handleChatAttachmentSelection,
    removeChatAttachment,
    startChatVoiceRecording,
    stopChatVoiceRecording,
    sendChatMessage,
    queryChatTeachingLlmDebug,
  } = useChatRuntime({
    apiFetch,
    t,
    lang,
    interactionAdapter,
    interactionChannel,
    activeUserKey,
    activeIdentityIds,
    interactionExternalUserId,
    interactionExternalChatId,
    fetchTaskById,
    onTaskSubmitted: markTaskSubmitted,
    onTaskResult: recordTaskResult,
  });

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
    systemRestarting,
    systemRestartMessage,
    piAppStatus,
    piAppRestarting,
    piAppRestartMessage,
    workspaceUpdateStatus,
    workspaceUpdateLoading,
    workspaceUpdateCanceling,
    workspaceUpdateMessage,
    fetchWorkspaceUpdateStatus,
    startWorkspaceUpdate,
    cancelWorkspaceUpdate,
    restartSystem,
    restartPiApp,
  } = useSystemRuntime({
    apiFetch,
    t,
    apiBase,
    uiAuthReady,
    isAdminIdentity,
    fetchHealth,
    setHealth,
    setError,
    clearLlmConfigError,
    clearSkillsConfigError,
    fetchLlmConfig,
    fetchMultimodalConfig,
    fetchSkillsConfig,
    fetchSkills,
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
  const {
    waLoginDialogOpen,
    setWaLoginDialogOpen,
    waLoginLoading,
    waLoginError,
    waLoginStatus,
    waWebBridgeReachable,
    waLogoutLoading,
    fetchWhatsappWebLoginStatus,
    logoutWhatsappWeb,
  } = useWhatsappWebRuntime({
    apiFetch,
    t,
    apiBase,
    uiAuthReady,
    whatsappWebHealthy: health?.whatsapp_web_healthy === true,
    setServiceActionMessage,
  });
  const {
    wechatLoginLoading,
    wechatLoginError,
    wechatLoginStatus,
    wechatQrStarting,
    wechatQrPreviewRequested,
    fetchWechatLoginStatus,
    startWechatQrLogin,
  } = useWechatRuntime({
    apiFetch,
    t,
    apiBase,
    uiAuthReady,
  });
  const {
    feishuBindLoading,
    feishuBindError,
    feishuBindSession,
    feishuBindQrDataUrl,
    feishuResetLoading,
    beginFeishuBind,
    resetFeishuSetup,
  } = useFeishuBindRuntime({
    apiFetch,
    t,
    uiAuthReady,
    onConfigRefresh: async () => {
      await fetchFeishuConfig();
    },
    onHealthRefresh: async () => {
      await fetchHealth();
    },
  });
  const {
    isOnline,
    queuePressureHigh,
    runningTooOld,
    wechatStatusLoading,
    telegramStatusLoading,
    feishuStatusLoading,
    wechatStepStatus,
    telegramStepStatus,
    dashboardCommunicationRows,
    feishuBindStatusCopy,
    feishuCurrentKeyBound,
    feishuSetupGuidance,
    feishuStepStatus,
    canControlFeishuService,
    wechatStatusSummary,
    telegramStatusSummary,
    feishuStatusSummary,
    navItems,
    onboardingSteps,
    dashboardOverviewItems,
  } = useConsoleProjections({
    lang,
    t,
    health,
    error,
    queueWarn,
    ageWarnSeconds,
    llmStepStatus,
    chatMessages,
    wechatConfigLoading,
    wechatConfigData,
    wechatConfigError,
    wechatLoginStatus,
    telegramConfigLoading,
    telegramConfigData,
    telegramConfigError,
    telegramBotTokenConfigured,
    hasUnsavedTelegramConfigChanges,
    feishuConfigLoading,
    feishuConfigData,
    feishuConfigError,
    feishuBindSession,
  });

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

  const maskedSavedUiKey = useMemo(() => {
    if (authMode === "webd") return "";
    return maskStoredKey(uiKey);
  }, [uiKey, authMode]);
  const maskedIdentityKey = useMemo(() => {
    const currentKey = authIdentity?.user_key?.trim() || "";
    return currentKey ? maskStoredKey(currentKey) : "";
  }, [authIdentity?.user_key]);
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
              tSlash={tSlash}
              chatMessages={chatMessages}
              chatThreads={chatThreads}
              activeChatThreadId={activeChatThreadId}
              chatInput={chatInput}
              chatAttachments={chatAttachments}
              chatAgentMode={chatAgentMode}
              chatTeachingMode={chatTeachingMode}
              chatTeachingTaskResult={chatTeachingTaskResult}
              chatTeachingLlmDebug={chatTeachingLlmDebug}
              chatTeachingLlmDebugLoading={chatTeachingLlmDebugLoading}
              chatTeachingLlmDebugError={chatTeachingLlmDebugError}
              chatSending={chatSending}
              chatRecording={chatRecording}
              chatVoiceRecordingSupported={chatVoiceRecordingSupported}
              chatError={chatError}
              chatAttachmentInputRef={chatAttachmentInputRef}
              toLocalTime={toLocalTime}
              onChatAgentModeChange={setChatAgentMode}
              onChatTeachingModeChange={setChatTeachingMode}
              onCreateNewChatThread={createNewChatThread}
              onSelectChatThread={selectChatThread}
              onDeleteChatThread={deleteChatThread}
              onClearMessages={clearChatMessages}
              onChatInputChange={setChatInput}
              onChatInputKeyDown={handleChatInputKeyDown}
              onAttachmentSelection={handleChatAttachmentSelection}
              onRemoveAttachment={removeChatAttachment}
              onStartVoiceRecording={startChatVoiceRecording}
              onStopVoiceRecording={stopChatVoiceRecording}
              onSendMessage={sendChatMessage}
              onQueryChatTeachingLlmDebug={queryChatTeachingLlmDebug}
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
              taskControlSubmittingId={taskControlSubmittingId}
              taskControlMessage={taskControlMessage}
              taskControlError={taskControlError}
              canUseInteractionContext={interactionUserId != null && interactionChatId != null}
              resumeDrafts={resumeDrafts}
              resumeSubmittingTaskId={resumeSubmittingTaskId}
              toLocalTime={toLocalTime}
              onFetchActiveTasks={fetchActiveTasks}
              onViewTask={viewTask}
              onCancelTask={cancelActiveTask}
              onControlTask={controlTaskById}
              onResumeDraftChange={setResumeDraftValue}
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
              taskLlmDebug={taskLlmDebug}
              taskLlmDebugLoading={taskLlmDebugLoading}
              taskLlmDebugError={taskLlmDebugError}
              onTaskIdChange={setTaskId}
              onQueryTask={queryTask}
              onQueryTaskLlmDebug={queryTaskLlmDebug}
            />
          ) : null}
    </ConsoleLayout>
  );
}
