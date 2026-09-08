import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { AippMediaItemCard, AippTaskActivityCard } from "../../src/components/AippPage";
import type { AippMediaItem, AippTaskActivityItem } from "../../src/types/api";
import "../../src/index.css";

const params = new URLSearchParams(location.search);
const lang = params.get("lang") === "zh" ? "zh" : "en";
document.documentElement.lang = lang;
document.documentElement.dataset.theme = params.get("theme") || "dark";
const t = (zh: string, en: string) => lang === "zh" ? zh : en;
const apiFetch = (path: string, init?: RequestInit) => fetch(path, init);
const media: AippMediaItem = {
  schema_version: 1, global_sequence: 1, sequence: 1, post_sequence: 1,
  image_sequence: 1, kind: "image", platform: "example", source_mode: "home_feed",
  search_keyword: "", title: t("采集的图片与帖子文案", "Collected image and post caption"),
  platform_text: t("保留帖子原有文案。", "The original post caption is retained."),
  source_url: null, image_url: null, preview_available: true,
  discovered_at: "2026-09-09T00:00:00Z", engagement: null,
};
const task: AippTaskActivityItem = {
  schema_version: 1, sequence: 1, task_id: "image-viewer-test", channel: "wechat",
  status: "succeeded", actions: ["example.download"],
  input_text: t("下载这些图片", "Download these images"),
  result_text: t("已保存图片和文本。", "Images and text saved."), error_text: null,
  source_urls: [], created_at: "1788912000", updated_at: "1788912000", event_at_ms: 1788912000000,
  artifacts: [
    { schema_version: 1, id: "portrait", filename: "portrait.png", kind: "image", mime_type: "image/png", size_bytes: 1024, preview_url: "/fixture/portrait/preview", download_url: "/fixture/portrait/download" },
    { schema_version: 1, id: "landscape", filename: "landscape-with-a-very-long-filename-that-must-not-displace-the-close-button.png", kind: "image", mime_type: "image/png", size_bytes: 2048, preview_url: "/fixture/landscape/preview", download_url: "/fixture/landscape/download" },
    { schema_version: 1, id: "text", filename: "transcript.txt", kind: "file", mime_type: "text/plain", size_bytes: 8, preview_url: "/fixture/text/preview", download_url: "/fixture/text/download" },
  ],
};

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <main className="theme-shell min-h-screen p-4">
      <div className="mx-auto grid max-w-5xl gap-4">
        <section data-testid="collection"><AippMediaItemCard item={media} skillName="example_collection" apiFetch={apiFetch} t={t} lang={lang} /></section>
        <section data-testid="cover"><AippMediaItemCard item={{ ...media, global_sequence: 2, kind: "video" }} skillName="example_collection" apiFetch={apiFetch} t={t} lang={lang} /></section>
        <section data-testid="activity"><AippTaskActivityCard item={task} apiFetch={apiFetch} t={t} lang={lang} /></section>
      </div>
    </main>
  </StrictMode>,
);
