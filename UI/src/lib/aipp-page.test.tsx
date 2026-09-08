import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import {
  AippCatalogCard,
  AippMediaItemCard,
  AippTaskActivityCard,
  localizedAippCopy,
  readCachedAippCatalog,
  readSelectedAipp,
  writeCachedAippCatalog,
} from "../components/AippPage";
import type { AippCatalogItem, AippMediaItem, AippTaskActivityItem } from "../types/api";

const t = (zh: string, _en: string) => zh;

test("selects AiPP presentation copy by locale with a declared fallback", () => {
  const values = { en: "Media Discovery", zh: "媒体发现" };
  assert.equal(localizedAippCopy(values, "zh", "en"), "媒体发现");
  assert.equal(localizedAippCopy({ en: values.en }, "zh", "en"), "Media Discovery");
});

test("restores the selected AiAPP after a browser refresh", () => {
  const storage = {
    getItem(key: string) {
      return key.endsWith("monitor.aipp.selectedSkill") ? " media_discovery " : null;
    },
  };
  assert.equal(readSelectedAipp(storage), "media_discovery");
  assert.equal(readSelectedAipp(undefined), "");
});

test("reuses only a fresh validated AiAPP catalog while refreshing in the background", () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
  };
  const app: AippCatalogItem = {
    skill_name: "example",
    package_version: "1.0.0",
    renderer: "collection_feed_v1",
    data_contract: "media_collection_v1",
    icon: "gallery_vertical_end",
    default_locale: "en",
    titles: { en: "Example" },
    descriptions: { en: "Example app" },
    installed: true,
    entrypoint: null,
    bridge_capabilities: [],
    task_channel_scope: null,
  };

  writeCachedAippCatalog(storage, [app], 1_000);
  assert.deepEqual(readCachedAippCatalog(storage, 1_001), [app]);
  assert.deepEqual(readCachedAippCatalog(storage, 1_000 + 5 * 60_000 + 1), []);

  writeCachedAippCatalog(storage, [app], 2_000);
  const key = [...values.keys()][0];
  values.set(key, JSON.stringify({ schema_version: 1, cached_at_ms: 2_000, apps: [{ ...app, installed: "yes" }] }));
  assert.deepEqual(readCachedAippCatalog(storage, 2_001), []);
});

test("keeps media collection automatically refreshed and sortable by collection time", () => {
  const source = readFileSync(new URL("../components/AippPage.tsx", import.meta.url), "utf8");
  assert.match(source, /const AIPP_AUTO_REFRESH_INTERVAL_MS = 10_000/);
  assert.match(source, /document\.visibilityState !== "visible"/);
  assert.match(source, /fetchPage\(true\)/);
  assert.match(source, /params\.set\("cursor_sequence", String\(cursor\)\)/);
  assert.match(source, /sort_order: sortOrder/);
  assert.match(source, /采集时间：最新优先/);
  assert.match(source, /采集时间：最早优先/);
});

test("renders collected media in two compact desktop columns", () => {
  const source = readFileSync(new URL("../components/AippPage.tsx", import.meta.url), "utf8");
  assert.match(source, /grid min-w-0 gap-2 lg:grid-cols-2/);
  assert.match(source, /sm:grid-cols-\[minmax\(104px,24%\)_minmax\(0,1fr\)\]/);
  assert.match(source, /max-h-36 min-h-24/);
});

