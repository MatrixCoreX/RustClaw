import type { KeyboardEvent, RefObject } from "react";
import { FileText, Loader2, Mic, Paperclip, RefreshCw, Square, X } from "lucide-react";
import ReactMarkdown from "react-markdown";

import {
  attachmentIsAudio,
  attachmentIsImage,
  formatAttachmentSize,
} from "../lib/chat-attachments";
import type { ChatAttachment, ChatMessage } from "../types/api";

type Translate = (zh: string, en: string) => string;

export interface ChatPageProps {
  t: Translate;
  chatMessages: ChatMessage[];
  chatInput: string;
  chatAttachments: ChatAttachment[];
  chatAgentMode: boolean;
  chatSending: boolean;
  chatRecording: boolean;
  chatError: string | null;
  chatAttachmentInputRef: RefObject<HTMLInputElement | null>;
  toLocalTime: (value: number | null | undefined) => string;
  onChatAgentModeChange: (value: boolean) => void;
  onClearMessages: () => void;
  onChatInputChange: (value: string) => void;
  onChatInputKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  onAttachmentSelection: (fileList: FileList | null) => unknown | Promise<unknown>;
  onRemoveAttachment: (index: number) => void;
  onStartVoiceRecording: () => unknown | Promise<unknown>;
  onStopVoiceRecording: () => unknown | Promise<unknown>;
  onSendMessage: () => unknown | Promise<unknown>;
}

