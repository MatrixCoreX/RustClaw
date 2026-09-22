import { ListOrdered, Play, X } from "lucide-react";
import type { QueuedChatMessage } from "../lib/chat-message-queue";

export function ChatMessageQueue({ messages, paused, busy, onRemove, onResume, t }: {
  messages: QueuedChatMessage[];
  paused: boolean;
  busy: boolean;
  onRemove: (id: string) => void;
  onResume: () => void;
  t: (zh: string, en: string) => string;
}) {
  if (!messages.length) return null;
  return (
    <section data-testid="chat-message-queue" aria-label={t("待发送消息", "Queued messages")}
      className="mt-2 min-w-0 shrink-0 border-t border-[var(--theme-border)] pt-2 text-xs text-[var(--theme-text-body)]">
      <div className="mb-1 flex flex-wrap items-center gap-2 text-[var(--theme-text-muted)]">
        <ListOrdered className="h-3.5 w-3.5" aria-hidden="true" />
        <span aria-live="polite">{t(`待发送（${messages.length}）`, `Queued (${messages.length})`)}</span>
        {paused ? <>
          <span>{t("队列已暂停", "Queue paused")}</span>
          <button type="button" onClick={onResume} disabled={busy}
            className="theme-secondary-btn ml-auto inline-flex items-center gap-1 rounded-md border px-2 py-1 disabled:opacity-50">
            <Play className="h-3 w-3" aria-hidden="true" />{t("继续发送", "Continue sending")}
          </button>
        </> : null}
      </div>
      <ol className="max-h-28 space-y-1 overflow-y-auto">
        {messages.map((message, index) => (
          <li key={message.id} className="flex min-w-0 items-center gap-2" data-queued-message-id={message.id}>
            <span className="w-4 shrink-0 tabular-nums text-[var(--theme-text-muted)]">{index + 1}</span>
            <span className="min-w-0 flex-1 truncate" title={[message.text, ...message.attachments].filter(Boolean).join("\n")}>
              {message.text || message.attachments.join(", ")}
              {message.text && message.attachments.length ? ` · ${t(`${message.attachments.length} 个附件`, `${message.attachments.length} attachments`)}` : ""}
            </span>
            <button type="button" onClick={() => onRemove(message.id)}
              title={t("移除待发送消息", "Remove queued message")} aria-label={t("移除待发送消息", "Remove queued message")}
              className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md hover:bg-black/10">
              <X className="h-3.5 w-3.5" />
            </button>
          </li>
        ))}
      </ol>
    </section>
  );
}
