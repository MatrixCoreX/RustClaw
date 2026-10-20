import { platformItemId } from "./platforms.mjs";
import { evaluateEngagementFilters, exactMetric, metricFromDisplay } from "./engagement.mjs";
import { metricAvailability } from "./engagement_availability.mjs";
import { browserOperationTimeoutMs } from "./browser_environment.mjs";

const ENGAGEMENT_SELECTORS = Object.freeze({
  douyin: Object.freeze({
    views: Object.freeze([
      '[data-e2e="video-views"]',
      '[data-e2e="video-play-count"]',
      '[data-e2e="play-count"]',
    ]),
    likes: Object.freeze([
      '[data-e2e="video-player-digg"]',
      '[data-e2e="video-like-count"]',
      '[data-e2e="like-count"]',
      '[data-e2e="digg-count"]',
    ]),
    comments: Object.freeze([
      '[data-e2e="feed-comment-icon"]',
      '[data-e2e="video-comment-count"]',
      '[data-e2e="comment-count"]',
    ]),
    favorites: Object.freeze([
      '[data-e2e="video-player-collect"]',
      '[data-e2e="video-collect-count"]',
      '[data-e2e="collect-count"]',
    ]),
    shares: Object.freeze([
      '[data-e2e="video-player-share"]',
      '[data-e2e="video-share-icon-container"]',
      '[data-e2e="video-share-count"]',
      '[data-e2e="share-count"]',
    ]),
  }),
  xiaohongshu: Object.freeze({
    likes: Object.freeze([
      '[data-testid="like-count"]',
      '.like-wrapper .count',
    ]),
    comments: Object.freeze([
      '[data-testid="comment-count"]',
      '.comment-wrapper .count',
    ]),
    favorites: Object.freeze([
      '[data-testid="collect-count"]',
      '.collect-wrapper .count',
    ]),
    shares: Object.freeze([
      '[data-testid="share-count"]',
      '.share-wrapper .count',
    ]),
  }),
  kuaishou: Object.freeze({
    likes: Object.freeze([
      '.interactive-item.like-item .item-count',
      '.video-info-content:has(.like-icon) .info-text',
      '.photo-btns .like-btn',
    ]),
    comments: Object.freeze(['.interactive-item.comment-item .item-count', '[data-testid="comment-count"]', '.photo-btns .commentPanel']),
    favorites: Object.freeze(['.interactive-item.collect-item .item-count', '[data-testid="collect-count"]', '.photo-btns .favorite']),
    shares: Object.freeze(['.interactive-item.share-item .item-count', '[data-testid="share-count"]']),
    views: Object.freeze(['.video-info-content:has(.play-icon) .info-text', '[data-testid="play-count"]']),
  }),
  toutiao: Object.freeze({
    likes: Object.freeze([".detail-side-interaction .detail-like > span"]),
    comments: Object.freeze([".detail-side-interaction .detail-interaction-comment > span"]),
  }),
  weibo: Object.freeze({
    views: Object.freeze(['[class*="_plusInfo_"]']),
    likes: Object.freeze(["footer .woo-like-count"]),
    comments: Object.freeze(['footer [class*="_item_"]:has(.woo-font--comment) [class*="_num_"]']),
    reposts: Object.freeze(['footer [class*="_item_"]:has(.woo-font--retweet) [class*="_num_"]']),
  }),
  tiktok: Object.freeze({
    views: Object.freeze(['[data-e2e="video-views"]', '[data-e2e="video-view-count"]']),
    likes: Object.freeze(['[data-e2e="like-count"]', '[data-e2e="browse-like-count"]']),
    comments: Object.freeze(['[data-e2e="comment-count"]', '[data-e2e="browse-comment-count"]']),
    favorites: Object.freeze(['[data-e2e="undefined-count"]', '[data-e2e="browse-bookmark-count"]']),
    shares: Object.freeze(['[data-e2e="share-count"]', '[data-e2e="browse-share-count"]']),
  }),
  youtube: Object.freeze({
    views: Object.freeze(['#info span.view-count', '#info-container #info', 'yt-formatted-string#info']),
    likes: Object.freeze(['#segmented-like-button button', 'like-button-view-model button']),
    comments: Object.freeze(['ytd-comments-header-renderer #count']),
  }),
  bilibili: Object.freeze({
    views: Object.freeze(['.view.item', '.view-text', '.video-data .view']),
    likes: Object.freeze(['.video-like-info', '.like .info-text']),
    comments: Object.freeze(['.reply-header .nav-title-text', '.comment-container .reply-header']),
    favorites: Object.freeze(['.video-fav-info', '.collect .info-text']),
    shares: Object.freeze(['.video-share-info', '.share .info-text']),
    coins: Object.freeze(['.video-coin-info', '.coin .info-text']),
    danmaku: Object.freeze(['.dm.item', '.dm-text']),
  }),
});

