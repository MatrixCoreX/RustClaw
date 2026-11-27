import type { ChatAttachment, ApiResponse } from "../types/api";
import { readTaskSubmissionResponse } from "./api-response";
export type UploadedTaskAttachment = { name: string; mime_type: string; size: number; kind: string; path: string };
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type UploadAttachment = Omit<ChatAttachment, "kind"> & { kind: ChatAttachment["kind"] | "video" };
export async function uploadTaskAttachments(apiFetch: ApiFetch, clientId: string,
  attachments: UploadAttachment[], signal?: AbortSignal): Promise<UploadedTaskAttachment[]> {
  if (!attachments.length) return [];
  const form = new FormData();
  for (const attachment of attachments) {
    const blob = attachment.file ?? (attachment.dataUrl?.startsWith("data:")
      ? await (await fetch(attachment.dataUrl, { signal })).blob()
      : null);
    if (!blob) throw new Error("ui_attachment_content_missing");
    form.append(attachment.kind, blob, attachment.name);
  }
  const response = await apiFetch(`/v1/task-attachments/${encodeURIComponent(clientId)}`, { method: "POST", body: form, signal });
  const body = await readTaskSubmissionResponse<ApiResponse<UploadedTaskAttachment[]>>(response);
  if (!body.ok || !body.data) throw new Error(body.error || "ui_attachment_upload_failed");
  return body.data;
}
