import type { ConversationInputEventRecord } from "../types/api";

type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface ConversationInputEventScope {
  conversationId: string;
  agentId: string;
  channel: string;
  channelAccountId: string;
}

export type ConversationInputEventHandler = (
  event: ConversationInputEventRecord,
) => void | Promise<void>;

const RECONNECT_DELAY_MS = 350;

export class ConversationInputSseParser {
  private buffer = "";
  private dataLines: string[] = [];

  constructor(private readonly onEvent: (event: ConversationInputEventRecord) => void) {}

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
    const value = JSON.parse(raw) as ConversationInputEventRecord;
    if (
      !value ||
      typeof value !== "object" ||
      typeof value.event_seq !== "number" ||
      typeof value.input_id !== "string" ||
      typeof value.event_kind !== "string"
    ) {
      throw new Error("conversation_input_event_schema_invalid");
    }
    this.onEvent(value);
  }
}

export async function followConversationInputEventStream(
  apiFetch: ApiFetch,
  scope: ConversationInputEventScope,
  onEvent: ConversationInputEventHandler,
  signal?: AbortSignal,
): Promise<void> {
  let cursor = 0;
  const path = `/v1/conversations/${encodeURIComponent(scope.conversationId)}/events`;

  while (!signal?.aborted) {
    const query = new URLSearchParams({
      agent_id: scope.agentId,
      channel: scope.channel,
      channel_account_id: scope.channelAccountId,
      cursor: String(cursor),
      follow: "true",
    });
    let response: Response;
    try {
      response = await apiFetch(`${path}?${query.toString()}`, {
        headers: {
          Accept: "text/event-stream",
          "Last-Event-ID": String(cursor),
        },
        signal,
      });
    } catch {
      if (signal?.aborted) return;
      await abortableDelay(RECONNECT_DELAY_MS, signal);
      continue;
    }
    if (!response.ok) {
      throw new Error(`conversation_input_event_stream_http_${response.status}`);
    }
    if (!response.body) {
      throw new Error("conversation_input_event_stream_body_missing");
    }

    let handlerChain = Promise.resolve();
    const parser = new ConversationInputSseParser((event) => {
      if (event.event_seq <= cursor) return;
      cursor = event.event_seq;
      handlerChain = handlerChain.then(() => onEvent(event));
    });
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let disconnected = false;
    try {
      while (!signal?.aborted) {
        let chunk: ReadableStreamReadResult<Uint8Array>;
        try {
          chunk = await reader.read();
        } catch {
          disconnected = true;
          break;
        }
        if (chunk.done) break;
        parser.push(decoder.decode(chunk.value, { stream: true }));
      }
      if (!disconnected) {
        parser.push(decoder.decode());
        parser.finish();
      }
    } finally {
      reader.releaseLock();
    }
    await handlerChain;
    if (!signal?.aborted) await abortableDelay(RECONNECT_DELAY_MS, signal);
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
