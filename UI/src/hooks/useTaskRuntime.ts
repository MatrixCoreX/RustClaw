import { useEffect, useRef, useState } from "react";

import { followTaskEventStream } from "../lib/task-event-stream";
import { shouldTrackTaskLive } from "../lib/task-lifecycle";
import { appendLiveTaskEvent } from "../lib/task-result";
import { formatUiError } from "../lib/ui-error";
import { newRequestUuid } from "../lib/request-id";
import type {
  ActiveTaskItem,
  ActiveTasksResponse,
  ApprovalScopeGrantListResponse,
  ApprovalScopeGrantView,
  ApiResponse,
  ChannelName,
  ConsolePage,
  SubmitTaskResponse,
  TaskHistoryItem,
  TaskHistoryResponse,
  TaskLlmDebugResponse,
  TaskApprovalDecision,
  TaskQueryResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type TaskSubmitKind = "ask" | "run_skill";

const TERMINAL_TASK_STATUSES = ["succeeded", "failed", "canceled", "timeout"];
const ACTIVE_TASK_PAGE_SIZE = 20;
const TASK_HISTORY_PAGE_SIZE = 20;

function isTaskQueryStatus(status: string): status is TaskQueryResponse["status"] {
  return ["queued", "running", ...TERMINAL_TASK_STATUSES].includes(status);
}

export interface UseTaskRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  apiBase: string;
  uiAuthReady: boolean;
  currentPage: ConsolePage;
  interactionUserId: number | null;
  interactionChatId: number | null;
  activeUserKey: string;
  activeIdentityIds: Record<string, unknown>;
}

