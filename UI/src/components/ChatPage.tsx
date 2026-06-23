import type { KeyboardEvent, RefObject } from "react";
import { Loader2, RefreshCw, X } from "lucide-react";
import ReactMarkdown from "react-markdown";

import type { ChatImageAttachment, ChatMessage } from "../types/api";

type Translate = (zh: string, en: string) => string;

export interface ChatPageProps {
  t: Translate;
  chatMessages: ChatMessage[];
  chatInput: string;
  chatImageAttachments: ChatImageAttachment[];
  chatAgentMode: boolean;
  chatSending: boolean;
  chatError: string | null;
  chatImageInputRef: RefObject<HTMLInputElement | null>;
  toLocalTime: (value: number | null | undefined) => string;
  onChatAgentModeChange: (value: boolean) => void;
  onClearMessages: () => void;
  onChatInputChange: (value: string) => void;
  onChatInputKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  onImageSelection: (fileList: FileList | null) => unknown | Promise<unknown>;
  onRemoveImageAttachment: (index: number) => void;
  onSendMessage: () => unknown | Promise<unknown>;
}

export function ChatPage({
  t,
  chatMessages,
  chatInput,
  chatImageAttachments,
  chatAgentMode,
  chatSending,
  chatError,
  chatImageInputRef,
  toLocalTime,
  onChatAgentModeChange,
  onClearMessages,
  onChatInputChange,
  onChatInputKeyDown,
  onImageSelection,
  onRemoveImageAttachment,
  onSendMessage,
}: ChatPageProps) {
  return (
    <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <h3 className="text-base font-semibold">{t("和 RustClaw 对话", "Chat with RustClaw")}</h3>
        <div className="flex flex-wrap items-center gap-3 text-sm">
          <label className="inline-flex items-center gap-2 text-white/80">
            <input type="checkbox" checked={chatAgentMode} onChange={(event) => onChatAgentModeChange(event.target.checked)} />
            agent_mode
          </label>
          <button
            type="button"
            onClick={onClearMessages}
            className="rounded-lg border border-white/15 bg-white/5 px-3 py-1.5 text-xs hover:bg-white/10"
          >
            {t("清空记录", "Clear")}
          </button>
        </div>
      </div>

      <div className="h-80 space-y-3 overflow-auto rounded-xl border border-white/10 bg-black/30 p-3">
        {chatMessages.map((message) => (
          <div key={message.id} className="space-y-1">
            <div className="flex items-center gap-2 text-[11px] text-white/50">
              <span>{message.role}</span>
              <span>{toLocalTime(message.ts)}</span>
            </div>
            <div
              className={
                message.role === "user"
                  ? "theme-user-bubble max-w-[95%] rounded-xl px-3 py-2 text-sm text-white"
                  : message.role === "assistant"
                    ? "max-w-[95%] rounded-xl bg-emerald-500/15 px-3 py-2 text-sm text-white"
                    : "max-w-[95%] rounded-xl bg-white/10 px-3 py-2 text-sm text-white/80"
              }
            >
              {message.role === "assistant" ? (
                <div className="chat-markdown">
                  <ReactMarkdown>{message.text}</ReactMarkdown>
                </div>
              ) : (
                <pre className="whitespace-pre-wrap break-words font-sans">{message.text}</pre>
              )}
              {message.images && message.images.length > 0 ? (
                <div className="mt-3 flex flex-wrap gap-2">
                  {message.images.map((image) => (
                    <img
                      key={`${message.id}-${image.name}`}
                      src={image.dataUrl}
                      alt={image.name}
                      className="max-h-40 rounded-lg border border-white/10 object-contain"
                    />
                  ))}
                </div>
              ) : null}
            </div>
          </div>
        ))}
      </div>

      <div className="mt-4 grid shrink-0 gap-3 md:grid-cols-[1fr_auto]">
        <div className="min-w-0">
          {chatImageAttachments.length > 0 ? (
            <div className="mb-3 flex flex-wrap gap-2 rounded-xl border border-white/10 bg-white/5 p-2">
              {chatImageAttachments.map((image, index) => (
                <div key={`${image.name}-${index}`} className="relative">
                  <img
                    src={image.dataUrl}
                    alt={image.name}
                    className="h-20 w-20 rounded-lg border border-white/10 object-cover"
                  />
                  <button
                    type="button"
                    onClick={() => onRemoveImageAttachment(index)}
                    className="absolute -right-2 -top-2 rounded-full border border-white/15 bg-black/70 p-1 text-white/80 hover:bg-black/85"
                    title={t("移除图片", "Remove image")}
                  >
                    <X className="h-3 w-3" />
                  </button>
                </div>
              ))}
            </div>
          ) : null}
          <div className="mb-3 flex flex-wrap items-center gap-2">
            <input
              ref={chatImageInputRef}
              type="file"
              accept="image/*"
              multiple
              className="hidden"
              onChange={(event) => void onImageSelection(event.target.files)}
            />
            <button
              type="button"
              onClick={() => chatImageInputRef.current?.click()}
              className="rounded-lg border border-white/15 bg-white/5 px-3 py-1.5 text-xs hover:bg-white/10"
            >
              {t("选图片", "Choose images")}
            </button>
            <span className="text-xs text-white/45">
              {t("可直接发图，也可以带一句说明。", "You can send images directly or add a short instruction.")}
            </span>
          </div>
          <textarea
            className="theme-input min-h-24 w-full resize-none"
            placeholder={t("例如：你好，请告诉我你现在能做什么；或发一张图片让我看看", "For example: Hello, tell me what you can do; or send an image for analysis")}
            value={chatInput}
            onChange={(event) => onChatInputChange(event.target.value)}
            onKeyDown={onChatInputKeyDown}
          />
        </div>
        <button
          type="button"
          onClick={() => void onSendMessage()}
          disabled={chatSending || (!chatInput.trim() && chatImageAttachments.length === 0)}
          className="theme-accent-btn shrink-0"
        >
          {chatSending ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
          {t("发送", "Send")}
        </button>
      </div>
      {chatError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {t("聊天错误", "Chat error")}: {chatError}
        </p>
      ) : null}
    </section>
  );
}
