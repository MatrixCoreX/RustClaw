import type { ApiResponse, TaskEventEnvelope, TaskQueryResponse } from "../types/api";
import { decodeAssistantPresentationEvent } from "./assistant-presentation";

type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export type TaskEventHandler = (event: TaskEventEnvelope) => void | Promise<void>;

const RECONNECT_DELAY_MS = 350;
const TERMINAL_POLL_INTERVAL_MS = 5_000;

export interface FollowTaskEventStreamOptions {
  terminalPollIntervalMs?: number;
}

export function taskEventClosesLiveStream(event: TaskEventEnvelope): boolean {
  if (event.event_kind === "task_final") return true;
  if (event.event_kind !== "task_state") return false;
  const executionState = typeof event.payload?.execution_state === "string" ? event.payload.execution_state : "";
  const lifecycle = event.payload?.lifecycle;
  const lifecycleState =
    lifecycle && typeof lifecycle === "object" && "state" in lifecycle && typeof lifecycle.state === "string"
      ? lifecycle.state
      : "";
  return (
    executionState === "needs_confirmation" ||
    executionState === "blocked" ||
    lifecycleState === "needs_user" ||
    lifecycleState === "blocked"
  );
}

export class TaskSseParser {
  private buffer = "";
  private dataLines: string[] = [];

  constructor(private readonly onEvent: (event: TaskEventEnvelope) => void) {}

  push(chunk: string): void {
    this.buffer += chunk;
    let newline = this.buffer.indexOf("\n");
    while (newline >= 0) {
      const line = this.buffer.slice(0, newline).replace(/\r$/, "");
      this.buffer = this.buffer.slice(newline + 1);
      this.consumeLine(line);
      newline = this.buffer.indexOf("\n");
    }
  }

  finish(): void {
    if (this.buffer) {
      this.consumeLine(this.buffer.replace(/\r$/, ""));
      this.buffer = "";
    }
    this.emitData();
  }

  private consumeLine(line: string): void {
    if (!line) {
      this.emitData();
      return;
    }
    if (line.startsWith("data:")) {
      const data = line.slice(5);
      this.dataLines.push(data.startsWith(" ") ? data.slice(1) : data);
    }
  }

  private emitData(): void {
    if (this.dataLines.length === 0) return;
    const raw = this.dataLines.join("\n");
    this.dataLines = [];
    const value = JSON.parse(raw) as TaskEventEnvelope;
    if (!value || typeof value !== "object" || typeof value.event_kind !== "string") {
      throw new Error("task_event_schema_invalid");
    }
    decodeAssistantPresentationEvent(value);
    this.onEvent(value);
  }
}

export async function followTaskEventStream(
  apiFetch: ApiFetch,
  taskId: string,
  onEvent: TaskEventHandler,
  signal?: AbortSignal,
  options: FollowTaskEventStreamOptions = {},
): Promise<void> {
  const normalizedTaskId = encodeURIComponent(taskId.trim());
  const terminalPollIntervalMs = normalizedPollInterval(options.terminalPollIntervalMs);
  let cursor = 0;
  let terminal = false;

  while (!terminal && !signal?.aborted) {
    let response: Response;
    try {
      response = await apiFetch(`/v1/tasks/${normalizedTaskId}/events?cursor=${cursor}`, {
        headers: {
          Accept: "text/event-stream",
          "Last-Event-ID": String(cursor),
        },
        signal,
      });
    } catch (error) {
      if (signal?.aborted) return;
      await abortableDelay(RECONNECT_DELAY_MS, signal);
      continue;
    }
    if (!response.ok) {
      throw new Error(`task_event_stream_http_${response.status}`);
    }
    if (!response.body) {
      throw new Error("task_event_stream_body_missing");
    }

    let handlerChain = Promise.resolve();
    const parser = new TaskSseParser((event) => {
      if (typeof event.seq === "number" && event.seq > cursor) {
        cursor = event.seq;
      }
      terminal = taskEventClosesLiveStream(event);
      handlerChain = handlerChain.then(() => onEvent(event));
    });
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let streamDisconnected = false;
    let pendingRead = readStreamChunk(reader);
    let pendingTerminalCheck = terminalCheckDelay(terminalPollIntervalMs, signal);
    try {
      while (!terminal && !signal?.aborted) {
        const outcome = await Promise.race([pendingRead, pendingTerminalCheck]);
        if (outcome.kind === "terminal_check") {
          if (signal?.aborted) return;
          if (await taskReachedFollowBoundary(apiFetch, normalizedTaskId, signal)) {
            terminal = true;
            streamDisconnected = true;
            try {
              await reader.cancel();
            } catch {
              // The task status is authoritative even if the transport already disconnected.
            }
            break;
          }
          pendingTerminalCheck = terminalCheckDelay(terminalPollIntervalMs, signal);
          continue;
        }
        if (outcome.kind === "stream_error") {
          if (signal?.aborted) return;
          streamDisconnected = true;
          break;
        }
        const { value, done } = outcome.chunk;
        if (done) break;
        parser.push(decoder.decode(value, { stream: true }));
        pendingRead = readStreamChunk(reader);
      }
      if (!streamDisconnected) {
        parser.push(decoder.decode());
        parser.finish();
      }
    } finally {
      reader.releaseLock();
    }
    await handlerChain;
    if (!terminal && !signal?.aborted) {
      await abortableDelay(RECONNECT_DELAY_MS, signal);
    }
  }
}

type StreamReadOutcome =
  | { kind: "stream_chunk"; chunk: ReadableStreamReadResult<Uint8Array> }
  | { kind: "stream_error" }
  | { kind: "terminal_check" };

function readStreamChunk(
  reader: ReadableStreamDefaultReader<Uint8Array>,
): Promise<StreamReadOutcome> {
  return reader
    .read()
    .then((chunk) => ({ kind: "stream_chunk" as const, chunk }))
    .catch(() => ({ kind: "stream_error" as const }));
}

function terminalCheckDelay(delayMs: number, signal?: AbortSignal): Promise<StreamReadOutcome> {
  return abortableDelay(delayMs, signal).then(() => ({ kind: "terminal_check" as const }));
}

function normalizedPollInterval(value: number | undefined): number {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.max(1, Math.floor(value))
    : TERMINAL_POLL_INTERVAL_MS;
}

async function taskReachedFollowBoundary(
  apiFetch: ApiFetch,
  normalizedTaskId: string,
  signal?: AbortSignal,
): Promise<boolean> {
  try {
    const response = await apiFetch(`/v1/tasks/${normalizedTaskId}`, {
      headers: { Accept: "application/json" },
      signal,
    });
    if (!response.ok) return false;
    const body = (await response.json()) as ApiResponse<TaskQueryResponse>;
    const task = body.ok ? body.data : undefined;
    if (!task) return false;
    if (["succeeded", "failed", "canceled", "timeout"].includes(task.status)) return true;
    return taskEventClosesLiveStream({
      schema_version: 1,
      task_id: task.task_id,
      event_kind: "task_state",
      payload: {
        status: task.status,
        execution_state: task.execution_state,
        lifecycle: task.lifecycle,
      },
    });
  } catch {
    return false;
  }
}

function abortableDelay(delayMs: number, signal?: AbortSignal): Promise<void> {
  if (signal?.aborted) return Promise.resolve();
  return new Promise((resolve) => {
    const timeout = globalThis.setTimeout(resolve, delayMs);
    signal?.addEventListener(
      "abort",
      () => {
        globalThis.clearTimeout(timeout);
        resolve();
      },
      { once: true },
    );
  });
}
