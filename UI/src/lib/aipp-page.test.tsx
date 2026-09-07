import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { AippMediaItemCard, localizedAippCopy } from "../components/AippPage";
import type { AippMediaItem } from "../types/api";

const t = (zh: string, _en: string) => zh;

test("selects AiPP presentation copy by locale with a declared fallback", () => {
  const values = { en: "Media Discovery", zh: "媒体发现" };
  assert.equal(localizedAippCopy(values, "zh", "en"), "媒体发现");
  assert.equal(localizedAippCopy({ en: values.en }, "zh", "en"), "Media Discovery");
});

test("renders a media collection record without exposing undeclared fields", () => {
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
    recognized_text: "Recognized copy",
    recognition_source: "local_ocr",
    source_url: "https://example.test/source",
    image_url: "https://example.test/image.webp",
    preview_available: false,
    discovered_at: "2026-09-07T00:00:00Z",
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
  assert.match(markup, /Recognized copy/);
  assert.match(markup, /xiaohongshu/);
  assert.match(markup, /href="https:\/\/example\.test\/source"/);
  assert.match(markup, /referrerPolicy="no-referrer"/);
});
