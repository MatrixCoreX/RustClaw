import { appStorageKey } from "./product-identity";
import { formatUiError } from "./ui-error";
import { conversationHistoryStorageKey, type ServerChatThreadProjection } from "./chat-history";
import { ChatAttachmentConstraintError, formatAttachmentSize } from "./chat-attachments";
import { normalizeTaskArtifacts, normalizeTaskArtifactDeliverySummary } from "./task-artifacts";
import type { ChatMessage, TaskLlmDebugResponse, TaskQueryResponse, UiAttachmentConstraints } from "../types/api";
import type { ChatThreadRecord, ChatThreadState, ChatThreadSummary, ChatTeachingRunRecord, ChatTeachingRunSummary } from "../types/chat-runtime";

type Translate = (zh: string, en: string) => string;

export function threadHasServerHistory(thread: ChatThreadRecord): boolean {
  return (
    Boolean(thread.lastTaskId) ||
    (thread.teachingRuns ?? []).some((run) => Boolean(run.taskId))
  );
}

export function emptyChatThreadState(t: Translate, defaultAgentId = "main"): ChatThreadState {
  const fallback = createChatThread(t, defaultAgentId);
  return { activeThreadId: fallback.id, threads: [fallback] };
}

export function loadChatThreadState(
  t: Translate,
  scope: string,
  defaultAgentId = "main",
): ChatThreadState {
  const fallback = emptyChatThreadState(t, defaultAgentId);
  if (typeof window === "undefined") {
    return fallback;
  }
  try {
    const storageKey = conversationHistoryStorageKey(scope);
    const raw = storageKey ? window.localStorage.getItem(storageKey) : null;
    if (!raw) {
      return fallback;
    }
    const parsed = JSON.parse(raw) as Partial<ChatThreadState>;
    const threads = Array.isArray(parsed.threads)
      ? parsed.threads
          .map((thread) => normalizeStoredChatThread(thread, t, defaultAgentId))
          .filter((thread): thread is ChatThreadRecord => Boolean(thread))
      : [];
    if (threads.length === 0) {
      return fallback;
    }
    const activeThreadId =
      typeof parsed.activeThreadId === "string" &&
      threads.some((thread) => thread.id === parsed.activeThreadId)
        ? parsed.activeThreadId
        : threads[0].id;
    return { activeThreadId, threads };
  } catch {
    return fallback;
  }
}

export function persistChatThreadState(state: ChatThreadState, scope: string) {
  if (typeof window === "undefined") return;
  try {
    const storageKey = conversationHistoryStorageKey(scope);
    if (!storageKey) return;
    const payload: ChatThreadState = {
      activeThreadId: state.activeThreadId,
      threads: state.threads.map((thread) => ({
        ...thread,
        teachingTaskResult: thread.teachingTaskResult
          ? compactTaskResultForChatStorage(thread.teachingTaskResult)
          : null,
        teachingLlmDebug: null,
        teachingLlmDebugError: null,
        activeTeachingRunId: thread.activeTeachingRunId ?? null,
        teachingRuns: (thread.teachingRuns ?? []).map(compactTeachingRunForChatStorage),
        messages: thread.messages.map(stripAttachmentPayloadsFromMessage),
      })),
    };
    window.localStorage.setItem(storageKey, JSON.stringify(payload));
  } catch {
    // Local history is a convenience cache; quota/private-mode failures must not block chat.
  }
}

