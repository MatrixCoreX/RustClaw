import assert from "node:assert/strict";
import test from "node:test";

import {
  canonicalCandidateUrls,
  isDetailUrl,
  manualVerificationTarget,
  matchesVerificationTarget,
  sourceTargets,
  sourceUrls,
  validatePlatformUrl,
} from "../src/platforms.mjs";
import {
  accessErrorAfterExplicitVisibleWait,
  detailNavigationError,
  platformAccessError,
  renderedCardMediaKind,
  xiaohongshuFeedCardMediaKind,
} from "../src/browser.mjs";

test("manual verification retains the selected keyword or blocked detail instead of the home feed", () => {
  const config = { source_mode: "topics", topics: ["财经"] };
  for (const platform of ["douyin", "xiaohongshu", "kuaishou"]) {
    const target = sourceTargets(platform, config)[0].url;
    assert.equal(manualVerificationTarget(platform, config), target);
    assert.equal(matchesVerificationTarget(platform, target, target), true);
    assert.equal(matchesVerificationTarget(platform, target, sourceTargets(platform, { source_mode: "home_feed" })[0].url), false);
    assert.equal(matchesVerificationTarget(platform, target, sourceTargets(platform, { ...config, topics: ["other"] })[0].url), false);
    assert.throws(() => manualVerificationTarget(platform, config, "https://example.com/"));
  }
  const detail = "https://www.xiaohongshu.com/explore/fixture?xsec_token=fixture-token";
  assert.equal(manualVerificationTarget("xiaohongshu", config, detail), detail);
  assert.equal(detailNavigationError("xiaohongshu", detail, "https://www.xiaohongshu.com/404", false), "source_unavailable");
});

test("Xiaohongshu search UI routes retain the exact keyword through their URL encoding", () => {
  for (const keyword of ["财经", "AI agent", "%20", "a+b & c"]) {
    const target = sourceTargets("xiaohongshu", { source_mode: "topics", topics: [keyword] })[0].url;
    const current = `https://www.xiaohongshu.com/search_result_ai?keyword=${encodeURIComponent(encodeURIComponent(keyword))}&source=web_explore_feed`;
    assert.equal(matchesVerificationTarget("xiaohongshu", target, current), true);
    assert.equal(matchesVerificationTarget("xiaohongshu", current, current), true);
    assert.equal(matchesVerificationTarget("xiaohongshu", target, current.replace("search_result_ai", "explore")), false);
  }
});

test("Xiaohongshu search detail links preserve page-provided parameters", () => {
  const url = "https://www.xiaohongshu.com/search_result/1234abcd?xsec_token=fixture&xsec_source=pc_search";
  assert.equal(isDetailUrl("xiaohongshu", url), true);
  assert.deepEqual(canonicalCandidateUrls("xiaohongshu", [url, url]), [url]);
  assert.equal(isDetailUrl("xiaohongshu", "https://www.xiaohongshu.com/search_result?keyword=fixture"), false);
});

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
  assert.equal(isDetailUrl("kuaishou", "https://www.kuaishou.com/short-video/3xexample123"), true);
  assert.equal(isDetailUrl("kuaishou", "https://www.kuaishou.com/short-video/%E7%83%AD%E6%90%9C%E8%AF%8D"), false);
});

test("home feed and topic sources are explicit schema modes", () => {
  assert.deepEqual(sourceUrls("douyin", { source_mode: "home_feed" }), ["https://www.douyin.com/"]);
  assert.deepEqual(sourceUrls("xiaohongshu", { source_mode: "topics", topics: ["AI agent"] }), [
    "https://www.xiaohongshu.com/search_result?keyword=AI%20agent",
  ]);
  assert.deepEqual(sourceUrls("kuaishou", { source_mode: "home_feed" }), [
    "https://www.kuaishou.com/brilliant",
  ]);
  assert.deepEqual(sourceUrls("kuaishou", { source_mode: "topics", topics: ["AI agent"] }), [
    "https://www.kuaishou.com/search/AI%20agent",
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

test("Xiaohongshu feed cards use the structural play control as their media kind", () => {
  assert.equal(xiaohongshuFeedCardMediaKind(true), "video");
  assert.equal(xiaohongshuFeedCardMediaKind(false), "image");
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

test("platform access checks classify machine challenge surfaces without page-language matching", () => {
  assert.equal(
    platformAccessError(
      "xiaohongshu",
      "https://www.xiaohongshu.com/website-login/error?error_code=300012",
    ),
    "network_access_restricted",
  );
  assert.equal(platformAccessError("xiaohongshu",
    "https://www.xiaohongshu.com/website-login/error?error_code=999999"), "challenge_required");
  assert.equal(
    platformAccessError("douyin", "https://www.douyin.com/", [
      "https://rmc.bytedance.com/verifycenter/captcha/v2?scene_level=p2",
    ]),
    "challenge_required",
  );
  assert.equal(
    platformAccessError("douyin", "https://www.douyin.com/jingxuan", [
      "https://lf-zt.douyin.com/obj/static/player.html",
    ]),
    null,
  );
  assert.equal(platformAccessError("kuaishou", "https://www.kuaishou.com/brilliant"), null);
});

test("network restrictions do not wait for a nonexistent manual captcha", async () => {
  const page = {
    locator: () => ({ evaluateAll: async () => [] }),
    url: () => "https://www.xiaohongshu.com/website-login/error?error_code=300012",
    waitForTimeout: () => { throw new Error("must not wait for manual verification"); },
  };
  assert.equal(await accessErrorAfterExplicitVisibleWait(page, "xiaohongshu", {
    browser_mode: "visible",
  }), "network_access_restricted");
});

test("only an explicitly visible run waits for a machine challenge to clear", async () => {
  const captcha = "https://rmc.bytedance.com/verifycenter/captcha/v2?scene_level=p2";
  let scans = 0;
  let waits = 0;
  const page = {
    isClosed: () => false,
    locator: () => ({
      evaluateAll: async () => (scans++ === 0 ? [captcha] : []),
      count: async () => 0,
    }),
    url: () => "https://www.douyin.com/",
    waitForTimeout: async () => { waits += 1; },
  };
  assert.equal(await accessErrorAfterExplicitVisibleWait(page, "douyin", {
    browser_mode: "visible",
    max_run_minutes: 5,
  }), null);
  assert.equal(waits, 2);

  scans = 0;
  waits = 0;
  assert.equal(await accessErrorAfterExplicitVisibleWait(page, "douyin", {
    browser_mode: "silent",
  }), "challenge_required");
  assert.equal(waits, 0);
  scans = 0;
  await assert.rejects(accessErrorAfterExplicitVisibleWait(page, "douyin", {
    browser_mode: "visible",
  }, async () => true), { message: "collection_stopped" });
});

test("visible verification requires consecutive clear observations", async () => {
  const captcha = "https://rmc.bytedance.com/verifycenter/captcha/v2";
  const frames = [[captcha], [], [captcha], [], []];
  let waits = 0;
  const page = {
    isClosed: () => false,
    locator: () => ({ evaluateAll: async () => frames.shift() || [], count: async () => 0 }),
    url: () => "https://www.douyin.com/",
    waitForTimeout: async () => { waits += 1; },
  };
  assert.equal(await accessErrorAfterExplicitVisibleWait(page, "douyin", { browser_mode: "visible" }), null);
  assert.equal(waits, 4);
});