const pageCounters = new WeakMap();
const pendingCounters = new WeakMap();

const STRUCTURED_FIELDS = Object.freeze({
  toutiao: { likes: "diggCount", comments: "commentCount", favorites: "repinCount", shares: "shareCount", reposts: "forwardCount", reads: "readCount", views: "playCount" },
  weibo: { likes: "attitudes_count", comments: "comments_count", reposts: "reposts_count", reads: "reads_count", views: "online_users_number" },
  bilibili: { views: "view", likes: "like", comments: "reply", favorites: "favorite", shares: "share", coins: "coin", danmaku: "danmaku" },
  xiaohongshu: { likes: "likedCount", comments: "commentCount", favorites: "collectedCount", shares: "shareCount" },
  tiktok: { views: "playCount", likes: "diggCount", comments: "commentCount", favorites: "collectCount", shares: "shareCount" },
});

export function supportedEngagementMetrics(platform) {
  return [...new Set([...Object.keys(ENGAGEMENT_SELECTORS[platform] || {}), ...Object.keys(STRUCTURED_FIELDS[platform] || {})])];
}

function weiboPostMatches(post, expectedId) {
  return expectedId != null && String(expectedId) !== ""
    && String(post?.idstr || post?.mid || post?.id || "") === String(expectedId);
}

export function weiboPostMetrics(post, expectedId) {
  if (!weiboPostMatches(post, expectedId)) return {};
  const { views: _views, ...fields } = STRUCTURED_FIELDS.weibo;
  const metrics = Object.fromEntries(Object.entries(fields).flatMap(([name, field]) => {
    const metric = exactMetric(post[field], `page_response:${field}`);
    return metric ? [[name, metric]] : [];
  }));
  const media = post.page_info?.media_info;
  if (media?.duration != null) {
    // The site's video UI renders online_users as "…次观看" and supplies
    // online_users_number as its exact total; it is separate from reads_count.
    const views = exactMetric(media.online_users_number, "page_response:media_info.online_users_number",
      metricFromDisplay(media.online_users));
    if (views) metrics.views = views;
  }
  return metrics;
}

// Observe only the exact post response the browser already requested. Persist
// whitelisted counters, never response bodies, credentials, or media tokens.
export function observeEngagementResponses(context, platform) {
  if (platform !== "weibo") return () => {};
  const onResponse = response => {
    let url, page;
    try {
      url = new URL(response.url());
      if (url.hostname !== "weibo.com" || url.pathname !== "/ajax/statuses/show") return;
      page = response.frame().page();
    } catch { return; }
    const pending = pendingCounters.get(page) || new Set();
    pendingCounters.set(page, pending);
    const operation = response.json().then(post => {
      const requested = url.searchParams.get("id");
      if (!requested) return;
      const id = platformItemId("weibo", `https://weibo.com/status/${requested}`);
      const expectedId = id.split(":").at(-1);
      // Reject the whole response before deriving missing-value evidence or
      // replacing a previous valid observation for the current post.
      if (id !== platformItemId("weibo", page.url()) || !weiboPostMatches(post, expectedId)) return;
      const notProvided = Object.fromEntries(Object.entries(STRUCTURED_FIELDS.weibo).filter(([name, field]) => name !== "views"
        && Object.hasOwn(post, field) && post[field] == null).map(([name, field]) => [name, `page_response:${field}`]));
      const media = post.page_info?.media_info;
      if (media?.duration != null && Object.hasOwn(media, "online_users_number") && media.online_users_number == null) {
        notProvided.views = "page_response:media_info.online_users_number";
      }
      pageCounters.set(page, { id, metrics: weiboPostMetrics(post, expectedId), notProvided });
    }).catch(() => {});
    pending.add(operation);
    operation.finally(() => pending.delete(operation));
  };
  context.on("response", onResponse);
  return () => context.off("response", onResponse);
}

