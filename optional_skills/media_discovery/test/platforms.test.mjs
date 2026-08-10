import assert from "node:assert/strict";
import test from "node:test";

import {
  canonicalCandidateUrls,
  isDetailUrl,
  sourceTargets,
  sourceUrls,
  validatePlatformUrl,
} from "../src/platforms.mjs";
import { detailNavigationError, renderedCardMediaKind } from "../src/browser.mjs";

test("platform URLs are validated structurally", () => {
  assert.equal(
    validatePlatformUrl("douyin", "https://www.douyin.com/video/123#comment"),
    "https://www.douyin.com/video/123",
  );
  assert.throws(() => validatePlatformUrl("douyin", "http://127.0.0.1/video/123"));
  assert.throws(() => validatePlatformUrl("xiaohongshu", "https://example.com/explore/1"));
});

test("candidate discovery uses URL contracts rather than page language", () => {
  const values = canonicalCandidateUrls("xiaohongshu", [
    "https://www.xiaohongshu.com/explore/abc",
    "https://www.xiaohongshu.com/explore/abc",
    "https://www.xiaohongshu.com/user/profile/abc",
  ]);
  assert.deepEqual(values, ["https://www.xiaohongshu.com/explore/abc"]);
  assert.equal(isDetailUrl("douyin", "https://www.douyin.com/video/123"), true);
});

test("home feed and topic sources are explicit schema modes", () => {
  assert.deepEqual(sourceUrls("douyin", { source_mode: "home_feed" }), ["https://www.douyin.com/"]);
  assert.deepEqual(sourceUrls("xiaohongshu", { source_mode: "topics", topics: ["AI agent"] }), [
    "https://www.xiaohongshu.com/search_result?keyword=AI%20agent",
  ]);
});

test("keyword search targets preserve structured keyword order and platform search provenance", () => {
  assert.deepEqual(sourceTargets("douyin", {
    source_mode: "topics",
    topics: ["AI agent", "咖啡 店"],
  }), [
    {
      source_mode: "topics",
      search_keyword: "AI agent",
      url: "https://www.douyin.com/search/AI%20agent",
    },
    {
      source_mode: "topics",
      search_keyword: "咖啡 店",
      url: "https://www.douyin.com/search/%E5%92%96%E5%95%A1%20%E5%BA%97",
    },
  ]);
  assert.throws(
    () => sourceTargets("xiaohongshu", { source_mode: "topics", topics: ["  "] }),
    /source_scope_empty/u,
  );
});

test("rendered card classification distinguishes a video poster from an image carousel", () => {
  assert.equal(renderedCardMediaKind({ visibleVideoCount: 1, visibleImageCount: 2, hasImageCarousel: true }), "video");
  assert.equal(renderedCardMediaKind({ visibleVideoCount: 0, visibleImageCount: 1, hasImageCarousel: false }), "video");
  assert.equal(renderedCardMediaKind({ visibleVideoCount: 0, visibleImageCount: 2, hasImageCarousel: false }), "image");
  assert.equal(renderedCardMediaKind({ visibleVideoCount: 0, visibleImageCount: 1, hasImageCarousel: true }), "image");
});

test("detail navigation rejects login redirects without inspecting page language", () => {
  const note = "https://www.xiaohongshu.com/explore/64a123456789012345678901";
  assert.equal(detailNavigationError("xiaohongshu", note, note, false), null);
  assert.equal(
    detailNavigationError("xiaohongshu", note, "https://www.xiaohongshu.com/explore", true),
    "login_required",
  );
  assert.equal(
    detailNavigationError("xiaohongshu", note, "https://www.xiaohongshu.com/explore", false),
    "login_required",
  );
  assert.equal(
    detailNavigationError("douyin", "https://www.douyin.com/video/123", "https://www.douyin.com/", false),
    "challenge_required",
  );
});
