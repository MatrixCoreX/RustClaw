import assert from "node:assert/strict";
import test from "node:test";

import { csvCell, renderCsv, VIDEO_COLUMNS } from "../src/csv.mjs";

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
    recognized_text: "日本語 العربية",
    cover_screenshot_path: "video_covers/douyin_1.png",
    video_page_url: "https://www.douyin.com/video/1",
    discovered_at: "2026-08-10T00:00:00Z",
  }]);
  assert.ok(rendered.startsWith("\uFEFF"));
  assert.ok(rendered.includes('"标题, ""quoted"""'));
  assert.ok(rendered.includes('"第一行\n第二行"'));
  assert.ok(rendered.includes('"video_covers/douyin_1.png"'));
  assert.ok(rendered.includes('"AI agent"'));
  assert.ok(rendered.includes('"https://www.douyin.com/search/AI%20agent"'));
  assert.ok(rendered.endsWith("\r\n"));
});

test("CSV protects spreadsheet formula prefixes without changing normal text", () => {
  assert.equal(csvCell("=1+1"), '"\'=1+1"');
  assert.equal(csvCell("  @command"), '"\'  @command"');
  assert.equal(csvCell("ordinary"), '"ordinary"');
});