export function ChatPage({
  t,
  chatMessages,
  chatInput,
  chatAttachments,
  chatAgentMode,
  chatSending,
  chatRecording,
  chatError,
  chatAttachmentInputRef,
  toLocalTime,
  onChatAgentModeChange,
  onClearMessages,
  onChatInputChange,
  onChatInputKeyDown,
  onAttachmentSelection,
  onRemoveAttachment,
  onStartVoiceRecording,
  onStopVoiceRecording,
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
              {(message.attachments ?? message.images)?.length ? (
                <div className="mt-3 flex flex-wrap gap-2">
                  {(message.attachments ?? message.images ?? []).map((attachment, index) => (
                    <AttachmentPreview
                      key={`${message.id}-${attachment.name}-${index}`}
                      attachment={attachment}
                      t={t}
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
          {chatAttachments.length > 0 ? (
            <div className="mb-3 flex flex-wrap gap-2 rounded-xl border border-white/10 bg-white/5 p-2">
              {chatAttachments.map((attachment, index) => (
                <div key={`${attachment.name}-${index}`} className="relative">
                  <AttachmentPreview attachment={attachment} t={t} compact />
                  <button
                    type="button"
                    onClick={() => onRemoveAttachment(index)}
                    className="absolute -right-2 -top-2 rounded-full border border-white/15 bg-black/70 p-1 text-white/80 hover:bg-black/85"
                    title={t("移除附件", "Remove attachment")}
                  >
                    <X className="h-3 w-3" />
                  </button>
                </div>
              ))}
            </div>
          ) : null}
          <div className="mb-3 flex flex-wrap items-center gap-2">
            <input
              ref={chatAttachmentInputRef}
              type="file"
              multiple
              className="hidden"
              onChange={(event) => void onAttachmentSelection(event.target.files)}
            />
            <button
              type="button"
              onClick={() => chatAttachmentInputRef.current?.click()}
              disabled={chatSending || chatRecording}
              className="inline-flex items-center gap-1.5 rounded-lg border border-white/15 bg-white/5 px-3 py-1.5 text-xs hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50"
            >
              <Paperclip className="h-3.5 w-3.5" />
              {t("上传图片/文件", "Upload image/file")}
            </button>
            <button
              type="button"
              onPointerDown={(event) => {
                if (event.button !== 0) return;
                event.preventDefault();
                event.currentTarget.setPointerCapture?.(event.pointerId);
                void onStartVoiceRecording();
              }}
              onPointerUp={(event) => {
                event.preventDefault();
                onStopVoiceRecording();
              }}
              onPointerCancel={() => onStopVoiceRecording()}
              onKeyDown={(event) => {
                if (event.repeat || (event.key !== " " && event.key !== "Enter")) return;
                event.preventDefault();
                void onStartVoiceRecording();
              }}
              onKeyUp={(event) => {
                if (event.key !== " " && event.key !== "Enter") return;
                event.preventDefault();
                onStopVoiceRecording();
              }}
              onContextMenu={(event) => event.preventDefault()}
              disabled={chatSending}
              className={
                chatRecording
                  ? "inline-flex select-none items-center gap-1.5 rounded-lg border border-emerald-400/35 bg-emerald-500/15 px-3 py-1.5 text-xs text-emerald-100 disabled:cursor-not-allowed disabled:opacity-50"
                  : "inline-flex select-none items-center gap-1.5 rounded-lg border border-white/15 bg-white/5 px-3 py-1.5 text-xs hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50"
              }
            >
              {chatRecording ? (
                <Square className="h-3.5 w-3.5" />
              ) : (
                <Mic className="h-3.5 w-3.5" />
              )}
              {chatRecording ? t("松开发送", "Release to send") : t("按住说话", "Hold to talk")}
            </button>
            <span className="text-xs text-white/45">
              {t(
                "可直接发送图片、文件或语音，也可以带一句说明。",
                "Send images, files, or voice directly, with an optional note.",
              )}
            </span>
          </div>
          <textarea
            className="theme-input min-h-24 w-full resize-none"
            placeholder={t(
              "例如：你好，请告诉我你现在能做什么；或上传附件让我看看",
              "For example: Hello, tell me what you can do; or upload an attachment for review",
            )}
            value={chatInput}
            onChange={(event) => onChatInputChange(event.target.value)}
            onKeyDown={onChatInputKeyDown}
          />
        </div>
        <button
          type="button"
          onClick={() => void onSendMessage()}
          disabled={
            chatSending || chatRecording || (!chatInput.trim() && chatAttachments.length === 0)
          }
          className="theme-accent-btn shrink-0"
        >
          {chatSending ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <RefreshCw className="h-4 w-4" />
          )}
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

function AttachmentPreview({
  attachment,
  t,
  compact = false,
}: {
  attachment: ChatAttachment;
  t: Translate;
  compact?: boolean;
}) {
  if (attachmentIsImage(attachment)) {
    return (
      <img
        src={attachment.dataUrl}
        alt={attachment.name}
        className={
          compact
            ? "h-20 w-20 rounded-lg border border-white/10 object-cover"
            : "max-h-40 rounded-lg border border-white/10 object-contain"
        }
      />
    );
  }
  if (attachmentIsAudio(attachment)) {
    return (
      <div
        className={
          compact
            ? "w-52 rounded-lg border border-white/10 bg-black/25 p-2"
            : "w-64 rounded-lg border border-white/10 bg-black/25 p-2"
        }
      >
        <div className="mb-2 flex items-center gap-2 text-xs text-white/75">
          <Mic className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 truncate" title={attachment.name}>
            {attachment.name}
          </span>
        </div>
        <audio
          controls
          src={attachment.dataUrl}
          className="h-8 w-full"
          title={t("语音预览", "Voice preview")}
        />
      </div>
    );
  }
  return (
    <div
      className={
        compact
          ? "flex h-20 w-44 items-center gap-2 rounded-lg border border-white/10 bg-black/25 p-2"
          : "flex max-w-72 items-center gap-2 rounded-lg border border-white/10 bg-black/25 p-2"
      }
    >
      <FileText className="h-5 w-5 shrink-0 text-white/70" />
      <div className="min-w-0 text-xs">
        <div className="truncate text-white/80" title={attachment.name}>
          {attachment.name}
        </div>
        <div className="text-white/45">{formatAttachmentSize(attachment.size)}</div>
      </div>
    </div>
  );
}
