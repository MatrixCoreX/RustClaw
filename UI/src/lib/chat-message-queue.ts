export interface QueuedChatMessage {
  id: string;
  threadId: string;
  text: string;
  attachments: string[];
  status: "queued" | "running";
}

interface QueueEntry extends QueuedChatMessage {
  run: (signal: AbortSignal) => Promise<boolean | "waiting">;
  controller: AbortController;
}

export class ChatMessageQueue {
  private entries: QueueEntry[] = [];
  private paused = new Set<string>();
  private blocked = new Set<string>();
  private listeners = new Set<() => void>();
  private disposed = false;
  private snapshot: { messages: QueuedChatMessage[]; paused: string[] } = { messages: [], paused: [] };

  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  enqueue(message: Omit<QueuedChatMessage, "status">, run: QueueEntry["run"]): boolean {
    if (this.disposed || this.entries.some(entry => entry.id === message.id)) return false;
    this.entries.push({ ...message, status: "queued", run, controller: new AbortController() });
    this.publish();
    this.pump();
    return true;
  }

  setBlockedThreads(threadIds: string[]) {
    this.blocked = new Set(threadIds);
    this.pump();
  }

  remove(id: string) {
    this.entries = this.entries.filter(entry => entry.id !== id || entry.status === "running");
    this.publish();
  }

  removeThread(threadId: string) {
    this.entries = this.entries.filter(entry => entry.threadId !== threadId || entry.status === "running");
    this.paused.delete(threadId);
    this.publish();
  }

  resume(threadId: string) {
    this.paused.delete(threadId);
    this.publish();
    this.pump();
  }

  pause(threadId: string) {
    this.paused.add(threadId);
    this.publish();
  }

  activate() {
    this.disposed = false;
  }

  dispose() {
    this.disposed = true;
    for (const entry of this.entries) entry.controller.abort();
    this.entries = [];
    this.paused.clear();
    this.publish();
  }

  private publish() {
    this.snapshot = {
      messages: this.entries.map(({ id, threadId, text, attachments, status }) => ({ id, threadId, text, attachments, status })),
      paused: [...this.paused],
    };
    for (const listener of this.listeners) listener();
  }

  private pump() {
    if (this.disposed) return;
    const active = new Set(this.entries.filter(entry => entry.status === "running").map(entry => entry.threadId));
    for (const entry of this.entries) {
      if (entry.status !== "queued" || active.has(entry.threadId) || this.blocked.has(entry.threadId) || this.paused.has(entry.threadId)) continue;
      active.add(entry.threadId);
      entry.status = "running";
      this.publish();
      void this.execute(entry);
    }
  }

  private async execute(entry: QueueEntry) {
    let outcome: boolean | "waiting" = false;
    try {
      outcome = await entry.run(entry.controller.signal);
    } catch {
      // Fail closed: later dependent messages need an explicit user decision.
    } finally {
      if (!this.disposed && !entry.controller.signal.aborted) {
        this.entries = this.entries.filter(item => item !== entry);
        if (outcome === false) this.paused.add(entry.threadId);
        this.publish();
        this.pump();
      }
    }
  }
}