test("opens both media renderers in a shared viewer before authenticated download", () => {
  const source = readFileSync(new URL("../components/AippPage.tsx", import.meta.url), "utf8");
  assert.match(source, /items\/\$\{item\.global_sequence\}\/preview/);
  assert.match(source, /filename: `media-/);
  assert.match(source, /放大图片/);
  assert.match(source, /if \(open && artifact.kind === "image"\)/);
  assert.equal((source.match(/<AippImageViewer/g) || []).length, 2);
  assert.doesNotMatch(source, /onClick=\{\(\) => void downloadImage\(\)\}/);
});

test("renders an installed AiPP as an application launcher card", () => {
  const app: AippCatalogItem = {
    skill_name: "media_discovery",
    package_version: "0.1.27",
    renderer: "collection_feed_v1",
    data_contract: "media_collection_v1",
    icon: "gallery_vertical_end",
    default_locale: "en",
    titles: { en: "Media Discovery", zh: "媒体发现" },
    descriptions: { en: "Collected media", zh: "查看采集内容" },
    installed: true,
    entrypoint: null,
    bridge_capabilities: [],
    task_channel_scope: null,
  };
  const markup = renderToStaticMarkup(
    <AippCatalogCard app={app} lang="zh" onOpen={() => undefined} onInstall={() => undefined} />,
  );
  assert.match(markup, /^<article/);
  assert.match(markup, /媒体发现/);
  assert.match(markup, /查看采集内容/);
});

test("keeps app switching in the launcher instead of duplicating apps inside a detail page", () => {
  const source = readFileSync(new URL("../components/AippPage.tsx", import.meta.url), "utf8");
  assert.match(source, /title=\{t\("返回应用列表", "Back to apps"\)\}/);
  assert.doesNotMatch(source, /role="tablist" aria-label="AiAPP"/);
});

test("offers Ai APP reinstallation without changing its skill", () => {
  const app: AippCatalogItem = {
    skill_name: "example",
    package_version: "1.0.0",
    renderer: "sandbox_bundle_v1",
    data_contract: "capability_bridge_v1",
    icon: "panels_top_left",
    default_locale: "en",
    titles: { en: "Example", zh: "示例" },
    descriptions: { en: "Example app", zh: "示例应用" },
    installed: false,
    entrypoint: "aipp/index.html",
    bridge_capabilities: [],
    task_channel_scope: null,
  };
  const markup = renderToStaticMarkup(
    <AippCatalogCard app={app} lang="zh" onOpen={() => undefined} onInstall={() => undefined} />,
  );
  assert.match(markup, /安装 Ai APP/);
  assert.doesNotMatch(markup, /chevron-right/);
});

test("loads sandbox bundles through an opaque iframe and a capability allowlist", () => {
  const source = readFileSync(new URL("../components/AippPage.tsx", import.meta.url), "utf8");
  assert.match(source, /sandbox="allow-scripts allow-downloads"/);
  assert.doesNotMatch(source, /allow-same-origin/);
  assert.match(source, /allowedCapabilities\.has\(request\.capability\)/);
  assert.match(source, /event\.source !== frameRef\.current\?\.contentWindow/);
  assert.match(source, /type === "aipp.ready"/);
  assert.match(source, /entrypoint: "run_capability"/);
  assert.match(source, /AIPP_BRIDGE_MAX_IN_FLIGHT = 4/);
  assert.match(source, /AIPP_BRIDGE_MAX_ARGS_BYTES = 64 \* 1024/);
});

test("keeps Ai APP installation state separate from its skill", () => {
  const source = readFileSync(new URL("../components/AippPage.tsx", import.meta.url), "utf8");
  assert.match(source, /method: installed \? "POST" : "DELETE"/);
  assert.match(source, /对应技能、配置和采集数据都会保留/);
  assert.match(source, /安装 Ai APP/);
});

test("renders a media collection record from the current no-OCR contract", () => {
  const item: AippMediaItem = {
    schema_version: 1,
    global_sequence: 42,
    sequence: 7,
    post_sequence: 3,
    image_sequence: 2,
    kind: "image",
    platform: "xiaohongshu",
    source_mode: "topics",
    search_keyword: "example",
    title: "Collected title",
    platform_text: "Platform copy",
    source_url: "https://example.test/source",
    image_url: "https://example.test/image.webp",
    preview_available: false,
    discovered_at: "2026-09-07T00:00:00Z",
    engagement: {
      schema_version: 1,
      platform: "xiaohongshu",
      captured_at: "2026-09-07T00:00:00Z",
      metrics: {
        likes: { display: "1.2万", value: null },
        comments: { display: "318", value: 318 },
      },
    },
  };
  const markup = renderToStaticMarkup(
    <AippMediaItemCard
      item={item}
      skillName="media_discovery"
      apiFetch={async () => new Response()}
      t={t}
      lang="zh"
    />,
  );
  assert.match(markup, /Collected title/);
  assert.match(markup, /帖子文案/);
  assert.match(markup, /Platform copy/);
  assert.doesNotMatch(markup, /图片文字/);
  assert.match(markup, /xiaohongshu/);
  assert.match(markup, /1\.2万/);
  assert.match(markup, /318/);
  assert.match(markup, /href="https:\/\/example\.test\/source"/);
  assert.match(markup, /referrerPolicy="no-referrer"/);
});

test("renders an image card when only a retained local preview is available", () => {
  const item: AippMediaItem = {
    schema_version: 1,
    global_sequence: 43,
    sequence: 8,
    post_sequence: 4,
    image_sequence: 1,
    kind: "image",
    platform: "xiaohongshu",
    source_mode: "home_feed",
    search_keyword: "",
    title: "Local preview",
    platform_text: "",
    source_url: null,
    image_url: null,
    preview_available: true,
    discovered_at: "2026-09-08T00:00:00Z",
    engagement: null,
  };
  const markup = renderToStaticMarkup(
    <AippMediaItemCard
      item={item}
      skillName="media_discovery"
      apiFetch={async () => new Response()}
      t={t}
      lang="en"
    />,
  );
  assert.match(markup, /Local preview/);
  assert.match(markup, /暂无预览/);
});

test("renders a video cover record without a visual-text section", () => {
  const item: AippMediaItem = {
    schema_version: 1,
    global_sequence: 44,
    sequence: 9,
    post_sequence: 5,
    image_sequence: null,
    kind: "video",
    platform: "douyin",
    source_mode: "home_feed",
    search_keyword: "",
    title: "Video title",
    platform_text: "Author caption",
    source_url: "https://example.test/video",
    image_url: null,
    preview_available: true,
    discovered_at: "2026-09-08T00:00:00Z",
    engagement: null,
  };
  const markup = renderToStaticMarkup(
    <AippMediaItemCard
      item={item}
      skillName="media_discovery"
      apiFetch={async () => new Response()}
      t={t}
      lang="zh"
    />,
  );
  assert.match(markup, /帖子文案/);
  assert.match(markup, /Author caption/);
  assert.doesNotMatch(markup, /画面文字/);
});

test("renders cross-channel media task input, processed content, links, and safe artifacts", () => {
  const item: AippTaskActivityItem = {
    schema_version: 1,
    sequence: 9,
    task_id: "12345678-activity-task",
    channel: "wechat",
    status: "succeeded",
    actions: ["media_download.download", "media_download.transcribe"],
    input_text: "下载并转写 https://media.example.test/post/1",
    result_text: "整理后的完整转写内容。",
    error_text: null,
    source_urls: ["https://media.example.test/post/1"],
    artifacts: [{
      schema_version: 1,
      id: "artifact-1",
      filename: "transcript.txt",
      kind: "file",
      mime_type: "text/plain",
      size_bytes: 2048,
      download_url: "/v1/tasks/12345678-activity-task/artifacts/artifact-1/content",
      preview_url: null,
    }],
    created_at: "1788846460",
    updated_at: "1788846461",
    event_at_ms: 1788846461000,
  };
  const markup = renderToStaticMarkup(
    <AippTaskActivityCard
      item={item}
      apiFetch={async () => new Response()}
      t={t}
      lang="zh"
    />,
  );
  assert.match(markup, /微信/);
  assert.match(markup, /原始请求/);
  assert.match(markup, /处理结果/);
  assert.match(markup, /整理后的完整转写内容/);
  assert.match(markup, /媒体链接 1/);
  assert.match(markup, /transcript\.txt/);
  assert.match(markup, /2\.0 KB/);
  assert.match(markup, />download</);
  assert.match(markup, />transcribe</);
});

test("supports the generic task activity AiAPP renderer without skill-specific core UI branches", () => {
  const source = readFileSync(new URL("../components/AippPage.tsx", import.meta.url), "utf8");
  assert.match(source, /selectedApp\.renderer === "task_activity_v1"/);
  assert.match(source, /params\.set\("channel", activityChannel\)/);
  assert.match(source, /params\.set\("status", activityStatus\)/);
  assert.match(source, /搜索原始请求或处理结果/);
  assert.doesNotMatch(source, /selectedSkill === "media_download"/);
});