export function mergeServerConversationHistory(
  current: ChatThreadState,
  restored: ServerChatThreadProjection[],
  t: Translate,
  defaultAgentId = "main",
): ChatThreadState {
  const existingById = new Map(current.threads.map((thread) => [thread.id, thread]));
  const serverThreads = restored.map((thread) => {
    const existing = existingById.get(thread.id);
    const existingInitialRunsByTask = new Map(
      (existing?.teachingRuns ?? [])
        .filter((run) => run.taskId && !run.conversationInputId)
        .map((run) => [run.taskId as string, run]),
    );
    const existingRunsByInput = new Map(
      (existing?.teachingRuns ?? [])
        .filter((run) => run.conversationInputId)
        .map((run) => [run.conversationInputId as string, run]),
    );
    const restoredRuns = thread.teachingRuns.map((run) => {
      const local = run.conversationInputId
        ? existingRunsByInput.get(run.conversationInputId)
        : existingInitialRunsByTask.get(run.taskId);
      return {
        ...run,
        conversationInputId: local?.conversationInputId ?? null,
        conversationInputClientMessageId:
          local?.conversationInputClientMessageId ?? null,
        conversationInputRevision: local?.conversationInputRevision ?? null,
        llmDebug: local?.llmDebug ?? null,
        llmDebugError: local?.llmDebugError ?? null,
        callCount: local?.callCount ?? debugCallCount(local?.llmDebug),
      };
    });
    const restoredTaskIds = new Set(restoredRuns.map((run) => run.taskId));
    const teachingRuns = [
      ...(existing?.teachingRuns ?? []).filter((run) => !restoredTaskIds.has(run.taskId)),
      ...restoredRuns,
    ].sort((left, right) => left.startedAt - right.startedAt);
    const restoredMessageIds = new Set(thread.messages.map((message) => message.id));
    const replacedLocalMessageIds = new Set(
      (existing?.teachingRuns ?? [])
        .filter((run) => Boolean(run.taskId) && restoredTaskIds.has(run.taskId))
        .flatMap((run) => [run.userMessageId, run.assistantMessageId])
        .filter((messageId): messageId is string => Boolean(messageId)),
    );
    const messages = [
      ...(existing?.messages ?? []).filter(
        (message) =>
          !restoredMessageIds.has(message.id) && !replacedLocalMessageIds.has(message.id),
      ),
      ...thread.messages,
    ].sort((left, right) => left.ts - right.ts || left.id.localeCompare(right.id));
    const latestRun = teachingRuns[teachingRuns.length - 1] ?? null;
    const activeTeachingRunId =
      existing?.activeTeachingRunId &&
      teachingRuns.some((run) => run.id === existing.activeTeachingRunId)
        ? existing.activeTeachingRunId
        : latestRun?.id ?? null;
    return {
      id: thread.id,
      agentId: thread.agentId || existing?.agentId || defaultAgentId,
      title: thread.title || t("未命名任务", "Untitled task"),
      messages,
      input: existing?.input ?? "",
      createdAt: Math.min(existing?.createdAt ?? thread.createdAt, thread.createdAt),
      updatedAt: Math.max(existing?.updatedAt ?? thread.updatedAt, thread.updatedAt),
      teachingMode: existing?.teachingMode ?? false,
      externalChatId: thread.externalChatId,
      lastTaskId:
        !existing || thread.updatedAt >= existing.updatedAt
          ? thread.lastTaskId
          : existing.lastTaskId ?? thread.lastTaskId,
      teachingTaskResult: latestRun?.taskResult ?? null,
      teachingLlmDebug: null,
      teachingLlmDebugError: null,
      activeTeachingRunId,
      teachingRuns,
    } satisfies ChatThreadRecord;
  });
  const retainedThreads = current.threads.filter(
    (thread) => !restored.some((candidate) => candidate.id === thread.id),
  );
  const threads = [...retainedThreads, ...serverThreads].sort(
    (left, right) => right.updatedAt - left.updatedAt,
  );
  if (threads.length === 0) {
    const fallback = createChatThread(t, defaultAgentId);
    return { activeThreadId: fallback.id, threads: [fallback] };
  }
  const activeThreadId = threads.some((thread) => thread.id === current.activeThreadId)
    ? current.activeThreadId
    : threads[0].id;
  return { activeThreadId, threads };
}

export function retainLocalDraftsForPagedRestore(
  current: ChatThreadState,
): ChatThreadState {
  const threads = current.threads.filter(
    (thread) =>
      threadHasPendingTask(thread) ||
      (!threadHasServerHistory(thread) && !threadIsPristineWelcome(thread)),
  );
  return {
    activeThreadId: current.activeThreadId,
    threads,
  };
}

export function threadIsPristineWelcome(thread: ChatThreadRecord): boolean {
  return (
    !thread.input.trim() &&
    !thread.teachingMode &&
    !thread.lastTaskId &&
    (thread.teachingRuns ?? []).length === 0 &&
    thread.messages.length === 1 &&
    thread.messages[0].role === "system" &&
    thread.messages[0].id.startsWith("chat-system-welcome-")
  );
}