async function structuredMetrics(scope, platform, id) {
  const page = typeof scope.page === "function" ? scope.page() : scope;
  return page.evaluate(({ platform, id, fields }) => {
    const view = globalThis;
    const result = {};
    const assign = (data, fields, source) => {
      for (const [name, field] of Object.entries(fields)) {
        const value = data?.[field];
        if (typeof value === "number" || typeof value === "string"
          || data && Object.hasOwn(data, field) && value == null) result[name] = { value, source: `${source}:${field}` };
      }
    };
    if (platform === "toutiao") {
      let data;
      try { data = JSON.parse(decodeURIComponent(view.document.querySelector("script#RENDER_DATA")?.textContent || "")).data; }
      catch { return result; }
      const video = data?.initialVideo;
      const expected = id || view.location.pathname.split("/").filter(Boolean).at(-1);
      if (!expected || String(video?.itemId || data?.itemId || "") !== expected) return result;
      const counters = Object.fromEntries(Object.entries(fields).filter(([name]) => name !== "views" && (!video || name !== "reads")));
      assign(video?.itemCell?.itemCounter || data.itemCell?.itemCounter, counters, "post_state:itemCounter");
      if (video) assign(video, { views: "playCount", likes: "diggCount", comments: "commentCount", favorites: "repinCount" }, "post_state:initialVideo");
    } else if (platform === "bilibili") {
      const video = view.__INITIAL_STATE__?.videoData;
      const matchesBvid = typeof video?.bvid === "string" && video.bvid !== "" && id === video.bvid;
      const matchesAid = /^av\d+$/iu.test(id || "") && id.slice(2) === String(video?.aid);
      if (!matchesBvid && !matchesAid) return result;
      assign(video?.stat, fields, "post_state:videoData.stat");
    } else if (platform === "xiaohongshu" && id) {
      const note = view.__INITIAL_STATE__?.note?.noteDetailMap?.[id]?.note;
      if (String(note?.noteId || "") !== id) return result;
      assign(note.interactInfo, fields, "post_state:interactInfo");
    } else if (platform === "tiktok" && id) {
      let state = view.__UNIVERSAL_DATA_FOR_REHYDRATION__;
      if (!state) {
        try { state = JSON.parse(view.document.querySelector("script#__UNIVERSAL_DATA_FOR_REHYDRATION__")?.textContent || "null"); } catch {}
      }
      let video = state?.__DEFAULT_SCOPE__?.["webapp.video-detail"]?.itemInfo?.itemStruct || view.SIGI_STATE?.ItemModule?.[id];
      if (!video) {
        try { video = JSON.parse(view.document.querySelector("script#SIGI_STATE")?.textContent || "null")?.ItemModule?.[id]; } catch {}
      }
      if (String(video?.id || "") !== id) return result;
      assign(video.stats, fields, "post_state:stats");
    }
    return result;
  }, { platform, id, fields: STRUCTURED_FIELDS[platform] || {} }).catch(() => ({}));
}

