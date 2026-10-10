import assert from "node:assert/strict";
import test from "node:test";

import {
  canonicalCandidateUrls,
  canonicalTikTokResultUrl,
  canonicalToutiaoResultUrl,
  canonicalWeiboResultUrl,
  canonicalYouTubeResultUrl,
  douyinModalItemId,
  douyinSearchGridItemUrls,
  isDetailUrl,
  manualVerificationTarget,
  matchesManualAccessTarget,
  matchesVerificationTarget,
  platformItemId,
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
import { searchResultLinkIndex } from "../src/browser_search_results.mjs";

test("manual verification retains the selected keyword or blocked detail instead of the home feed", () => {
  const config = { source_mode: "topics", topics: ["财经"] };
  for (const platform of ["douyin", "xiaohongshu", "kuaishou", "toutiao", "weibo", "tiktok", "youtube"]) {
    const target = sourceTargets(platform, config)[0].url;
    const home = sourceTargets(platform, { source_mode: "home_feed" })[0].url;
    assert.equal(manualVerificationTarget(platform, config), target);
    assert.equal(matchesVerificationTarget(platform, target, target), true);
    assert.equal(matchesVerificationTarget(platform, target, home), false);
    assert.equal(matchesVerificationTarget(platform, target, sourceTargets(platform, { ...config, topics: ["other"] })[0].url), false);
    assert.throws(() => manualVerificationTarget(platform, config, "https://example.com/"));
  }
  assert.equal(matchesVerificationTarget("douyin", "https://www.douyin.com/", "https://www.douyin.com/jingxuan"), true);
  assert.equal(matchesVerificationTarget("douyin", "https://www.douyin.com/search/finance", "https://www.douyin.com/jingxuan/search/finance"), true);
  assert.equal(matchesVerificationTarget("douyin", "https://www.douyin.com/search/%E8%B4%A2%E7%BB%8F", "https://www.douyin.com/jingxuan/search/%E8%B4%A2%E7%BB%8F"), true);
  assert.equal(matchesVerificationTarget("douyin", "https://www.douyin.com/search/finance", "https://www.douyin.com/jingxuan"), false);
  assert.equal(matchesManualAccessTarget("douyin", "https://www.douyin.com/search/finance", "https://www.douyin.com/jingxuan"), true);
  assert.equal(matchesManualAccessTarget("douyin", "https://www.douyin.com/", "https://www.douyin.com/jingxuan"), true);
  assert.equal(matchesManualAccessTarget("douyin", "https://www.douyin.com/search/finance", "https://www.douyin.com/video/1234"), false);
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

test("Douyin jingxuan search cards expose numeric item IDs as video URLs", () => {
  assert.deepEqual(douyinSearchGridItemUrls(["7684941496998743331", "bad", "7684941496998743331", ""]), [
    "https://www.douyin.com/video/7684941496998743331",
  ]);
});

test("Douyin search overlays identify the open post through modal_id", () => {
  const search = "https://www.douyin.com/jingxuan/search/%E8%B4%A2%E7%BB%8F?aid=123&type=general";
  const overlay = `${search}&modal_id=7685283494558209137`;
  const video = "https://www.douyin.com/video/7685283494558209137";
  assert.equal(douyinModalItemId(overlay), "7685283494558209137");
  assert.equal(douyinModalItemId(search), null);
  assert.equal(isDetailUrl("douyin", overlay), true);
  assert.equal(isDetailUrl("douyin", search), false);
  assert.equal(platformItemId("douyin", overlay), "douyin:7685283494558209137");
  assert.equal(platformItemId("douyin", video), "douyin:7685283494558209137");
  assert.equal(matchesVerificationTarget("douyin", search, overlay), true);
  assert.deepEqual(canonicalCandidateUrls("douyin", [overlay, video, search]), [overlay]);
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
  for (const kind of ["article", "video", "w"]) {
    const url = `https://www.toutiao.com/${kind}/7694557030652084736/`;
    assert.equal(isDetailUrl("toutiao", url), true);
    assert.equal(platformItemId("toutiao", url), "toutiao:7694557030652084736");
  }
  assert.equal(isDetailUrl("toutiao", "https://www.toutiao.com/c/user/token/example/"), false);
  assert.equal(isDetailUrl("weibo", "https://weibo.com/2286908003/RlQuA7sqn"), true);
  assert.equal(platformItemId("weibo", "https://weibo.com/2286908003/RlQuA7sqn"), "weibo:5352066721777563");
  assert.equal(platformItemId("weibo", "https://m.weibo.cn/status/5352066721777563"), "weibo:5352066721777563");
  assert.equal(isDetailUrl("weibo", "https://weibo.com/u/2286908003"), false);
  assert.equal(isDetailUrl("tiktok", "https://www.tiktok.com/@creator/video/7512345678901234567"), true);
  assert.equal(platformItemId("tiktok", "https://www.tiktok.com/@creator/video/7512345678901234567"), "tiktok:7512345678901234567");
  assert.equal(isDetailUrl("youtube", "https://www.youtube.com/watch?v=dQw4w9WgXcQ"), true);
  assert.equal(isDetailUrl("youtube", "https://www.youtube.com/shorts/dQw4w9WgXcQ"), true);
  assert.equal(platformItemId("youtube", "https://youtu.be/dQw4w9WgXcQ?t=12"), "youtube:dQw4w9WgXcQ");
});

test("TikTok and YouTube result routes normalize to stable post identities", () => {
  const tiktok = "https://www.tiktok.com/@creator/video/7512345678901234567?is_from_webapp=1";
  assert.equal(canonicalTikTokResultUrl(tiktok),
    "https://www.tiktok.com/@creator/video/7512345678901234567");
  assert.deepEqual(canonicalCandidateUrls("tiktok", [tiktok, `${tiktok}&duplicate=1`]), [
    "https://www.tiktok.com/@creator/video/7512345678901234567",
  ]);
  assert.equal(canonicalTikTokResultUrl("https://outside.example/@creator/video/7512345678901234567"), null);

  for (const input of [
    "https://www.youtube.com/watch?v=dQw4w9WgXcQ&feature=share",
    "https://www.youtube.com/shorts/dQw4w9WgXcQ?si=fixture",
    "https://youtu.be/dQw4w9WgXcQ?t=12",
  ]) {
    assert.equal(canonicalYouTubeResultUrl(input),
      "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
  }
  assert.deepEqual(canonicalCandidateUrls("youtube", [
    "https://www.youtube.com/shorts/dQw4w9WgXcQ",
    "https://youtu.be/dQw4w9WgXcQ",
  ]), ["https://www.youtube.com/watch?v=dQw4w9WgXcQ"]);
  assert.equal(searchResultLinkIndex("youtube", [
    "https://www.youtube.com/@channel",
    "https://www.youtube.com/shorts/dQw4w9WgXcQ",
  ], "https://youtu.be/dQw4w9WgXcQ"), 1);
});

test("Weibo result URLs retain exact post identities across desktop and mobile routes", () => {
  const desktop = "http://www.weibo.com/2286908003/RlQuA7sqn?refer_flag=1001030103_";
  const mobile = "https://m.weibo.cn/detail/5352066721777563?jumpfrom=weibocom";
  assert.equal(canonicalWeiboResultUrl(desktop), "https://weibo.com/2286908003/RlQuA7sqn");
  assert.equal(canonicalWeiboResultUrl(mobile), "https://m.weibo.cn/status/5352066721777563");
  assert.deepEqual(canonicalCandidateUrls("weibo", [desktop, mobile]), [
    "https://weibo.com/2286908003/RlQuA7sqn",
  ]);
  assert.equal(canonicalWeiboResultUrl("https://outside.example/2286908003/RlQuA7sqn"), null);
  assert.equal(searchResultLinkIndex("weibo", [
    "https://weibo.com/u/2286908003",
    desktop,
  ], mobile), 1);
});

test("Toutiao search jump links resolve only exact first-party content identities", () => {
  const article = "7694611241247015479";
  const video = "7540277264643325991";
  const microPost = "1878489458132992";
  const articleJump = `https://so.toutiao.com/search/jump?url=${encodeURIComponent(
    `https://article.zlink.toutiao.com/J4?alert=0&h5_url=${encodeURIComponent(
      `https://toutiao.com/group/${article}/?source=news`,
    )}`,
  )}`;
  const videoJump = `https://so.toutiao.com/search/jump?url=${encodeURIComponent(
    `https://m.toutiaoimg.cn/group/${video}/?source=video`,
  )}`;
  const microPostJump = `https://so.toutiao.com/search/jump?url=${encodeURIComponent(
    `https://weitoutiao.zjurl.cn/ugc/share/wap/thread/${microPost}/?source=weitoutiao`,
  )}`;
  assert.equal(canonicalToutiaoResultUrl(articleJump),
    `https://www.toutiao.com/article/${article}/`);
  assert.equal(canonicalToutiaoResultUrl(videoJump),
    `https://www.toutiao.com/video/${video}/`);
  assert.equal(canonicalToutiaoResultUrl(microPostJump),
    `https://www.toutiao.com/w/${microPost}/`);
  assert.deepEqual(canonicalCandidateUrls("toutiao", [articleJump, videoJump, microPostJump]), [
    `https://www.toutiao.com/article/${article}/`,
    `https://www.toutiao.com/video/${video}/`,
    `https://www.toutiao.com/w/${microPost}/`,
  ]);
  assert.equal(canonicalToutiaoResultUrl(
    "https://outside.example/search/jump?url=https%3A%2F%2Fwww.toutiao.com%2Farticle%2F7694611241247015479%2F",
  ), null);
  assert.equal(searchResultLinkIndex("toutiao", [
    "https://so.toutiao.com/search/jump?url=https%3A%2F%2Foutside.example%2Fstory",
    videoJump,
    articleJump,
  ], `https://www.toutiao.com/article/${article}/`), 2);
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
  assert.deepEqual(sourceUrls("toutiao", { source_mode: "home_feed" }), [
    "https://www.toutiao.com/",
  ]);
  assert.deepEqual(sourceUrls("toutiao", { source_mode: "topics", topics: ["AI agent"] }), [
    "https://so.toutiao.com/search?keyword=AI%20agent&pd=information&source=search_subtab_switch&from=information&aid=1455",
  ]);
  assert.deepEqual(sourceUrls("toutiao", { source_mode: "seed_urls", seed_urls: [
    "https://www.toutiao.com/group/7694557030652084736/?source=news",
    "https://m.toutiaoimg.cn/group/7689461431258858010/?source=video",
    "https://weitoutiao.zjurl.cn/ugc/share/wap/thread/1878457723490307/",
  ] }), [
    "https://www.toutiao.com/article/7694557030652084736/",
    "https://www.toutiao.com/video/7689461431258858010/",
    "https://www.toutiao.com/w/1878457723490307/",
  ]);
  assert.deepEqual(sourceUrls("weibo", { source_mode: "home_feed" }), [
    "https://weibo.com/hot/weibo/102803",
  ]);
  assert.deepEqual(sourceUrls("weibo", { source_mode: "topics", topics: ["AI agent"] }), [
    "https://s.weibo.com/weibo?q=AI%20agent",
  ]);
  assert.deepEqual(sourceUrls("weibo", { source_mode: "seed_urls", seed_urls: [
    "http://weibo.com/2286908003/RlQuA7sqn?refer_flag=1001030103_",
  ] }), ["https://weibo.com/2286908003/RlQuA7sqn"]);
  assert.deepEqual(sourceUrls("tiktok", { source_mode: "topics", topics: ["AI agent"] }), [
    "https://www.tiktok.com/search?q=AI%20agent",
  ]);
  assert.deepEqual(sourceUrls("tiktok", { source_mode: "seed_urls", seed_urls: [
    "https://www.tiktok.com/@creator/video/7512345678901234567?is_from_webapp=1",
  ] }), ["https://www.tiktok.com/@creator/video/7512345678901234567"]);
  assert.deepEqual(sourceUrls("youtube", { source_mode: "home_feed" }), [
    "https://www.youtube.com/",
  ]);
  assert.deepEqual(sourceUrls("youtube", { source_mode: "topics", topics: ["AI agent"] }), [
    "https://www.youtube.com/results?search_query=AI%20agent",
  ]);
  assert.deepEqual(sourceUrls("youtube", { source_mode: "seed_urls", seed_urls: [
    "https://youtu.be/dQw4w9WgXcQ?t=12",
  ] }), ["https://www.youtube.com/watch?v=dQw4w9WgXcQ"]);
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
  assert.equal(
    platformAccessError("toutiao", "https://so.toutiao.com/search?keyword=AI", [
      "https://rmc.bytedance.com/verifycenter/captcha/v2?from=iframe",
    ]),
    "challenge_required",
  );
  assert.equal(platformAccessError("kuaishou", "https://www.kuaishou.com/brilliant"), null);
  assert.equal(platformAccessError("weibo", "https://passport.weibo.com/sso/signin"), "login_required");
  assert.equal(platformAccessError("weibo", "https://weibo.com/2286908003/RlQuA7sqn"), null);
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