export function normalizeStoredChatThread(
  raw: unknown,
  t: Translate,
  defaultAgentId = "main",
): ChatThreadRecord | null {
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Partial<ChatThreadRecord>;
  if (typeof record.id !== "string" || !record.id.trim()) return null;
  const now = Date.now();
  const messages = Array.isArray(record.messages)
    ? record.messages
        .map(normalizeStoredChatMessage)
        .filter((message): message is ChatMessage => Boolean(message))
    : [];
  return {
    id: record.id,
    agentId:
      typeof record.agentId === "string" && record.agentId.trim()
        ? record.agentId.trim()
        : defaultAgentId,
    title:
      typeof record.title === "string" && record.title.trim()
        ? record.title.trim()
        : t("未命名任务", "Untitled task"),
    messages: messages.length > 0 ? messages : [welcomeChatMessage(t)],
    input: typeof record.input === "string" ? record.input : "",
    createdAt: typeof record.createdAt === "number" ? record.createdAt : now,
    updatedAt: typeof record.updatedAt === "number" ? record.updatedAt : now,
    teachingMode: typeof record.teachingMode === "boolean" ? record.teachingMode : false,
    externalChatId:
      typeof record.externalChatId === "string" && record.externalChatId.trim()
        ? record.externalChatId.trim()
        : createThreadExternalChatId(),
    lastTaskId: typeof record.lastTaskId === "string" ? record.lastTaskId : null,
    teachingTaskResult: normalizeStoredTaskResult(record.teachingTaskResult),
    teachingLlmDebug: null,
    teachingLlmDebugError: null,
    activeTeachingRunId:
      typeof record.activeTeachingRunId === "string" ? record.activeTeachingRunId : null,
    teachingRuns: Array.isArray(record.teachingRuns)
      ? record.teachingRuns
          .map(normalizeStoredTeachingRun)
          .filter((run): run is ChatTeachingRunRecord => Boolean(run))
      : [],
  };
}

export function compactTeachingRunForChatStorage(run: ChatTeachingRunRecord): ChatTeachingRunRecord {
  return {
    id: run.id,
    taskId: run.taskId ?? null,
    conversationInputId: run.conversationInputId ?? null,
    conversationInputClientMessageId: run.conversationInputClientMessageId ?? null,
    conversationInputRevision: run.conversationInputRevision ?? null,
    userMessageId: run.userMessageId,
    assistantMessageId: run.assistantMessageId ?? null,
    userText: run.userText,
    assistantText: run.assistantText ?? null,
    status: run.status,
    startedAt: run.startedAt,
    completedAt: run.completedAt ?? null,
    taskResult: run.taskResult ? compactTaskResultForChatStorage(run.taskResult) : null,
    llmDebug: null,
    llmDebugError: null,
    callCount: run.callCount ?? debugCallCount(run.llmDebug),
  };
}

export function compactTaskResultForChatStorage(result: TaskQueryResponse): TaskQueryResponse {
  return {
    task_id: result.task_id,
    status: result.status,
    goal: result.goal ?? null,
    result_json: null,
    error_text: result.error_text ?? null,
  };
}

export function normalizeStoredTeachingRun(raw: unknown): ChatTeachingRunRecord | null {
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Partial<ChatTeachingRunRecord>;
  if (
    typeof record.id !== "string" ||
    typeof record.userMessageId !== "string" ||
    typeof record.userText !== "string" ||
    typeof record.startedAt !== "number"
  ) {
    return null;
  }
  const status = isTaskStatusOrRunning(record.status) ? record.status : "running";
  return {
    id: record.id,
    taskId: typeof record.taskId === "string" && record.taskId.trim() ? record.taskId : null,
    conversationInputId:
      typeof record.conversationInputId === "string" && record.conversationInputId.trim()
        ? record.conversationInputId
        : null,
    conversationInputClientMessageId:
      typeof record.conversationInputClientMessageId === "string" &&
      record.conversationInputClientMessageId.trim()
        ? record.conversationInputClientMessageId
        : null,
    conversationInputRevision:
      typeof record.conversationInputRevision === "number"
        ? record.conversationInputRevision
        : null,
    userMessageId: record.userMessageId,
    assistantMessageId:
      typeof record.assistantMessageId === "string" ? record.assistantMessageId : null,
    userText: record.userText,
    assistantText: typeof record.assistantText === "string" ? record.assistantText : null,
    status,
    startedAt: record.startedAt,
    completedAt: typeof record.completedAt === "number" ? record.completedAt : null,
    taskResult: normalizeStoredTaskResult(record.taskResult),
    llmDebug: null,
    llmDebugError: null,
    callCount: typeof record.callCount === "number" ? record.callCount : null,
  };
}