export function useTaskRuntime({
  apiFetch,
  t,
  apiBase,
  uiAuthReady,
  currentPage,
  interactionUserId,
  interactionChatId,
  activeUserKey,
  activeIdentityIds,
}: UseTaskRuntimeParams) {
  const [taskId, setTaskId] = useState("");
  const [taskLoading, setTaskLoading] = useState(false);
  const [taskResult, setTaskResult] = useState<TaskQueryResponse | null>(null);
  const [taskError, setTaskError] = useState<string | null>(null);
  const [taskLlmDebug, setTaskLlmDebug] = useState<TaskLlmDebugResponse | null>(null);
  const [taskLlmDebugLoading, setTaskLlmDebugLoading] = useState(false);
  const [taskLlmDebugError, setTaskLlmDebugError] = useState<string | null>(null);
  const [trackingTaskId, setTrackingTaskId] = useState<string | null>(null);
  const [activeTasks, setActiveTasks] = useState<ActiveTaskItem[]>([]);
  const [activeTasksLoading, setActiveTasksLoading] = useState(false);
  const [activeTasksError, setActiveTasksError] = useState<string | null>(null);
  const [activeTasksLastUpdated, setActiveTasksLastUpdated] = useState<number | null>(null);
  const [activeTasksTotal, setActiveTasksTotal] = useState(0);
  const [activeTasksOffset, setActiveTasksOffset] = useState(0);
  const [activeTasksLimit, setActiveTasksLimit] = useState(ACTIVE_TASK_PAGE_SIZE);
  const [taskHistory, setTaskHistory] = useState<TaskHistoryItem[]>([]);
  const [taskHistoryLoading, setTaskHistoryLoading] = useState(false);
  const [taskHistoryLoaded, setTaskHistoryLoaded] = useState(false);
  const [taskHistoryError, setTaskHistoryError] = useState<string | null>(null);
  const [taskHistoryTotal, setTaskHistoryTotal] = useState(0);
  const [taskHistoryOffset, setTaskHistoryOffset] = useState(0);
  const [taskHistoryLimit, setTaskHistoryLimit] = useState(TASK_HISTORY_PAGE_SIZE);
  const taskHistoryRequestEpoch = useRef(0);
  const activeTasksRequestEpoch = useRef(0);
  const taskRequestEpoch = useRef(0);
  const teachingRequestEpoch = useRef(0);
  const grantsRequestEpoch = useRef(0);
  const selectedTaskIdRef = useRef("");
  const scopeKey = JSON.stringify([apiBase, activeUserKey, interactionUserId, interactionChatId, uiAuthReady]);
  const taskScopeRef = useRef({ key: scopeKey });
  // A fresh token also fences an old request across an identity A -> B -> A roundtrip.
  if (taskScopeRef.current.key !== scopeKey) taskScopeRef.current = { key: scopeKey };
  const taskScope = taskScopeRef.current;
  const isCurrentScope = () => taskScopeRef.current === taskScope;
  const beginRead = (epoch: { current: number }) => {
    const requestEpoch = ++epoch.current;
    return () => isCurrentScope() && epoch.current === requestEpoch;
  };
  const deliveryFlightRef = useRef<object | null>(null);
  const [resumeDrafts, setResumeDrafts] = useState<Record<string, string>>({});
  const [resumeSubmittingTaskId, setResumeSubmittingTaskId] = useState<string | null>(null);
  const [resumeTaskMessage, setResumeTaskMessage] = useState<string | null>(null);
  const [resumeTaskError, setResumeTaskError] = useState<string | null>(null);
  const [cancelingTaskId, setCancelingTaskId] = useState<string | null>(null);
  const [cancelTaskMessage, setCancelTaskMessage] = useState<string | null>(null);
  const [cancelTaskError, setCancelTaskError] = useState<string | null>(null);
  const [taskControlSubmittingId, setTaskControlSubmittingId] = useState<string | null>(null);
  const [taskControlMessage, setTaskControlMessage] = useState<string | null>(null);
  const [taskControlError, setTaskControlError] = useState<string | null>(null);
  const [approvalScopeGrants, setApprovalScopeGrants] = useState<ApprovalScopeGrantView[]>([]);
  const [approvalScopeGrantsLoading, setApprovalScopeGrantsLoading] = useState(false);
  const [approvalScopeGrantsError, setApprovalScopeGrantsError] = useState<string | null>(null);
  const [approvalScopeGrantRevokingId, setApprovalScopeGrantRevokingId] = useState<string | null>(null);

  const [interactionKind, setInteractionKind] = useState<TaskSubmitKind>("ask");
  const [interactionChannel, setInteractionChannel] = useState<ChannelName>("ui");
  const [interactionExternalUserId, setInteractionExternalUserId] = useState("");
  const [interactionExternalChatId, setInteractionExternalChatId] = useState("");
  const [interactionAdapter, setInteractionAdapter] = useState("");

  async function reconcileTaskDelivery(taskId: string, operation: "confirm_received" | "retry_unknown" | "retry_failed", snapshotDigest: string) {
    if (!isCurrentScope() || taskControlSubmittingId || deliveryFlightRef.current) return;
    const flight = {};
    deliveryFlightRef.current = flight;
    setTaskControlSubmittingId(`delivery:${taskId}`);
    setTaskControlError(null);
    setTaskControlMessage(null);
    try {
      const response = await apiFetch(`/v1/tasks/${encodeURIComponent(taskId)}/delivery/reconcile`, {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ operation, snapshot_digest: snapshotDigest, decision_id: newRequestUuid(), confirmed_duplicate_risk: operation === "retry_unknown" }),
      });
      const body = await response.json() as ApiResponse<unknown>;
      if (!isCurrentScope()) return;
      if (!response.ok || !body.ok) throw new Error(body.error || "task_delivery_reconciliation_failed");
      setTaskControlMessage(operation === "confirm_received"
        ? t("已记录你的收取确认。", "Your receipt confirmation was recorded.")
        : t("补发已授权，后台将处理可补发部分；请刷新查看投送状态。", "Retry is authorized. The worker will process eligible parts; refresh to inspect delivery status."));
      const refreshed = await fetchTaskById(taskId);
      if (isCurrentScope() && selectedTaskIdRef.current === taskId) setTaskResult(refreshed);
    } catch (error) {
      if (isCurrentScope()) setTaskControlError(formatUiError(error, t, "投送核查未完成，请刷新状态后重试。", "Reconciliation did not complete. Refresh the status before retrying."));
    } finally {
      if (deliveryFlightRef.current === flight) deliveryFlightRef.current = null;
      if (isCurrentScope()) setTaskControlSubmittingId(null);
    }
  }
  const [interactionAskText, setInteractionAskText] = useState("你好，请汇报当前系统状态");
  const [interactionIndependentWorkspace, setInteractionIndependentWorkspace] = useState(false);
  const [interactionSkillName, setInteractionSkillName] = useState("health_check");
  const [interactionSkillArgs, setInteractionSkillArgs] = useState("{\"target\":\"self\"}");
  const [interactionLoading, setInteractionLoading] = useState(false);
  const [interactionError, setInteractionError] = useState<string | null>(null);
  const [interactionSubmittedTaskId, setInteractionSubmittedTaskId] = useState<string | null>(null);

  const fetchTaskById = async (id: string): Promise<TaskQueryResponse> => {
    const res = await apiFetch(`/v1/tasks/${id.trim()}`);
    const body = (await res.json()) as ApiResponse<TaskQueryResponse>;
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(body.error || `task_query_http_${res.status}`);
    }
    return body.data;
  };

  const fetchTaskLlmDebugById = async (
    id: string,
    offset = 0,
  ): Promise<TaskLlmDebugResponse> => {
    const normalizedId = encodeURIComponent(id.trim());
    const res = await apiFetch(
      `/v1/debug/tasks/${normalizedId}?teaching=true&limit=20&offset=${Math.max(0, Math.floor(offset))}`,
    );
    const body = (await res.json()) as ApiResponse<TaskLlmDebugResponse>;
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(body.error || `task_llm_debug_query_http_${res.status}`);
    }
    return body.data;
  };

  const fetchActiveTasks = async (
    silent = false,
    offset = activeTasksOffset,
  ): Promise<ActiveTaskItem[]> => {
    if (!isCurrentScope()) return [];
    const keyAuthenticated = Boolean(activeUserKey.trim());
    if (!keyAuthenticated && (interactionUserId == null || interactionChatId == null)) {
      if (!silent) {
        setActiveTasksError(t("本地身份还没有加载完成。", "Local identity is not loaded yet."));
      }
      return [];
    }
    const isCurrentRequest = beginRead(activeTasksRequestEpoch);
    if (!silent) {
      setActiveTasksLoading(true);
      setActiveTasksError(null);
    }
    try {
      const res = await apiFetch(`/v1/tasks/active`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          user_id: interactionUserId ?? 0,
          chat_id: interactionChatId ?? 0,
          limit: ACTIVE_TASK_PAGE_SIZE,
          offset: Math.max(0, Math.floor(offset)),
        }),
      });
      const body = (await res.json()) as ApiResponse<ActiveTasksResponse>;
      if (!isCurrentRequest()) return [];
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `active_tasks_fetch_http_${res.status}`);
      }
      const tasks = body.data.tasks ?? [];
      setActiveTasks(tasks);
      setActiveTasksTotal(body.data.total ?? tasks.length);
      setActiveTasksOffset(body.data.offset ?? 0);
      setActiveTasksLimit(body.data.limit ?? ACTIVE_TASK_PAGE_SIZE);
      setActiveTasksError(null);
      setActiveTasksLastUpdated(Date.now());
      return tasks;
    } catch (err) {
      if (!isCurrentRequest()) return [];
      const message = formatUiError(err, t, "正在处理的任务暂时无法读取。", "Active tasks are temporarily unavailable.");
      setActiveTasksError(message);
      return [];
    } finally {
      if (isCurrentRequest()) {
        setActiveTasksLoading(false);
      }
    }
  };

  const fetchTaskHistory = async (offset = 0): Promise<TaskHistoryItem[]> => {
    if (!isCurrentScope()) return [];
    const keyAuthenticated = Boolean(activeUserKey.trim());
    if (!keyAuthenticated && (interactionUserId == null || interactionChatId == null)) {
      setTaskHistoryError(t("本地身份还没有加载完成。", "Local identity is not loaded yet."));
      return [];
    }
    const normalizedOffset = Math.max(0, Math.floor(offset));
    const isCurrentRequest = beginRead(taskHistoryRequestEpoch);
    setTaskHistoryLoading(true);
    setTaskHistoryError(null);
    try {
      const res = await apiFetch("/v1/tasks/history", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          user_id: interactionUserId ?? 0,
          chat_id: interactionChatId ?? 0,
          limit: TASK_HISTORY_PAGE_SIZE,
          offset: normalizedOffset,
        }),
      });
      const body = (await res.json()) as ApiResponse<TaskHistoryResponse>;
      if (!isCurrentRequest()) return [];
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `task_history_fetch_http_${res.status}`);
      }
      setTaskHistory(body.data.tasks ?? []);
      setTaskHistoryTotal(body.data.total ?? 0);
      setTaskHistoryOffset(body.data.offset ?? normalizedOffset);
      setTaskHistoryLimit(body.data.limit ?? TASK_HISTORY_PAGE_SIZE);
      setTaskHistoryLoaded(true);
      return body.data.tasks ?? [];
    } catch (error) {
      if (!isCurrentRequest()) return [];
      const message = formatUiError(error, t, "无法读取任务历史。", "Could not load task history.");
      setTaskHistoryError(message);
      return [];
    } finally {
      if (isCurrentRequest()) {
        setTaskHistoryLoading(false);
      }
    }
  };

  const queryTaskById = async (id: string, resetBeforeLoad = true): Promise<TaskQueryResponse | null> => {
    const normalizedId = id.trim();
    if (!isCurrentScope() || !normalizedId) return null;
    if (resetBeforeLoad) {
      selectedTaskIdRef.current = normalizedId;
      teachingRequestEpoch.current += 1;
      setTrackingTaskId(null);
      setTaskLoading(true);
      setTaskError(null);
      setTaskResult(null);
      setTaskLlmDebug(null);
      setTaskLlmDebugError(null);
      setTaskLlmDebugLoading(false);
    } else if (selectedTaskIdRef.current !== normalizedId) {
      return null;
    }
    const isCurrentRequest = beginRead(taskRequestEpoch);
    try {
      const result = await fetchTaskById(normalizedId);
      if (!isCurrentRequest()) return null;
      setTaskResult(result);
      setTaskError(null);
      return result;
    } catch (err) {
      if (!isCurrentRequest()) return null;
      const message = formatUiError(err, t, "任务详情暂时无法读取。", "Task details are temporarily unavailable.");
      setTaskError(message);
      return null;
    } finally {
      if (isCurrentRequest()) {
        setTaskLoading(false);
      }
    }
  };

  const queryTask = async () => {
    if (!taskId.trim()) return;
    await queryTaskById(taskId);
  };

  const queryTaskLlmDebug = async (id?: string, offset = 0) => {
    const targetTaskId = (id ?? taskId).trim();
    if (!isCurrentScope() || !targetTaskId) return null;
    const isCurrentRequest = beginRead(teachingRequestEpoch);
    setTaskLlmDebugLoading(true);
    setTaskLlmDebugError(null);
    try {
      const result = await fetchTaskLlmDebugById(targetTaskId, offset);
      if (!isCurrentRequest()) return null;
      setTaskLlmDebug(result);
      return result;
    } catch (err) {
      if (!isCurrentRequest()) return null;
      const message = formatUiError(err, t, "教学过程暂时无法读取。", "The teaching trace is temporarily unavailable.");
      setTaskLlmDebugError(message);
      setTaskLlmDebug(null);
      return null;
    } finally {
      if (isCurrentRequest()) setTaskLlmDebugLoading(false);
    }
  };

  const markTaskSubmitted = (submittedTaskId: string) => {
    if (!isCurrentScope()) return;
    selectedTaskIdRef.current = submittedTaskId;
    taskRequestEpoch.current += 1;
    teachingRequestEpoch.current += 1;
    setTaskId(submittedTaskId);
    setTrackingTaskId(submittedTaskId);
    setTaskResult(null);
    setTaskError(null);
    setTaskLlmDebug(null);
    setTaskLlmDebugError(null);
    setTaskLoading(false);
    setTaskLlmDebugLoading(false);
  };

  const recordTaskResult = (submittedTaskId: string, finalResult: TaskQueryResponse) => {
    if (!isCurrentScope() || selectedTaskIdRef.current !== submittedTaskId) return;
    taskRequestEpoch.current += 1;
    setTaskResult(finalResult);
    setTaskLoading(false);
    setTrackingTaskId(
      shouldTrackTaskLive(finalResult.status, finalResult.execution_state, finalResult.lifecycle)
        ? submittedTaskId
        : null,
    );
  };

  const setResumeDraftValue = (resumeTaskId: string, value: string) => {
    setResumeDrafts((prev) => ({
      ...prev,
      [resumeTaskId]: value,
    }));
  };

  const submitResumeForTask = async (resumeTaskId: string) => {
    if (!isCurrentScope()) return;
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
      if (!isCurrentScope()) return;
      if (!res.ok || !resp.ok || !resp.data?.task_id) {
        throw new Error(resp.error || `task_resume_submit_http_${res.status}`);
      }

      setResumeDrafts((prev) => {
        const next = { ...prev };
        delete next[resumeTaskId];
        return next;
      });
      setResumeTaskMessage(t("已提交继续执行请求。", "Resume request submitted."));
      setInteractionSubmittedTaskId(resp.data.task_id);
      markTaskSubmitted(resp.data.task_id);
      void fetchActiveTasks(true);
    } catch (err) {
      if (!isCurrentScope()) return;
      const message = formatUiError(err, t, "任务继续执行失败，请稍后重试。", "The task could not resume. Try again shortly.");
      setResumeTaskError(message);
    } finally {
      if (isCurrentScope()) setResumeSubmittingTaskId(null);
    }
  };

  const cancelActiveTask = async (item: ActiveTaskItem) => {
    if (!isCurrentScope()) return;
    setCancelingTaskId(item.task_id);
    setCancelTaskMessage(null);
    setCancelTaskError(null);
    try {
      const res = await apiFetch(`/v1/tasks/cancel-by-task-id`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          task_id: item.task_id,
          idempotency_key: `ui-cancel-${newRequestUuid()}`,
        }),
      });
      const body = (await res.json()) as ApiResponse<{ canceled?: number }>;
      if (!isCurrentScope()) return;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `task_cancel_http_${res.status}`);
      }
      setCancelTaskMessage(t("任务取消请求已提交。", "Task cancel request submitted."));
      await fetchActiveTasks(true);
      if (taskResult?.task_id === item.task_id) {
        void queryTaskById(item.task_id, false);
      }
    } catch (err) {
      if (!isCurrentScope()) return;
      const message = formatUiError(err, t, "任务取消失败，请稍后重试。", "The task could not be cancelled. Try again shortly.");
      setCancelTaskError(message);
    } finally {
      if (isCurrentScope()) setCancelingTaskId(null);
    }
  };

  const controlTaskById = async (
    control: "steer" | "pause" | "resume",
    controlTaskId: string,
    userMessage?: string,
  ) => {
    const normalizedTaskId = controlTaskId.trim();
    if (!isCurrentScope() || !normalizedTaskId) return;
    setTaskControlSubmittingId(`${control}:${normalizedTaskId}`);
    setTaskControlMessage(null);
    setTaskControlError(null);
    try {
      const path =
        control === "steer"
          ? "/v1/tasks/steer-by-task-id"
          : control === "pause"
            ? "/v1/tasks/pause-by-task-id"
            : "/v1/tasks/resume-by-task-id";
      const payload =
        control === "steer"
          ? {
              task_id: normalizedTaskId,
              user_message: userMessage?.trim(),
              idempotency_key: `ui-steer-${newRequestUuid()}`,
            }
          : control === "pause"
            ? {
                task_id: normalizedTaskId,
                idempotency_key: `ui-pause-${newRequestUuid()}`,
              }
            : {
                task_id: normalizedTaskId,
                idempotency_key: `ui-resume-${newRequestUuid()}`,
              };
      const res = await apiFetch(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const body = (await res.json()) as ApiResponse<{ status?: string; task_id?: string }>;
      if (!isCurrentScope()) return;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `task_control_http_${res.status}`);
      }
      setTaskControlMessage(
        control === "steer"
          ? t("新要求已送达，将在安全步骤边界应用。", "The new direction was accepted and will apply at a safe step boundary.")
          : control === "pause"
          ? t("任务已暂停，需要你明确恢复后才会继续。", "Task paused and will continue only after you explicitly resume it.")
          : t("任务恢复请求已提交。", "Task resume request submitted."),
      );
      if (control === "steer") {
        setResumeDrafts((prev) => ({ ...prev, [normalizedTaskId]: "" }));
      }
      await fetchActiveTasks(true);
      if (taskResult?.task_id === normalizedTaskId) {
        void queryTaskById(normalizedTaskId, false);
      }
    } catch (err) {
      if (!isCurrentScope()) return;
      const message = formatUiError(err, t, "任务控制操作未完成，请稍后重试。", "The task control action did not complete. Try again shortly.");
      setTaskControlError(message);
    } finally {
      if (isCurrentScope()) setTaskControlSubmittingId(null);
    }
  };

  const controlSubagentById = async (
    control: "steer" | "pause" | "resume" | "stop" | "stop_all" | "close",
    parentTaskId: string,
    childTaskId?: string,
    userMessage?: string,
  ) => {
    const normalizedParentId = parentTaskId.trim();
    const normalizedChildId = childTaskId?.trim() ?? "";
    if (!isCurrentScope() || !normalizedParentId || (control !== "stop_all" && !normalizedChildId)) return;
    const targetId = normalizedChildId || normalizedParentId;
    setTaskControlSubmittingId(`subagent-${control}:${targetId}`);
    setTaskControlMessage(null);
    setTaskControlError(null);
    try {
      let path = "/v1/tasks/resume-by-task-id";
      const idempotencyKey = `ui-subagent-${control}-${newRequestUuid()}`;
      let payload: Record<string, unknown> = {
        task_id: normalizedChildId,
        idempotency_key: idempotencyKey,
      };
      if (control === "steer") {
        payload = {
          task_id: normalizedChildId,
          resume_reason: "subagent_steered",
          user_message: userMessage?.trim(),
          idempotency_key: idempotencyKey,
        };
      } else if (control === "pause") {
        path = "/v1/tasks/pause-by-task-id";
        payload = { task_id: normalizedChildId, idempotency_key: idempotencyKey };
      } else if (control === "resume") {
        payload = { task_id: normalizedChildId, resume_reason: "subagent_resumed", idempotency_key: idempotencyKey };
      } else if (control === "stop") {
        path = "/v1/tasks/cancel-by-task-id";
        payload = { task_id: normalizedChildId, idempotency_key: idempotencyKey };
      } else if (control === "stop_all") {
        path = "/v1/tasks/stop-child-tasks-by-parent";
        payload = { parent_task_id: normalizedParentId };
      } else if (control === "close") {
        path = "/v1/tasks/close-child-by-task-id";
        payload = {
          parent_task_id: normalizedParentId,
          child_task_id: normalizedChildId,
        };
      }
      const res = await apiFetch(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!isCurrentScope()) return;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `subagent_control_http_${res.status}`);
      }
      setTaskControlMessage(
        control === "stop_all"
          ? t("已停止这个任务仍在运行的并行项。", "Active parallel items for this task were stopped.")
          : control === "close"
            ? t("已从活动视图关闭该并行项，历史记录仍会保留。", "The item was closed from the active view and its history was kept.")
            : t("并行任务控制已提交。", "Parallel task control submitted."),
      );
      await fetchActiveTasks(true);
      await queryTaskById(normalizedParentId, false);
    } catch (err) {
      if (!isCurrentScope()) return;
      const message = formatUiError(err, t, "子任务控制操作未完成，请稍后重试。", "The subtask control action did not complete. Try again shortly.");
      setTaskControlError(message);
    } finally {
      if (isCurrentScope()) setTaskControlSubmittingId(null);
    }
  };

  const decideTaskApprovalById = async (
    controlTaskId: string,
    approvalRequestId: string,
    approvalDecision: TaskApprovalDecision,
  ) => {
    const normalizedTaskId = controlTaskId.trim();
    const normalizedRequestId = approvalRequestId.trim();
    if (!isCurrentScope() || !normalizedTaskId || !normalizedRequestId) return;
    setTaskControlSubmittingId(`${approvalDecision}:${normalizedTaskId}`);
    setTaskControlMessage(null);
    setTaskControlError(null);
    try {
      const res = await apiFetch("/v1/tasks/resume-by-task-id", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          task_id: normalizedTaskId,
          approval_request_id: normalizedRequestId,
          approval_decision: approvalDecision,
          idempotency_key: `ui-approval-${newRequestUuid()}`,
        }),
      });
      const body = (await res.json()) as ApiResponse<{ status?: string; task_id?: string }>;
      if (!isCurrentScope()) return;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `task_approval_http_${res.status}`);
      }
      setTaskControlMessage(
        approvalDecision === "deny"
          ? t("已拒绝这一次操作，任务不会继续执行。", "This action was denied and the task will not continue.")
          : approvalDecision === "always_for_scope"
            ? t(
                "已为当前会话的相同操作和资源授权，任务正在重新排队。",
                "The same operation and resources are approved for this session, and the task is queued again.",
              )
            : t("已授权这一次操作，任务正在重新排队。", "This action was approved once and the task is queued again."),
      );
      if (approvalDecision === "always_for_scope") {
        void fetchApprovalScopeGrants(true);
      }
      await fetchActiveTasks(true);
      void queryTaskById(normalizedTaskId, false);
    } catch (err) {
      if (!isCurrentScope()) return;
      const message = formatUiError(err, t, "任务确认操作未完成，请稍后重试。", "The task approval action did not complete. Try again shortly.");
      setTaskControlError(message);
    } finally {
      if (isCurrentScope()) setTaskControlSubmittingId(null);
    }
  };

  const fetchApprovalScopeGrants = async (silent = false): Promise<ApprovalScopeGrantView[]> => {
    if (!isCurrentScope()) return [];
    const isCurrentRequest = beginRead(grantsRequestEpoch);
    if (!silent) {
      setApprovalScopeGrantsLoading(true);
      setApprovalScopeGrantsError(null);
    }
    try {
      const res = await apiFetch("/v1/tasks/approval-grants");
      const body = (await res.json()) as ApiResponse<ApprovalScopeGrantListResponse>;
      if (!isCurrentRequest()) return [];
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `approval_scope_grants_fetch_http_${res.status}`);
      }
      const grants = body.data.grants ?? [];
      setApprovalScopeGrants(grants);
      setApprovalScopeGrantsError(null);
      return grants;
    } catch (err) {
      if (!isCurrentRequest()) return [];
      const message = formatUiError(err, t, "确认授权范围暂时无法读取。", "Approval scopes are temporarily unavailable.");
      setApprovalScopeGrantsError(message);
      return [];
    } finally {
      if (isCurrentRequest()) {
        setApprovalScopeGrantsLoading(false);
      }
    }
  };

  const revokeApprovalScopeGrant = async (grantId: string) => {
    const normalizedGrantId = grantId.trim();
    if (!isCurrentScope() || !normalizedGrantId) return;
    setApprovalScopeGrantRevokingId(normalizedGrantId);
    setApprovalScopeGrantsError(null);
    try {
      const res = await apiFetch("/v1/tasks/approval-grants/revoke", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ grant_id: normalizedGrantId }),
      });
      const body = (await res.json()) as ApiResponse<{ status?: string; grant_id?: string }>;
      if (!isCurrentScope()) return;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `approval_scope_grant_revoke_http_${res.status}`);
      }
      await fetchApprovalScopeGrants(true);
    } catch (err) {
      if (!isCurrentScope()) return;
      const message = formatUiError(err, t, "确认授权撤销失败，请稍后重试。", "The approval grant could not be revoked. Try again shortly.");
      setApprovalScopeGrantsError(message);
    } finally {
      if (isCurrentScope()) setApprovalScopeGrantRevokingId(null);
    }
  };

  const controlTaskGoalById = async (
    operation: "edit" | "clear",
    controlTaskId: string,
    goal?: Record<string, unknown>,
  ) => {
    const normalizedTaskId = controlTaskId.trim();
    if (!isCurrentScope() || !normalizedTaskId) return;
    setTaskControlSubmittingId(`goal-${operation}:${normalizedTaskId}`);
    setTaskControlMessage(null);
    setTaskControlError(null);
    try {
      const res = await apiFetch("/v1/tasks/goal-by-task-id", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          task_id: normalizedTaskId,
          operation,
          ...(operation === "edit" ? { goal: goal ?? {} } : {}),
        }),
      });
      const body = (await res.json()) as ApiResponse<{ status?: string; task_id?: string }>;
      if (!isCurrentScope()) return;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `task_goal_control_http_${res.status}`);
      }
      setTaskControlMessage(
        operation === "edit"
          ? t("目标已更新。", "Goal updated.")
          : t("目标已清除。", "Goal cleared."),
      );
      await fetchActiveTasks(true);
      if (taskResult?.task_id === normalizedTaskId) {
        void queryTaskById(normalizedTaskId, false);
      }
    } catch (err) {
      if (!isCurrentScope()) return;
      const message = formatUiError(err, t, "目标任务控制操作未完成，请稍后重试。", "The goal control action did not complete. Try again shortly.");
      setTaskControlError(message);
    } finally {
      if (isCurrentScope()) setTaskControlSubmittingId(null);
    }
  };

  const submitInteractionTask = async () => {
    if (!isCurrentScope()) return;
    setInteractionLoading(true);
    setInteractionError(null);
    setInteractionSubmittedTaskId(null);
    try {
      let payload: Record<string, unknown>;
      if (interactionKind === "ask") {
        payload = {
          text: interactionAskText.trim(),
          ...(interactionIndependentWorkspace
            ? { execution_workspace: { mode: "independent" } }
            : {}),
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
      if (!isCurrentScope()) return;
      if (!res.ok || !resp.ok || !resp.data?.task_id) {
        throw new Error(resp.error || `task_submit_http_${res.status}`);
      }

      setInteractionSubmittedTaskId(resp.data.task_id);
      markTaskSubmitted(resp.data.task_id);
      void fetchActiveTasks(true);
    } catch (err) {
      if (!isCurrentScope()) return;
      const message = formatUiError(err, t, "任务提交失败，请稍后重试。", "The task could not be submitted. Try again shortly.");
      setInteractionError(message);
    } finally {
      if (isCurrentScope()) setInteractionLoading(false);
    }
  };

  const viewTask = async (taskIdToView: string) => {
    if (!isCurrentScope()) return null;
    setTaskId(taskIdToView);
    setTaskLlmDebug(null);
    setTaskLlmDebugError(null);
    const result = await queryTaskById(taskIdToView);
    if (result && isCurrentScope() && selectedTaskIdRef.current === result.task_id) {
      setTrackingTaskId(shouldTrackTaskLive(result.status, result.execution_state, result.lifecycle)
        ? result.task_id : null);
    }
    return result;
  };

  useEffect(() => {
    selectedTaskIdRef.current = "";
    taskRequestEpoch.current += 1;
    teachingRequestEpoch.current += 1;
    activeTasksRequestEpoch.current += 1;
    grantsRequestEpoch.current += 1;
    setTaskId("");
    setTaskLoading(false);
    setTaskResult(null);
    setTaskError(null);
    setTaskLlmDebug(null);
    setTaskLlmDebugLoading(false);
    setTaskLlmDebugError(null);
    setTrackingTaskId(null);
    setActiveTasks([]);
    setActiveTasksLoading(false);
    setActiveTasksError(null);
    setActiveTasksLastUpdated(null);
    setActiveTasksTotal(0);
    setActiveTasksOffset(0);
    setActiveTasksLimit(ACTIVE_TASK_PAGE_SIZE);
    taskHistoryRequestEpoch.current += 1;
    setTaskHistory([]);
    setTaskHistoryLoading(false);
    setTaskHistoryLoaded(false);
    setTaskHistoryError(null);
    setTaskHistoryTotal(0);
    setTaskHistoryOffset(0);
    setTaskHistoryLimit(TASK_HISTORY_PAGE_SIZE);
    setTaskControlSubmittingId(null);
    setTaskControlMessage(null);
    setTaskControlError(null);
    setApprovalScopeGrants([]);
    setApprovalScopeGrantsLoading(false);
    setApprovalScopeGrantsError(null);
    setApprovalScopeGrantRevokingId(null);
    setResumeDrafts({});
    setResumeSubmittingTaskId(null);
    setResumeTaskMessage(null);
    setResumeTaskError(null);
    setCancelingTaskId(null);
    setCancelTaskMessage(null);
    setCancelTaskError(null);
    setInteractionLoading(false);
    setInteractionError(null);
    setInteractionSubmittedTaskId(null);
    deliveryFlightRef.current = null;
  }, [taskScope]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!trackingTaskId) return;
    const controller = new AbortController();
    const isCurrentStream = () => !controller.signal.aborted && isCurrentScope()
      && selectedTaskIdRef.current === trackingTaskId;
    void followTaskEventStream(
      apiFetch,
      trackingTaskId,
      (event) => {
        if (!isCurrentStream()) return;
        setTaskResult((current) => {
          if (!isCurrentStream()) return current;
          const withEvent = appendLiveTaskEvent(current, trackingTaskId, event);
          if (event.event_kind !== "task_state") return withEvent;
          const status = event.payload?.status;
          if (typeof status !== "string" || !isTaskQueryStatus(status)) return withEvent;
          return {
            ...withEvent,
            status,
            execution_state:
              typeof event.payload?.execution_state === "string"
                ? event.payload.execution_state
                : withEvent.execution_state,
            lifecycle:
              event.payload?.lifecycle && typeof event.payload.lifecycle === "object"
                ? (event.payload.lifecycle as TaskQueryResponse["lifecycle"])
                : withEvent.lifecycle,
          };
        });
      },
      controller.signal,
    )
      .then(async () => {
        if (!isCurrentStream()) return;
        const result = await fetchTaskById(trackingTaskId);
        if (!isCurrentStream()) return;
        setTaskResult(result);
        setTaskError(null);
        if (!shouldTrackTaskLive(result.status, result.execution_state, result.lifecycle)) {
          setTrackingTaskId(null);
        }
      })
      .catch((error) => {
        if (!isCurrentStream()) return;
        setTaskError(formatUiError(error, t, "任务进度连接已中断，请刷新后重试。", "The task progress connection was interrupted. Refresh and try again."));
      });
    return () => controller.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [trackingTaskId, apiBase, uiAuthReady, taskScope]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (currentPage !== "tasks") return;
    if (
      !activeUserKey.trim() &&
      (interactionUserId == null || interactionChatId == null)
    ) {
      return;
    }
    let refreshPending = false;
    const refreshWhenVisible = () => {
      if (document.visibilityState !== "visible" || refreshPending) return;
      refreshPending = true;
      void Promise.all([
        fetchActiveTasks(true, activeTasksOffset), fetchApprovalScopeGrants(true),
      ]).finally(() => { refreshPending = false; });
    };
    refreshWhenVisible();
    const interval = window.setInterval(() => {
      refreshWhenVisible();
    }, 5000);
    document.addEventListener("visibilitychange", refreshWhenVisible);
    return () => {
      window.clearInterval(interval);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    currentPage,
    apiBase,
    uiAuthReady,
    activeUserKey,
    interactionUserId,
    interactionChatId,
    activeTasksOffset,
  ]);

  return {
    reconcileTaskDelivery,
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
    activeTasksTotal,
    activeTasksOffset,
    activeTasksLimit,
    taskHistory,
    taskHistoryLoading,
    taskHistoryLoaded,
    taskHistoryError,
    taskHistoryTotal,
    taskHistoryOffset,
    taskHistoryLimit,
    resumeDrafts,
    resumeSubmittingTaskId,
    resumeTaskMessage,
    resumeTaskError,
    cancelingTaskId,
    cancelTaskMessage,
    cancelTaskError,
    taskControlSubmittingId,
    taskControlMessage,
    taskControlError,
    approvalScopeGrants,
    approvalScopeGrantsLoading,
    approvalScopeGrantsError,
    approvalScopeGrantRevokingId,
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
    interactionIndependentWorkspace,
    setInteractionIndependentWorkspace,
    interactionSkillName,
    setInteractionSkillName,
    interactionSkillArgs,
    setInteractionSkillArgs,
    interactionLoading,
    interactionError,
    interactionSubmittedTaskId,
    fetchTaskById,
    fetchTaskLlmDebugById,
    fetchActiveTasks,
    fetchTaskHistory,
    queryTaskById,
    queryTask,
    queryTaskLlmDebug,
    viewTask,
    setResumeDraftValue,
    submitResumeForTask,
    cancelActiveTask,
    controlTaskById,
    controlSubagentById,
    decideTaskApprovalById,
    fetchApprovalScopeGrants,
    revokeApprovalScopeGrant,
    controlTaskGoalById,
    submitInteractionTask,
    markTaskSubmitted,
    recordTaskResult,
  };
}
