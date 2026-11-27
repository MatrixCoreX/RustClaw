import { useEffect, useRef, useState } from "react";
import { uploadTaskAttachments } from "../lib/task-attachment-upload";

const LIMIT = 20 * 1024 * 1024;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const SCRIPT_SLOT = "<!-- AIPP_SCRIPT -->";
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type Props = {
  skill: string;
  version: string;
  lang: "zh" | "en";
  title: string;
  apiFetch: ApiFetch;
};

async function jsonResponse(response: Response) {
  const body = await response.json();
  if (!response.ok || !body.ok || body.data == null) {
    throw new Error(typeof body.error === "string" ? body.error : "aipp_request_failed");
  }
  return body.data as unknown;
}

export function AippTranscriptFrame({ skill, version, lang, title, apiFetch }: Props) {
  const frame = useRef<HTMLIFrameElement>(null);
  const fetchRef = useRef(apiFetch);
  fetchRef.current = apiFetch;
  const [source, setSource] = useState<string>();
  const [loadError, setLoadError] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    const base = `/v1/aipps/${encodeURIComponent(skill)}`;
    const jobs = `${base}/transcripts`;
    const pending = new Set<string>();
    let submitting = false;

    let readyTimer: number | undefined;
    void (async () => {
      try {
        const response = await fetchRef.current(`${jobs}/frame`, {
          method: "POST", headers: { "Content-Type": "application/json" }, signal: controller.signal,
          body: JSON.stringify({ package_version: version }),
        });
        const data = await jsonResponse(response) as { html?: unknown; script_ticket?: unknown };
        const { html, script_ticket: ticket } = data;
        if (typeof html !== "string" || html.length > 64 * 1024 || html.split(SCRIPT_SLOT).length !== 2
            || typeof ticket !== "string" || !UUID.test(ticket) || !/<head>/i.test(html)) throw new Error();
        const script = new URL(`${jobs}/script/${ticket}`, window.location.origin).href;
        const policy = `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src ${script}; style-src 'unsafe-inline'; connect-src 'none'; form-action 'none'; base-uri 'none'">`;
        if (controller.signal.aborted) return;
        setSource(html.replace(/<head>/i, (tag) => tag + policy)
          .replace(SCRIPT_SLOT, `<script src="${script}" referrerpolicy="no-referrer"></script>`));
        readyTimer = window.setTimeout(() => setLoadError(true), 10000);
      } catch {
        if (!controller.signal.aborted) setLoadError(true);
      }
    })();

    const listener = (event: MessageEvent) => {
      const target = frame.current?.contentWindow;
      const message = event.data;
      if (!target || event.source !== target || event.origin !== "null" || message?.schema_version !== 1) return;
      if (message.type === "transcript.ready") {
        window.clearTimeout(readyTimer);
        target.postMessage({ schema_version: 1, type: "transcript.context", lang, maxBytes: LIMIT,
          theme: document.documentElement.getAttribute("data-theme") }, "*");
        return;
      }
      const id: unknown = message.id;
      if (typeof id !== "string" || !UUID.test(id) || pending.has(id) || pending.size >= 4) return;
      if (!["transcript.submit", "transcript.list", "transcript.read"].includes(message.type)) return;
      pending.add(id);
      const send = (ok: boolean, data?: unknown, error?: string) => {
        if (!controller.signal.aborted && frame.current?.contentWindow === target) {
          target.postMessage({ schema_version: 1, type: "transcript.result", id, ok, data, error }, "*");
        }
      };
      void (async () => {
        try {
          if (message.type === "transcript.submit") {
            if (submitting) throw new Error("ui_attachment_upload_busy");
            const file = message.file;
            if (!(file instanceof File) || file.size < 1 || file.size > LIMIT) {
              throw new Error("ui_attachment_too_large");
            }
            const kind = file.type.split("/", 1)[0];
            if (kind !== "audio" && kind !== "video") throw new Error("aipp_transcript_attachment_invalid");
            submitting = true;
            try {
              const [attachment] = await uploadTaskAttachments(fetchRef.current, id, [{
                name: file.name, kind: kind === "video" ? "file" : "audio", mimeType: file.type, file, size: file.size,
              }], controller.signal);
              if (!attachment) throw new Error("ui_attachment_content_missing");
              const response = await fetchRef.current(jobs, {
                method: "POST", headers: { "Content-Type": "application/json" }, signal: controller.signal,
                body: JSON.stringify({ request_id: id, package_version: version, attachment }),
              });
              send(true, await jsonResponse(response));
            } finally {
              submitting = false;
            }
          } else if (message.type === "transcript.read") {
            if (typeof message.taskId !== "string" || !UUID.test(message.taskId)) throw new Error("aipp_transcript_not_found");
            send(true, await jsonResponse(await fetchRef.current(`${jobs}/${message.taskId}`, { signal: controller.signal })));
          } else {
            const before = typeof message.before === "number" && Number.isSafeInteger(message.before)
              && message.before > 0 ? `?before=${message.before}` : "";
            send(true, await jsonResponse(await fetchRef.current(jobs + before, { signal: controller.signal })));
          }
        } catch (error) {
          send(false, undefined, error instanceof Error ? error.message : "aipp_request_failed");
        } finally {
          pending.delete(id);
        }
      })();
    };
    window.addEventListener("message", listener);
    return () => {
      controller.abort();
      window.clearTimeout(readyTimer);
      window.removeEventListener("message", listener);
    };
  }, [lang, skill, version]);

  if (loadError) return <p role="alert">{lang === "zh" ? "无法打开应用，请刷新后重试。" : "The app could not open. Refresh to retry."}</p>;
  if (!source) return <p role="status">{lang === "zh" ? "正在打开…" : "Opening…"}</p>;
  return <iframe ref={frame} title={title} srcDoc={source} sandbox="allow-scripts allow-downloads"
    referrerPolicy="no-referrer" className="min-h-[72vh] w-full rounded-xl border border-[var(--theme-border)]" />;
}