export function isTaskStatusOrRunning(value: unknown): value is ChatTeachingRunRecord["status"] {
  return ["queued", "running", "succeeded", "failed", "canceled", "timeout"].includes(String(value));
}

export function activeTaskStatus(status: ChatTeachingRunRecord["status"]): boolean {
  return status === "queued" || status === "running";
}

export function terminalTaskStatus(status: TaskQueryResponse["status"]): boolean {
  return !activeTaskStatus(status);
}

export function formatChatAttachmentError(
  error: unknown,
  constraints: UiAttachmentConstraints,
  t: Translate,
): string {
  if (!(error instanceof ChatAttachmentConstraintError)) {
    return formatUiError(error, t, "读取文件失败。", "Failed to read files.");
  }
  switch (error.code) {
    case "ui_attachments_too_many":
      return t(
        `一次最多发送 ${constraints.max_attachments} 个附件。`,
        `You can send up to ${constraints.max_attachments} attachments at once.`,
      );
    case "ui_attachment_too_large":
      return t(
        `单个附件不能超过 ${formatAttachmentSize(constraints.max_attachment_bytes)}。`,
        `Each attachment must be no larger than ${formatAttachmentSize(constraints.max_attachment_bytes)}.`,
      );
    case "ui_attachments_total_too_large":
      return t(
        `附件总大小不能超过 ${formatAttachmentSize(constraints.max_total_attachment_bytes)}。`,
        `The total attachment size must not exceed ${formatAttachmentSize(constraints.max_total_attachment_bytes)}.`,
      );
    default:
      return t("附件不符合上传要求。", "The attachments do not meet the upload requirements.");
  }
}

export function threadHasPendingTask(thread: ChatThreadRecord): boolean {
  return (thread.teachingRuns ?? []).some(
    (run) => Boolean(run.taskId) && activeTaskStatus(run.status),
  );
}

export function normalizeStoredTaskResult(raw: unknown): TaskQueryResponse | null {
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Partial<TaskQueryResponse>;
  if (typeof record.task_id !== "string" || !record.task_id.trim()) return null;
  return {
    task_id: record.task_id,
    status: typeof record.status === "string" ? record.status : "succeeded",
    goal: record.goal ?? null,
    result_json: null,
    error_text: typeof record.error_text === "string" ? record.error_text : null,
  };
}

export function normalizeStoredChatMessage(raw: unknown): ChatMessage | null {
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Partial<ChatMessage>;
  if (
    typeof record.id !== "string" ||
    typeof record.text !== "string" ||
    typeof record.ts !== "number" ||
    !["user", "assistant", "system"].includes(String(record.role))
  ) {
    return null;
  }
  return {
    id: record.id,
    role: record.role as ChatMessage["role"],
    text: record.text,
    ts: record.ts,
    artifacts: normalizeTaskArtifacts(record.artifacts),
    artifactDelivery: normalizeTaskArtifactDeliverySummary(record.artifactDelivery),
    bodyResult: normalizeStoredConversationBodyDescriptor(record.bodyResult),
  };
}

export function stripAttachmentPayloadsFromMessage(message: ChatMessage): ChatMessage {
  return {
    id: message.id,
    role: message.role,
    text: message.text,
    ts: message.ts,
    artifacts: normalizeTaskArtifacts(message.artifacts),
    artifactDelivery: normalizeTaskArtifactDeliverySummary(message.artifactDelivery),
    bodyResult: normalizeStoredConversationBodyDescriptor(message.bodyResult),
  };
}

export function normalizeStoredConversationBodyDescriptor(
  raw: ChatMessage["bodyResult"],
): ChatMessage["bodyResult"] {
  if (!raw || typeof raw !== "object") return null;
  if (
    raw.schema_version !== 1 ||
    typeof raw.complete !== "boolean" ||
    !Number.isSafeInteger(raw.original_size_bytes) ||
    !Number.isSafeInteger(raw.returned_size_bytes) ||
    raw.original_size_bytes < raw.returned_size_bytes ||
    !/^[0-9a-f]{64}$/i.test(raw.content_sha256)
  ) {
    return null;
  }
  if (
    !raw.complete &&
    (!raw.continuation ||
      raw.continuation.kind !== "conversation_body_range" ||
      typeof raw.continuation.url !== "string" ||
      !Number.isSafeInteger(raw.continuation.next_start_byte))
  ) {
    return null;
  }
  return raw;
}

