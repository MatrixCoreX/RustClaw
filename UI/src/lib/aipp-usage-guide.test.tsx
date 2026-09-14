import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { AippUsageGuide } from "../components/AippUsageGuide";
import type { AippCatalogItem } from "../types/api";

const app: AippCatalogItem = {
  skill_name: "any_collection", package_version: "1.0.0", renderer: "collection_feed_v1",
  data_contract: "media_collection_v1", icon: "gallery_vertical_end", default_locale: "en",
  titles: { en: "Custom collection" }, descriptions: {}, installed: true,
  entrypoint: null, bridge_capabilities: [], task_channel_scope: null,
};
const render = (item: AippCatalogItem, lang = "en", title = "Custom collection") => renderToStaticMarkup(<AippUsageGuide app={item} title={title} description="Package description" platforms={[]} t={(zh, en) => lang === "zh" ? zh : en} onOpenAgent={() => { throw new Error("must not execute"); }} />);
test("collection guide derives its identity from the package and covers start, progress and stop", () => {
  const text = render(app);
  for (const label of ["Custom collection", "Try a small collection", "Search by keyword", "Keep collecting", "Check current status", "Stop and confirm", "Why are results missing", "ZIP"]) assert.ok(text.includes(label), label);
  assert.ok(text.includes("[platform name]"));
  assert.doesNotMatch(text, /media_discovery|media_download/);
});
test("task guide keeps manual media requests separate from background collection", () => {
  const text = render({ ...app, renderer: "task_activity_v1", data_contract: "skill_task_activity_v1", task_channel_scope: "all" });
  assert.ok(text.includes("Download media")); assert.ok(text.includes("Extract text"));
  assert.ok(text.includes("UI Agent and linked channels"));
  assert.ok(text.includes("canceled")); assert.ok(!text.includes("Keep collecting in the background"));
});
test("channel-only activity guide does not promise UI task visibility", () => {
  const text = render({ ...app, renderer: "task_activity_v1", data_contract: "skill_task_activity_v1", task_channel_scope: "communication" });
  assert.ok(text.includes("channel tasks only")); assert.ok(!text.includes("UI Agent and linked channels"));
});
test("unknown application contracts get a generic guide, not fabricated collection capabilities", () => {
  const text = render({ ...app, renderer: "sandbox_bundle_v1", data_contract: "capability_bridge_v1" });
  assert.ok(text.includes("Check before execution")); assert.ok(!text.includes("recommended posts"));
});
test("Chinese guide includes graceful stop, saved-data preservation and explicit placeholders", () => {
  const text = render(app, "zh", "自定义应用");
  for (const label of ["开始前准备", "【平台名称】", "【搜索关键词】", "完成收尾", "保留已经采集的内容", "不会自行发送任务"]) assert.ok(text.includes(label), label);
});
test("package-supplied labels are rendered as text, never executable markup", () => {
  const text = render(app, "en", '<img src=x onerror="alert(1)">');
  assert.ok(text.includes("&lt;img")); assert.doesNotMatch(text, /<img src=x/);
});
