import assert from "node:assert/strict";
import test from "node:test";

import { douyinCoverIdentity, douyinSearchCardCandidates } from "../src/browser_douyin_search.mjs";

test("Douyin search cards keep visible video tiles and ignore related-search placeholders", () => {
  assert.equal(douyinCoverIdentity("https://p3.douyinpic.com/cover/abc.webp?x=1", "fallback"),
    "https://p3.douyinpic.com/cover/abc.webp");
  assert.equal(douyinCoverIdentity("blob:https://www.douyin.com/abc", "card:0"), "card:0");
  const cards = douyinSearchCardCandidates([
    {
      index: 0, width: 80, height: 80, top: 10, bottom: 90, left: 10, viewportHeight: 800,
      hasVideoImage: true, coverUrl: "https://p3.douyinpic.com/cover/small.webp", hidden: false,
    },
    {
      index: 1, width: 240, height: 320, top: 20, bottom: 340, left: 40, viewportHeight: 800,
      hasVideoImage: false, coverUrl: "https://p3.douyinpic.com/cover/related.webp", hidden: false,
    },
    {
      index: 2, width: 240, height: 320, top: 20, bottom: 340, left: 300, viewportHeight: 800,
      hasVideoImage: true, coverUrl: "https://p3.douyinpic.com/cover/video.webp?token=1", hidden: false,
    },
    {
      index: 3, width: 240, height: 320, top: 900, bottom: 1220, left: 40, viewportHeight: 800,
      hasVideoImage: true, coverUrl: "https://p3.douyinpic.com/cover/offscreen.webp", hidden: false,
    },
  ]);
  assert.deepEqual(cards, [{ index: 2, coverKey: "https://p3.douyinpic.com/cover/video.webp" }]);
});