export function createChatThread(t: Translate, agentId = "main"): ChatThreadRecord {
  const now = Date.now();
  return {
    id: `chat-thread-${now}-${Math.random().toString(36).slice(2, 8)}`,
    agentId,
    title: t("新任务", "New task"),
    messages: [welcomeChatMessage(t)],
    input: "",
    createdAt: now,
    updatedAt: now,
    teachingMode: false,
    externalChatId: createThreadExternalChatId(),
    lastTaskId: null,
    teachingTaskResult: null,
    teachingLlmDebug: null,
    teachingLlmDebugError: null,
    activeTeachingRunId: null,
    teachingRuns: [],
  };
}

export function createThreadExternalChatId(): string {
  return `ui-chat-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

export function welcomeChatMessage(t: Translate): ChatMessage {
  return {
    id: `chat-system-welcome-${Date.now()}`,
    role: "system",
    text: t(
      "会话窗口已连接 clawd。发送消息后会自动提交 ask 任务并轮询结果。",
      "The chat window is connected to clawd. Messages submit ask tasks and poll for results automatically.",
    ),
    ts: Date.now(),
  };
}

export function clearedChatMessage(t: Translate): ChatMessage {
  return {
    id: `chat-clear-${Date.now()}`,
    role: "system",
    text: t("当前任务的聊天记录已清空。", "This task's chat history was cleared."),
    ts: Date.now(),
  };
}

export function buildChatThreadSummaries(
  threads: ChatThreadRecord[],
  t: Translate,
): ChatThreadSummary[] {
  return threads.map((thread) => {
      const latestRun = latestTeachingRun(thread);
      const taskResult = latestRun?.taskResult ?? thread.teachingTaskResult ?? null;
      return {
        id: thread.id,
        agentId: thread.agentId,
        title: thread.title,
        preview: threadPreview(thread, t),
        updatedAt: thread.updatedAt,
        messageCount: thread.messages.filter((message) => message.role !== "system").length,
        teachingMode: thread.teachingMode,
        taskId: latestRun?.taskId ?? taskResult?.task_id ?? thread.lastTaskId ?? null,
        taskStatus: latestRun?.status ?? taskResult?.status ?? null,
        llmCallCount:
          latestRun?.callCount ??
          debugCallCount(latestRun?.llmDebug) ??
          debugCallCount(thread.teachingLlmDebug),
      };
    });
}

export function selectedTeachingRun(thread: ChatThreadRecord): ChatTeachingRunRecord | null {
  const runs = thread.teachingRuns ?? [];
  if (runs.length === 0) return null;
  const activeId = thread.activeTeachingRunId;
  const activeRun = runs.find((run) => run.id === activeId);
  if (activeRun) return activeRun;
  return thread.teachingMode ? (runs[runs.length - 1] ?? null) : null;
}

export function latestTeachingRun(thread: ChatThreadRecord): ChatTeachingRunRecord | null {
  const runs = thread.teachingRuns ?? [];
  return runs.reduce<ChatTeachingRunRecord | null>((latest, run) => {
    if (!latest) return run;
    return run.startedAt >= latest.startedAt ? run : latest;
  }, null);
}

export function buildChatTeachingRunSummaries(thread: ChatThreadRecord): ChatTeachingRunSummary[] {
  const activeId = selectedTeachingRun(thread)?.id ?? null;
  return [...(thread.teachingRuns ?? [])]
    .sort(
      (left, right) =>
        right.startedAt - left.startedAt ||
        (right.conversationInputRevision ?? -1) -
          (left.conversationInputRevision ?? -1) ||
        right.id.localeCompare(left.id),
    )
    .map((run) => ({
      id: run.id,
      taskId: run.taskId ?? null,
      conversationInputId: run.conversationInputId ?? null,
      conversationInputClientMessageId: run.conversationInputClientMessageId ?? null,
      conversationInputRevision: run.conversationInputRevision ?? null,
      userMessageId: run.userMessageId,
      assistantMessageId: run.assistantMessageId ?? null,
      userText: run.userText,
      assistantText: run.assistantText ?? null,
      status: run.status,
      startedAt: run.startedAt,
      completedAt: run.completedAt ?? null,
      callCount: run.callCount ?? debugCallCount(run.llmDebug),
      hasTrace: Boolean(run.llmDebug),
      traceError: run.llmDebugError ?? null,
      selected: run.id === activeId,
    }));
}

export function appendTeachingRun(
  runs: ChatTeachingRunRecord[] | undefined,
  run: ChatTeachingRunRecord,
): ChatTeachingRunRecord[] {
  return [...(runs ?? []), run];
}

export function updateTeachingRunById(
  runs: ChatTeachingRunRecord[] | undefined,
  runId: string,
  updater: (run: ChatTeachingRunRecord) => ChatTeachingRunRecord,
): ChatTeachingRunRecord[] {
  return (runs ?? []).map((run) => (run.id === runId ? updater(run) : run));
}

export function updateTeachingRunsByTaskId(
  runs: ChatTeachingRunRecord[] | undefined,
  taskId: string,
  updater: (run: ChatTeachingRunRecord) => ChatTeachingRunRecord,
): ChatTeachingRunRecord[] {
  return (runs ?? []).map((run) => (run.taskId === taskId ? updater(run) : run));
}

export function debugCallCount(debug: TaskLlmDebugResponse | null | undefined): number | null {
  if (!debug) return null;
  if (typeof debug.call_count === "number") return debug.call_count;
  return debug.calls?.length ?? debug.entries?.length ?? null;
}

export function threadPreview(thread: ChatThreadRecord, t: Translate): string {
  const latest = [...thread.messages]
    .reverse()
    .find((message) => message.role === "user" || message.role === "assistant");
  return latest?.text.trim() || t("还没有消息", "No messages yet");
}

export function titleForThreadAfterUserMessage(
  thread: ChatThreadRecord,
  message: ChatMessage,
  t: Translate,
): string {
  const hasPriorUserMessage = thread.messages.some((item) => item.role === "user");
  const defaultTitles = new Set([t("新任务", "New task"), t("未命名任务", "Untitled task")]);
  if (hasPriorUserMessage || !defaultTitles.has(thread.title)) {
    return thread.title;
  }
  const cleaned = message.text.replace(/\s+/g, " ").trim();
  if (!cleaned) {
    return t("附件任务", "Attachment task");
  }
  return cleaned.length > 28 ? `${cleaned.slice(0, 28)}...` : cleaned;
}

export function appendThreadMessages(messages: ChatMessage[], message: ChatMessage): ChatMessage[] {
  return [...messages, message];
}

export function upsertThreadMessage(messages: ChatMessage[], message: ChatMessage): ChatMessage[] {
  const index = messages.findIndex((item) => item.id === message.id);
  if (index < 0) return appendThreadMessages(messages, message);
  const next = [...messages];
  next[index] = message;
  return next;
}

const SELECTED_VOICE_INPUT_DEVICE_STORAGE_KEY =
  appStorageKey("ui.chat.selected_voice_input_device.v1");

export function loadSelectedVoiceInputDeviceId(): string {
  if (typeof window === "undefined") return "";
  try {
    return window.localStorage.getItem(SELECTED_VOICE_INPUT_DEVICE_STORAGE_KEY)?.trim() ?? "";
  } catch {
    return "";
  }
}

export function persistSelectedVoiceInputDeviceId(deviceId: string): void {
  if (typeof window === "undefined") return;
  try {
    if (deviceId) {
      window.localStorage.setItem(SELECTED_VOICE_INPUT_DEVICE_STORAGE_KEY, deviceId);
    } else {
      window.localStorage.removeItem(SELECTED_VOICE_INPUT_DEVICE_STORAGE_KEY);
    }
  } catch {
    // Browser privacy settings may disable local storage; recording still works.
  }
}

export function defaultAttachmentPrompt(
  t: Translate,
  imageCount: number,
  audioCount: number,
  fileCount: number,
): string {
  if (audioCount > 0 && imageCount === 0 && fileCount === 0) {
    return t("请根据这段语音继续对话", "Please continue the conversation based on this voice message");
  }
  if (imageCount > 0 && fileCount === 0 && audioCount === 0) {
    return t("请描述这张图片", "Please describe this image");
  }
  return t("请查看我上传的附件", "Please review the attachments I uploaded");
}

export function defaultAttachmentMessage(
  t: Translate,
  imageCount: number,
  audioCount: number,
  fileCount: number,
): string {
  if (audioCount > 0 && imageCount === 0 && fileCount === 0) {
    return t("发送了一段语音", "Sent a voice message");
  }
  if (imageCount > 0 && fileCount === 0 && audioCount === 0) {
    return t("发送了一张图片", "Sent an image");
  }
  return t("发送了附件", "Sent attachments");
}
