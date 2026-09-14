import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { AippUsageGuide } from "../../src/components/AippUsageGuide";
import type { AippCatalogItem } from "../../src/types/api";
import "../../src/index.css";
const params = new URLSearchParams(location.search);
const lang = params.get("lang") === "zh" ? "zh" : "en";
document.documentElement.dataset.theme = params.get("theme") || "dark";
const activity = params.get("kind") === "activity";
const app: AippCatalogItem = {
  skill_name: "example_app", package_version: "1.0.0", renderer: activity ? "task_activity_v1" : "collection_feed_v1",
  data_contract: activity ? "skill_task_activity_v1" : "media_collection_v1", icon: "gallery_vertical_end", default_locale: "en",
  titles: {}, descriptions: {}, installed: true, entrypoint: null, bridge_capabilities: [], task_channel_scope: activity ? "all" : null,
};
createRoot(document.getElementById("root")!).render(<StrictMode><main className="theme-shell min-h-screen p-4"><AippUsageGuide app={app} title={lang === "zh" ? "采集应用" : "Collection app"} description="" platforms={["example"]} t={(zh, en) => lang === "zh" ? zh : en} onOpenAgent={() => { document.documentElement.dataset.agentOpened = "true"; }} /></main></StrictMode>);
