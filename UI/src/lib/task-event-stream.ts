import type { TaskEventEnvelope } from "../types/api";

type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export type TaskEventHandler = (event: TaskEventEnvelope) => void | Promise<void>;

const RECONNECT_DELAY_MS = 350;

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
    this.onEvent(value);
  }
}

export async function followTaskEventStream(
  apiFetch: ApiFetch,
  taskId: string,
  onEvent: TaskEventHandler,
  signal?: AbortSignal,
): Promise<void> {
  const normalizedTaskId = encodeURIComponent(taskId.trim());
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

    const pendingHandlers: Promise<void>[] = [];
    const parser = new TaskSseParser((event) => {
      if (typeof event.seq === "number" && event.seq > cursor) {
        cursor = event.seq;
      }
      terminal = event.event_kind === "task_final";
      pendingHandlers.push(Promise.resolve(onEvent(event)));
    });
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    try {
      while (!terminal && !signal?.aborted) {
        const { value, done } = await reader.read();
        if (done) break;
        parser.push(decoder.decode(value, { stream: true }));
      }
      parser.push(decoder.decode());
      parser.finish();
      await Promise.all(pendingHandlers);
    } finally {
      reader.releaseLock();
    }
    if (!terminal && !signal?.aborted) {
      await abortableDelay(RECONNECT_DELAY_MS, signal);
    }
  }
}

function abortableDelay(delayMs: number, signal?: AbortSignal): Promise<void> {
  if (signal?.aborted) return Promise.resolve();
  return new Promise((resolve) => {
    const timeout = window.setTimeout(resolve, delayMs);
    signal?.addEventListener(
      "abort",
      () => {
        window.clearTimeout(timeout);
        resolve();
      },
      { once: true },
    );
  });
}
