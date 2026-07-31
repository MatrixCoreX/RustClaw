import type { TaskEventEnvelope } from "../types/api";

export type ChatActivityStage =
  | "analyzing"
  | "llm_request"
  | "llm_response"
  | "choosing_tool"
  | "running_tool"
  | "tool_returned"
  | "finalizing";

export interface ChatActivitySummary {
  stage: ChatActivityStage;
  activeName: string | null;
  commandPreview: string | null;
  llmCallCount: number;
  roundNo: number | null;
  lastSeq: number | null;
}

export function emptyChatActivity(): ChatActivitySummary {
  return {
    stage: "analyzing",
    activeName: null,
    commandPreview: null,
    llmCallCount: 0,
    roundNo: null,
    lastSeq: null,
  };
}

function readableName(payload: Record<string, unknown>): string | null {
  for (const key of [
    "resolved_tool_or_skill",
    "tool_or_skill",
    "action_ref",
    "skill",
    "tool_name",
    "requested_capability",
  ]) {
    const value = payload[key];
    if (typeof value !== "string") continue;
    const compact = value.trim().replace(/\s+/g, " ").slice(0, 80);
    if (compact) return compact;
  }
  return null;
}

function positiveInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0
    ? value
    : null;
}

function commandPreview(payload: Record<string, unknown>): string | null {
  const value = payload.command_preview;
  if (typeof value !== "string") return null;
  const compact = value.trim().replace(/\s+/g, " ").slice(0, 80);
  return compact || null;
}

export function reduceChatActivity(
  current: ChatActivitySummary,
  event: TaskEventEnvelope,
): ChatActivitySummary {
  if (
    typeof event.seq === "number" &&
    current.lastSeq !== null &&
    event.seq <= current.lastSeq
  ) {
    return current;
  }
  const payload = event.payload ?? {};
  const eventType = event.event_type?.trim() || event.event_kind.trim();
  const phase = typeof payload.type === "string" ? payload.type.trim() : "";
  const roundNo = positiveInteger(payload.round_no) ?? current.roundNo;
  const reportedLlmCalls =
    positiveInteger(payload.llm_call_count) ??
    positiveInteger(payload.cumulative_model_turns);
  let next: ChatActivitySummary = {
    ...current,
    roundNo,
    lastSeq: typeof event.seq === "number" ? event.seq : current.lastSeq,
    llmCallCount: reportedLlmCalls
      ? Math.max(current.llmCallCount, reportedLlmCalls)
      : current.llmCallCount,
  };

  if (eventType === "model_turn") {
    if (phase === "started") {
      next = {
        ...next,
        stage: "llm_request",
        activeName: null,
        commandPreview: null,
        llmCallCount: current.llmCallCount + 1,
      };
    } else if (phase === "text_delta" || phase === "usage" || phase === "finished") {
      next = { ...next, stage: "llm_response", activeName: null, commandPreview: null };
    } else if (phase === "tool_call" || phase === "tool_call_delta") {
      next = {
        ...next,
        stage: "choosing_tool",
        activeName: readableName(payload),
        commandPreview: null,
      };
    }
    return next;
  }

  if (eventType === "tool_active" || eventType === "tool_started") {
    return {
      ...next,
      stage: "running_tool",
      activeName: readableName(payload),
      commandPreview: commandPreview(payload),
    };
  }
  if (eventType === "tool_finished") {
    return {
      ...next,
      stage: "tool_returned",
      activeName: readableName(payload) ?? current.activeName,
      commandPreview: commandPreview(payload) ?? current.commandPreview,
    };
  }
  if (eventType === "assistant_output_started" || eventType === "task_final") {
    return { ...next, stage: "finalizing", activeName: null, commandPreview: null };
  }
  if (eventType === "state_transition") {
    const stateTo = typeof payload.state_to === "string" ? payload.state_to : "";
    if (stateTo === "finalizing" || stateTo === "completed") {
      return { ...next, stage: "finalizing", activeName: null, commandPreview: null };
    }
    if (stateTo === "planning") {
      return { ...next, stage: "analyzing", activeName: null, commandPreview: null };
    }
  }
  return next;
}
