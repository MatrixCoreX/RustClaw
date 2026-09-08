import assert from "node:assert/strict";
import test from "node:test";

import { csvCell, IMAGE_COLUMNS, renderCsv, VIDEO_COLUMNS } from "../src/csv.mjs";

test("CSV uses BOM, CRLF, RFC 4180 quoting, and preserves multilingual newlines", () => {
  const rendered = renderCsv(VIDEO_COLUMNS, [{
    sequence: 1,
    global_sequence: 2,
    platform: "douyin",
    source_mode: "topics",
    search_keyword: "AI agent",
    discovery_source_url: "https://www.douyin.com/search/AI%20agent",
    title: '标题, "quoted"',
    platform_text: "第一行\n第二行",
    cover_screenshot_path: "video_covers/douyin_1.png",
    video_page_url: "https://www.douyin.com/video/1",
    discovered_at: "2026-08-10T00:00:00Z",
    engagement: {
      captured_at: "2026-08-10T00:00:00Z",
      metrics: {
        views: { display: "1.2万" },
        likes: { display: "318", value: 318 },
      },
    },
  }]);
  assert.ok(rendered.startsWith("\uFEFF"));
  assert.ok(rendered.includes('"标题, ""quoted"""'));
  assert.ok(rendered.includes('"第一行\n第二行"'));
  assert.ok(rendered.includes('"video_covers/douyin_1.png"'));
  assert.ok(rendered.includes('"AI agent"'));
  assert.ok(rendered.includes('"https://www.douyin.com/search/AI%20agent"'));
  assert.ok(rendered.includes('"1.2万","318"'));
  assert.equal(VIDEO_COLUMNS.includes("recognized_text"), false);
  assert.ok(rendered.endsWith("\r\n"));
});

test("CSV protects spreadsheet formula prefixes without changing normal text", () => {
  assert.equal(csvCell("=1+1"), '"\'=1+1"');
  assert.equal(csvCell("  @command"), '"\'  @command"');
  assert.equal(csvCell("ordinary"), '"ordinary"');
});

test("image CSV retains the authenticated-download screenshot path", () => {
  const rendered = renderCsv(IMAGE_COLUMNS, [{
    sequence: 1,
    global_sequence: 1,
    kind: "image",
    image_url: "https://images.example.test/source.webp",
    image_screenshot_path: "images/xiaohongshu_fixture_001.png",
  }]);
  assert.ok(rendered.includes('"image_screenshot_path"'));
  assert.ok(rendered.includes('"images/xiaohongshu_fixture_001.png"'));
  assert.equal(IMAGE_COLUMNS.includes("recognized_text"), false);
});
