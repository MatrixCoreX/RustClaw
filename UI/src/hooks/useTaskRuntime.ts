import { useEffect, useState } from "react";

import { followTaskEventStream } from "../lib/task-event-stream";
import type {
  ActiveTaskItem,
  ActiveTasksResponse,
  ApprovalScopeGrantListResponse,
  ApprovalScopeGrantView,
  ApiResponse,
  ChannelName,
  ConsolePage,
  SubmitTaskResponse,
  TaskLlmDebugResponse,
  TaskApprovalDecision,
  TaskQueryResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type TaskSubmitKind = "ask" | "run_skill";

const TERMINAL_TASK_STATUSES = ["succeeded", "failed", "canceled", "timeout"];

function isTerminalTaskStatus(status: string): boolean {
  return TERMINAL_TASK_STATUSES.includes(status);
}

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
  const [resumeDrafts, setResumeDrafts] = useState<Record<string, string>>({});
  const [resumeSubmittingTaskId, setResumeSubmittingTaskId] = useState<string | null>(null);
  const [resumeTaskMessage, setResumeTaskMessage] = useState<string | null>(null);
  const [resumeTaskError, setResumeTaskError] = useState<string | null>(null);
  const [cancelingTaskIndex, setCancelingTaskIndex] = useState<number | null>(null);
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
  const [interactionAskText, setInteractionAskText] = useState("你好，请汇报当前系统状态");
  const [interactionSkillName, setInteractionSkillName] = useState("health_check");
  const [interactionSkillArgs, setInteractionSkillArgs] = useState("{\"target\":\"self\"}");
  const [interactionLoading, setInteractionLoading] = useState(false);
  const [interactionError, setInteractionError] = useState<string | null>(null);
  const [interactionSubmittedTaskId, setInteractionSubmittedTaskId] = useState<string | null>(null);

  const fetchTaskById = async (id: string): Promise<TaskQueryResponse> => {
    const res = await apiFetch(`/v1/tasks/${id.trim()}`);
    const body = (await res.json()) as ApiResponse<TaskQueryResponse>;
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(body.error || `task query failed (${res.status})`);
    }
    return body.data;
  };

  const fetchTaskLlmDebugById = async (id: string): Promise<TaskLlmDebugResponse> => {
    const normalizedId = encodeURIComponent(id.trim());
    const res = await apiFetch(`/v1/debug/tasks/${normalizedId}`);
    const body = (await res.json()) as ApiResponse<TaskLlmDebugResponse>;
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(body.error || `task llm debug query failed (${res.status})`);
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
        throw new Error(body.error || `active tasks fetch failed (${res.status})`);
      }
      const tasks = body.data.tasks ?? [];
      setActiveTasks(tasks);
      setActiveTasksError(null);
      setActiveTasksLastUpdated(Date.now());
      return tasks;
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
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
      setTaskLlmDebug(null);
      setTaskLlmDebugError(null);
    }
    try {
      const result = await fetchTaskById(id);
      setTaskResult(result);
      setTaskError(null);
      return result;
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
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

  const queryTaskLlmDebug = async (id?: string) => {
    const targetTaskId = (id ?? taskId).trim();
    if (!targetTaskId) return null;
    setTaskLlmDebugLoading(true);
    setTaskLlmDebugError(null);
    try {
      const result = await fetchTaskLlmDebugById(targetTaskId);
      setTaskLlmDebug(result);
      return result;
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setTaskLlmDebugError(message);
      setTaskLlmDebug(null);
      return null;
    } finally {
      setTaskLlmDebugLoading(false);
    }
  };

  const markTaskSubmitted = (submittedTaskId: string) => {
    setTaskId(submittedTaskId);
    setTrackingTaskId(submittedTaskId);
    setTaskResult(null);
    setTaskError(null);
    setTaskLlmDebug(null);
    setTaskLlmDebugError(null);
  };

  const recordTaskResult = (submittedTaskId: string, finalResult: TaskQueryResponse) => {
    setTaskResult(finalResult);
    setTrackingTaskId(isTerminalTaskStatus(finalResult.status) ? null : submittedTaskId);
  };

  const setResumeDraftValue = (resumeTaskId: string, value: string) => {
    setResumeDrafts((prev) => ({
      ...prev,
      [resumeTaskId]: value,
    }));
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
      markTaskSubmitted(resp.data.task_id);
      void fetchActiveTasks(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
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
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setCancelTaskError(message);
    } finally {
      setCancelingTaskIndex(null);
    }
  };

  const controlTaskById = async (control: "pause" | "resume", controlTaskId: string) => {
    const normalizedTaskId = controlTaskId.trim();
    if (!normalizedTaskId) return;
    setTaskControlSubmittingId(`${control}:${normalizedTaskId}`);
    setTaskControlMessage(null);
    setTaskControlError(null);
    try {
      const path = control === "pause" ? "/v1/tasks/pause-by-task-id" : "/v1/tasks/resume-by-task-id";
      const payload =
        control === "pause"
          ? { task_id: normalizedTaskId, pause_seconds: 3600 }
          : { task_id: normalizedTaskId };
      const res = await apiFetch(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const body = (await res.json()) as ApiResponse<{ status?: string; task_id?: string }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `task control failed (${res.status})`);
      }
      setTaskControlMessage(
        control === "pause"
          ? t("任务已暂停，会在稍后再继续。", "Task paused and will continue later.")
          : t("任务恢复请求已提交。", "Task resume request submitted."),
      );
      await fetchActiveTasks(true);
      if (taskResult?.task_id === normalizedTaskId) {
        void queryTaskById(normalizedTaskId, false);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setTaskControlError(message);
    } finally {
      setTaskControlSubmittingId(null);
    }
  };

  const decideTaskApprovalById = async (
    controlTaskId: string,
    approvalRequestId: string,
    approvalDecision: TaskApprovalDecision,
  ) => {
    const normalizedTaskId = controlTaskId.trim();
    const normalizedRequestId = approvalRequestId.trim();
    if (!normalizedTaskId || !normalizedRequestId) return;
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
        }),
      });
      const body = (await res.json()) as ApiResponse<{ status?: string; task_id?: string }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `task approval failed (${res.status})`);
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
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setTaskControlError(message);
    } finally {
      setTaskControlSubmittingId(null);
    }
  };

  const fetchApprovalScopeGrants = async (silent = false): Promise<ApprovalScopeGrantView[]> => {
    if (!silent) {
      setApprovalScopeGrantsLoading(true);
      setApprovalScopeGrantsError(null);
    }
    try {
      const res = await apiFetch("/v1/tasks/approval-grants");
      const body = (await res.json()) as ApiResponse<ApprovalScopeGrantListResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `approval scope grants fetch failed (${res.status})`);
      }
      const grants = body.data.grants ?? [];
      setApprovalScopeGrants(grants);
      setApprovalScopeGrantsError(null);
      return grants;
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setApprovalScopeGrantsError(message);
      return [];
    } finally {
      if (!silent) {
        setApprovalScopeGrantsLoading(false);
      }
    }
  };

  const revokeApprovalScopeGrant = async (grantId: string) => {
    const normalizedGrantId = grantId.trim();
    if (!normalizedGrantId) return;
    setApprovalScopeGrantRevokingId(normalizedGrantId);
    setApprovalScopeGrantsError(null);
    try {
      const res = await apiFetch("/v1/tasks/approval-grants/revoke", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ grant_id: normalizedGrantId }),
      });
      const body = (await res.json()) as ApiResponse<{ status?: string; grant_id?: string }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `approval scope grant revoke failed (${res.status})`);
      }
      await fetchApprovalScopeGrants(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setApprovalScopeGrantsError(message);
    } finally {
      setApprovalScopeGrantRevokingId(null);
    }
  };

  const controlTaskGoalById = async (
    operation: "edit" | "clear",
    controlTaskId: string,
    goal?: Record<string, unknown>,
  ) => {
    const normalizedTaskId = controlTaskId.trim();
    if (!normalizedTaskId) return;
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
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `task goal control failed (${res.status})`);
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
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setTaskControlError(message);
    } finally {
      setTaskControlSubmittingId(null);
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
        throw new Error(resp.error || `task submit failed (${res.status})`);
      }

      setInteractionSubmittedTaskId(resp.data.task_id);
      markTaskSubmitted(resp.data.task_id);
      void fetchActiveTasks(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setInteractionError(message);
    } finally {
      setInteractionLoading(false);
    }
  };

  const viewTask = async (taskIdToView: string) => {
    setTaskId(taskIdToView);
    setTaskLlmDebug(null);
    setTaskLlmDebugError(null);
    return queryTaskById(taskIdToView);
  };

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!trackingTaskId) return;
    const controller = new AbortController();
    void followTaskEventStream(
      apiFetch,
      trackingTaskId,
      (event) => {
        if (event.event_kind !== "task_state") return;
        const status = event.payload?.status;
        if (typeof status !== "string" || !isTaskQueryStatus(status)) return;
        setTaskResult((current) => ({
          task_id: trackingTaskId,
          status,
          execution_state:
            typeof event.payload?.execution_state === "string"
              ? event.payload.execution_state
              : current?.execution_state,
          result_json: current?.result_json ?? null,
          error_text: current?.error_text ?? null,
          lifecycle:
            event.payload?.lifecycle && typeof event.payload.lifecycle === "object"
              ? (event.payload.lifecycle as TaskQueryResponse["lifecycle"])
              : current?.lifecycle,
        }));
      },
      controller.signal,
    )
      .then(async () => {
        if (controller.signal.aborted) return;
        const result = await fetchTaskById(trackingTaskId);
        setTaskResult(result);
        setTaskError(null);
        if (isTerminalTaskStatus(result.status)) setTrackingTaskId(null);
      })
      .catch((error) => {
        if (controller.signal.aborted) return;
        setTaskError(error instanceof Error ? error.message : "task_event_stream_failed");
      });
    return () => controller.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [trackingTaskId, apiBase, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (currentPage !== "tasks") return;
    if (interactionUserId == null || interactionChatId == null) return;
    void fetchActiveTasks(true);
    void fetchApprovalScopeGrants(true);
    const interval = window.setInterval(() => {
      void fetchActiveTasks(true);
    }, 5000);
    return () => window.clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPage, apiBase, uiAuthReady, interactionUserId, interactionChatId]);

  return {
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
    queryTaskById,
    queryTask,
    queryTaskLlmDebug,
    viewTask,
    setResumeDraftValue,
    submitResumeForTask,
    cancelActiveTask,
    controlTaskById,
    decideTaskApprovalById,
    fetchApprovalScopeGrants,
    revokeApprovalScopeGrant,
    controlTaskGoalById,
    submitInteractionTask,
    markTaskSubmitted,
    recordTaskResult,
  };
}
