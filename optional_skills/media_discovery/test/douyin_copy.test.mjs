import assert from "node:assert/strict";
import test from "node:test";

import { douyinRecordCopy, screenshotLooksBlankFromBytes } from "../src/browser.mjs";

test("Douyin records keep the author title and leave caption empty without a distinct body", () => {
  assert.deepEqual(douyinRecordCopy({
    authorTitle: "年入十万真的很难的么？ #财经 #兜姐财经 #年入10万",
    listingTitle: "发现更多精彩视频 - 抖音搜索",
  }), {
    title: "年入十万真的很难的么？ #财经 #兜姐财经 #年入10万",
    platform_text: "",
  });
});

test("Douyin search listing chrome is not stored as the post title", () => {
  assert.deepEqual(douyinRecordCopy({
    authorTitle: "",
    listingTitle: "发现更多精彩视频 - 抖音搜索",
  }), { title: "", platform_text: "" });
});

test("blank cover bytes are rejected while a detailed PNG is kept", () => {
  assert.equal(screenshotLooksBlankFromBytes(Buffer.alloc(0)), true);
  assert.equal(screenshotLooksBlankFromBytes(Buffer.from("not-a-png")), true);
  const header = (width, height, length) => {
    const buffer = Buffer.alloc(Math.max(24, length));
    buffer[0] = 0x89;
    buffer.write("PNG", 1, "ascii");
    buffer.writeUInt32BE(width, 16);
    buffer.writeUInt32BE(height, 20);
    return buffer;
  };
  assert.equal(screenshotLooksBlankFromBytes(header(1280, 853, 5023)), true);
  assert.equal(screenshotLooksBlankFromBytes(header(240, 320, 80_000)), false);
});