async function captureSnapshot(scope, platform, capturedAt, itemId = null) {
  const page = typeof scope.page === "function" ? scope.page() : scope;
  if (!itemId) {
    try { itemId = platformItemId(platform, page.url()); } catch {}
  }
  const id = itemId?.split(":").at(-1) || null;
  const metrics = {};
  for (const [name, selectors] of Object.entries(ENGAGEMENT_SELECTORS[platform] || {})) {
    for (const selector of selectors) {
      const candidates = await scope.locator(selector).evaluateAll((nodes, platformName) => nodes.filter(node => {
        const rect = node.getBoundingClientRect();
        const style = getComputedStyle(node);
        return rect.width > 0 && rect.height > 0 && style.display !== "none" && style.visibility !== "hidden"
          && !node.closest('[data-comment-id], .comments-container')
          && !(platformName === "douyin" && node.closest(".search-result-card"));
      }).map(node => ({
        machine: node.getAttribute("data-count") || node.getAttribute("data-value"),
        rendered: node instanceof HTMLElement ? node.innerText : node.textContent || "",
      })), platform);
      for (const candidate of candidates) {
        const metric = metricFromDisplay(candidate.machine) || metricFromDisplay(candidate.rendered);
        if (metric) { metrics[name] = metric; break; }
      }
      if (metrics[name]) break;
    }
  }
  const counters = await structuredMetrics(scope, platform, id);
  const notProvided = {};
  for (const [name, counter] of Object.entries(counters)) {
    if (counter.value == null || String(counter.value).trim() === "") notProvided[name] = counter.source;
    const metric = exactMetric(counter.value, platform === "bilibili" ? undefined : counter.source, metrics[name])
      || metricFromDisplay(counter.value, counter.source);
    if (metric) metrics[name] = metric;
  }
  await Promise.allSettled([...(pendingCounters.get(page) || [])]);
  const response = pageCounters.get(page);
  if (response?.id === itemId) Object.assign(metrics, response.metrics);
  let contentType = "unknown";
  if (platform === "toutiao") {
    const pathname = new URL(page.url()).pathname;
    contentType = pathname.startsWith("/article/") ? "article" : pathname.startsWith("/video/") ? "video" : "unknown";
  }
  return { schema_version: 1, platform, captured_at: capturedAt, metrics,
    availability: metricAvailability(platform, metrics, supportedEngagementMetrics(platform), contentType,
      { ...notProvided, ...(response?.id === itemId ? response.notProvided : {}) }) };
}

export async function captureEngagementMetrics(scope, platform, capturedAt, itemId = null, options = {}) {
  const snapshot = await captureSnapshot(scope, platform, capturedAt, itemId);
  const filters = options.config?.engagement_filters;
  if (!filters) return snapshot;
  const decision = evaluateEngagementFilters(snapshot, filters, platform);
  const missing = decision.observations.filter(rule => rule.reason === "metric_unavailable"
    && snapshot.availability[rule.metric]?.retryable);
  if (decision.matched || !missing.length) return snapshot;
  const page = typeof scope.page === "function" ? scope.page() : scope;
  const timeout = options.readinessTimeoutMs ?? browserOperationTimeoutMs();
  try {
    const expected = itemId || platformItemId(platform, page.url());
    if (platform === "weibo" && pageCounters.get(page)?.id !== expected) {
      await page.waitForResponse(response => {
        const url = new URL(response.url());
        return url.hostname === "weibo.com" && url.pathname === "/ajax/statuses/show"
          && platformItemId("weibo", `https://weibo.com/status/${url.searchParams.get("id")}`) === expected;
      }, { timeout });
    } else {
      const selectors = [...new Set(missing.flatMap(rule => ENGAGEMENT_SELECTORS[platform]?.[rule.metric] || []))];
      if (!selectors.length) return snapshot;
      await scope.locator(selectors.join(",")).first().waitFor({ state: "visible", timeout });
    }
    const refreshed = await captureSnapshot(scope, platform, capturedAt, itemId);
    return { ...refreshed, observation: { rechecked: true, readiness: "observed" } };
  } catch (error) {
    return { ...snapshot, observation: { rechecked: true,
      readiness: error?.name === "TimeoutError" ? "not_observed_before_io_deadline" : "observation_unavailable" } };
  }
}

