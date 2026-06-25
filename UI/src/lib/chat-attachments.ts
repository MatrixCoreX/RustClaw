import type { ChatAttachment, ChatAttachmentKind } from "../types/api";

export const CHAT_MAX_ATTACHMENTS = 6;
export const CHAT_MAX_ATTACHMENT_BYTES = 20 * 1024 * 1024;

export function fileToDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") resolve(reader.result);
      else reject(new Error("读取文件失败"));
    };
    reader.onerror = () => reject(new Error("读取文件失败"));
    reader.readAsDataURL(file);
  });
}

export function chatAttachmentKindForFile(file: File, forcedKind?: ChatAttachmentKind): ChatAttachmentKind {
  if (forcedKind) return forcedKind;
  if (file.type.startsWith("image/")) return "image";
  if (file.type.startsWith("audio/")) return "audio";
  return "file";
}

export async function fileToChatAttachment(file: File, forcedKind?: ChatAttachmentKind): Promise<ChatAttachment> {
  if (file.size > CHAT_MAX_ATTACHMENT_BYTES) {
    throw new Error(`文件过大：${formatAttachmentSize(file.size)}，单个文件上限 ${formatAttachmentSize(CHAT_MAX_ATTACHMENT_BYTES)}`);
  }
  return {
    name: file.name || defaultAttachmentName(chatAttachmentKindForFile(file, forcedKind), file.type),
    dataUrl: await fileToDataUrl(file),
    mimeType: file.type || "application/octet-stream",
    size: file.size,
    kind: chatAttachmentKindForFile(file, forcedKind),
  };
}

export function formatAttachmentSize(bytes: number | null | undefined): string {
  if (!Number.isFinite(bytes) || !bytes || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  const digits = value >= 10 || unitIndex === 0 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unitIndex]}`;
}

export function attachmentIsImage(attachment: ChatAttachment): boolean {
  return attachment.kind === "image" || attachment.mimeType.startsWith("image/");
}

export function attachmentIsAudio(attachment: ChatAttachment): boolean {
  return attachment.kind === "audio" || attachment.mimeType.startsWith("audio/");
}

export function defaultAttachmentName(kind: ChatAttachmentKind, mimeType = ""): string {
  const ext = extensionForMime(kind, mimeType);
  return `${kind}-${Date.now()}.${ext}`;
}

export function audioExtensionForMime(mimeType = ""): string {
  return extensionForMime("audio", mimeType);
}

function extensionForMime(kind: ChatAttachmentKind, mimeType = ""): string {
  const normalized = mimeType.split(";")[0]?.trim().toLowerCase() ?? "";
  if (normalized.includes("wav")) return "wav";
  if (normalized.includes("mpeg") || normalized.includes("mp3")) return "mp3";
  if (normalized.includes("mp4") || normalized.includes("m4a")) return "m4a";
  if (normalized.includes("ogg") || normalized.includes("opus")) return "ogg";
  if (normalized.includes("webm")) return "webm";
  if (normalized.includes("png")) return "png";
  if (normalized.includes("jpeg") || normalized.includes("jpg")) return "jpg";
  if (normalized.includes("webp")) return "webp";
  if (kind === "audio") return "webm";
  if (kind === "image") return "png";
  return "bin";
}

export function formatVisionResultText(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed.startsWith("{")) return raw;
  try {
    const parsed = JSON.parse(trimmed) as {
      summary?: unknown;
      objects?: unknown;
      visible_text?: unknown;
      uncertainties?: unknown;
    };
    const lines: string[] = [];
    if (typeof parsed.summary === "string" && parsed.summary.trim()) {
      lines.push(parsed.summary.trim());
    }
    if (Array.isArray(parsed.objects) && parsed.objects.length > 0) {
      lines.push(`Objects: ${parsed.objects.join(", ")}`);
    }
    if (Array.isArray(parsed.visible_text) && parsed.visible_text.length > 0) {
      lines.push(`Visible text: ${parsed.visible_text.join(" ; ")}`);
    }
    if (Array.isArray(parsed.uncertainties) && parsed.uncertainties.length > 0) {
      lines.push(`Uncertainties: ${parsed.uncertainties.join(" ; ")}`);
    }
    return lines.length > 0 ? lines.join("\n\n") : raw;
  } catch {
    return raw;
  }
}
