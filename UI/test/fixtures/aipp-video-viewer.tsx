import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { AippTaskActivityCard } from "../../src/components/AippPage";
import type { AippTaskActivityItem } from "../../src/types/api";
import { withSessionArtifactStreaming } from "../../src/lib/artifact-session-stream";
import { TaskArtifactCard } from "../../src/components/ChatPage";
import "../../src/index.css";

const params = new URLSearchParams(location.search);
const lang = params.get("lang") === "zh" ? "zh" : "en";
document.documentElement.dataset.theme = params.get("theme") || "dark";
const t = (zh: string, en: string) => lang === "zh" ? zh : en;
const apiFetch = withSessionArtifactStreaming((path, init) => fetch(path, init), {
  authMode: params.get("auth") === "webd" ? "webd" : "key",
  apiBase: location.origin, pageUrl: location.href, isCurrent: () => true,
});
const task: AippTaskActivityItem = {
  schema_version: 1, sequence: 1, task_id: "video-viewer-test", channel: "wechat",
  status: "succeeded", actions: ["example.download"], input_text: "Download videos",
  result_text: "Saved", error_text: null, source_urls: [],
  created_at: "1788912000", updated_at: "1788912000", event_at_ms: 1788912000000,
  artifacts: [1, 2].map((id) => ({
    schema_version: 1, id: `video-${id}`, kind: "video",
    filename: id === 1 ? "clip.webm" : "large-video-with-a-long-filename-that-must-not-displace-the-close-button.mov",
    mime_type: id === 1 ? "video/webm" : "video/quicktime",
    size_bytes: id === 1 ? 1024 : 1024 * 1024 * 1024,
    download_url: `/v1/tasks/video-viewer-test/artifacts/video-${id}/content`,
    preview_url: `/v1/tasks/video-viewer-test/artifacts/video-${id}/content?disposition=inline`,
  })),
};
createRoot(document.getElementById("root")!).render(
  <StrictMode><main className="theme-shell min-h-screen p-4"><div className="mx-auto max-w-4xl">
    {params.get("view") === "chat" ? <TaskArtifactCard artifact={{ ...task.artifacts[0], schema_version: 2,
      artifact_ref: "artifact:task/video-viewer-test/video-1", sha256: "a".repeat(64), size_bytes: 1024 ** 3,
      preview_url: task.artifacts[0].preview_url ?? null }} artifactFetch={apiFetch} t={t} />
      : <AippTaskActivityCard item={task} apiFetch={apiFetch} t={t} lang={lang} />}
  </div></main></StrictMode>,
);
