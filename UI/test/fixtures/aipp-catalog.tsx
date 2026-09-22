import { createRoot } from "react-dom/client";
import { AippCatalogGrid } from "../../src/components/AippPage";
import type { AippCatalogItem } from "../../src/types/api";
import "../../src/index.css";

const params = new URLSearchParams(location.search);
const lang = params.get("lang") === "zh" ? "zh" : "en";
document.documentElement.dataset.theme = params.get("theme") || "dark";
const titles = [
  { zh: "媒体采集", en: "Media Collection" },
  { zh: "媒体整理", en: "Media Organizer" },
  { zh: "我的资料", en: "My Library" },
  { zh: "一个较长的应用名称用于检查自动换行", en: "An application with a longer name" },
  { zh: "阅读记录", en: "Reading History" },
  { zh: "收藏", en: "Bookmarks" },
  { zh: "待安装应用", en: "Available App" },
];
const apps: AippCatalogItem[] = titles.map((title, index) => ({
  skill_name: `example_app_${index}`, package_version: "1.0.0",
  renderer: "collection_feed_v1", data_contract: "media_collection_v1",
  icon: ["gallery_vertical_end", "download", "panels_top_left"][index % 3],
  default_locale: "en", titles: title,
  descriptions: { zh: "查看已保存的内容。", en: "View saved content." },
  installed: index !== titles.length - 1, entrypoint: null,
  bridge_capabilities: [], task_channel_scope: null,
}));

createRoot(document.getElementById("root")!).render(
  <main className="theme-shell min-h-screen p-6 sm:p-8">
    <header className="mb-6">
      <p className="text-xs text-[var(--theme-text-muted)]">AiAPP</p>
      <h1 className="mt-1 text-xl font-semibold text-[var(--theme-text-strong)]">{lang === "zh" ? "应用" : "Apps"}</h1>
    </header>
    <AippCatalogGrid apps={apps} lang={lang}
      onOpen={(app) => { document.documentElement.dataset.openedApp = app.skill_name; }}
      onInstall={(app) => { document.documentElement.dataset.installedApp = app.skill_name; }} />
  </main>,
);
