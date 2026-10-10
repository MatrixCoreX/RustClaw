import { randomUUID } from "node:crypto";

const PLATFORM_SPECS = Object.freeze({
  douyin: {
    defaultBrowserMode: "silent",
    homeUrl: "https://www.douyin.com/",
    hosts: ["douyin.com"],
    detailPath: /^\/(?:video|note)\/[A-Za-z0-9_-]+(?:\/|$)/u,
    topicUrl: (topic) => `https://www.douyin.com/search/${encodeURIComponent(topic)}`,
  },
  xiaohongshu: {
    defaultBrowserMode: "silent",
    homeUrl: "https://www.xiaohongshu.com/explore",
    hosts: ["xiaohongshu.com"],
    detailPath: /^\/(?:explore|search_result|discovery\/item)\/[A-Za-z0-9_-]+(?:\/|$)/u,
    topicUrl: (topic) =>
      `https://www.xiaohongshu.com/search_result?keyword=${encodeURIComponent(topic)}`,
  },
  kuaishou: {
    defaultBrowserMode: "silent",
    homeUrl: "https://www.kuaishou.com/brilliant",
    hosts: ["kuaishou.com"],
    detailPath: /^\/short-video\/[A-Za-z0-9_-]{8,}(?:\/|$)/u,
    topicUrl: (topic) =>
      `https://www.kuaishou.com/search/${encodeURIComponent(topic)}`,
  },
  toutiao: {
    defaultBrowserMode: "silent",
    homeUrl: "https://www.toutiao.com/",
    hosts: ["toutiao.com", "toutiaoimg.cn", "weitoutiao.zjurl.cn"],
    detailPath: /^\/(?:article|video|w)\/\d+(?:\/|$)/u,
    topicNavigation: "direct",
    topicUrl: (topic) =>
      `https://so.toutiao.com/search?keyword=${encodeURIComponent(topic)}&pd=information&source=search_subtab_switch&from=information&aid=1455`,
  },
  weibo: {
    defaultBrowserMode: "silent",
    homeUrl: "https://weibo.com/hot/weibo/102803",
    hosts: ["weibo.com", "weibo.cn"],
    detailPath: /^\/(?:\d+\/[A-Za-z0-9]+|(?:status|detail)\/[A-Za-z0-9]+|tv\/show\/\d+:\d+)(?:\/|$)/u,
    topicNavigation: "direct",
    topicUrl: (topic) => `https://s.weibo.com/weibo?q=${encodeURIComponent(topic)}`,
  },
  tiktok: {
    defaultBrowserMode: "silent",
    homeUrl: "https://www.tiktok.com/",
    hosts: ["tiktok.com"],
    detailPath: /^\/@[^/]+\/(?:video|photo)\/\d+(?:\/|$)/u,
    topicNavigation: "direct",
    topicUrl: (topic) => `https://www.tiktok.com/search?q=${encodeURIComponent(topic)}`,
  },
  youtube: {
    defaultBrowserMode: "silent",
    homeUrl: "https://www.youtube.com/",
    hosts: ["youtube.com", "youtu.be"],
    detailPath: /^\/(?:watch\/?$|(?:shorts|live)\/[A-Za-z0-9_-]{11}(?:\/|$))/u,
    topicNavigation: "direct",
    topicUrl: (topic) => `https://www.youtube.com/results?search_query=${encodeURIComponent(topic)}`,
  },
});

export const SUPPORTED_PLATFORMS = Object.freeze(Object.keys(PLATFORM_SPECS));

export function resolveBrowserMode(platform, mode) {
  const resolved = mode ?? (platform ? platformSpec(platform).defaultBrowserMode : "silent");
  if (!["visible", "silent"].includes(resolved)) throw new Error("browser_mode_invalid");
  return resolved;
}

export function platformSpec(platform) {
  const spec = PLATFORM_SPECS[platform];
  if (!spec) throw new Error("platform_unsupported");
  return spec;
}

function hostAllowed(host, allowed) {
  return allowed.some((domain) => host === domain || host.endsWith(`.${domain}`));
}

function decodeUrlLayer(value) {
  const unescaped = String(value || "")
    .replaceAll("&amp;", "&")
    .replaceAll("&#38;", "&")
    .replaceAll("&#x26;", "&")
    .replaceAll("&quot;", '"');
  try { return decodeURIComponent(unescaped); } catch { return unescaped; }
}

function toutiaoContentKind(value) {
  const normalized = String(value || "").toLowerCase();
  if (/(?:^|[^a-z])(?:weitoutiao|thread)(?:[^a-z]|$)/u.test(normalized)) return "w";
  if (/(?:^|[^a-z])video(?:[^a-z]|$)/u.test(normalized)) return "video";
  return "article";
}

function canonicalToutiaoPath(kind, itemId) {
  return `https://www.toutiao.com/${kind}/${itemId}/`;
}

function directToutiaoContentUrl(value, context = "") {
  let parsed;
  try { parsed = new URL(value, "https://so.toutiao.com/"); } catch { return null; }
  const host = parsed.hostname.toLowerCase();
  let match;
  if (hostAllowed(host, ["toutiao.com"])
    && (match = parsed.pathname.match(/^\/(article|video|w)\/(\d{10,})(?:\/|$)/u))) {
    return canonicalToutiaoPath(match[1], match[2]);
  }
  if (hostAllowed(host, ["toutiao.com", "toutiaoimg.cn"])
    && (match = parsed.pathname.match(/^\/group\/(\d{10,})(?:\/|$)/u))) {
    return canonicalToutiaoPath(toutiaoContentKind(`${context} ${parsed.search}`), match[1]);
  }
  if (host === "weitoutiao.zjurl.cn"
    && (match = parsed.pathname.match(/\/thread\/(\d{10,})(?:\/|$)/u))) {
    return canonicalToutiaoPath("w", match[1]);
  }
  return null;
}

export function canonicalToutiaoResultUrl(rawUrl) {
  const initial = String(rawUrl || "");
  const queue = [{ value: initial, context: initial, trusted: false }];
  const visited = new Set();
  for (let cursor = 0; cursor < queue.length && cursor < 20; cursor += 1) {
    const entry = queue[cursor];
    const value = decodeUrlLayer(entry.value);
    if (!value || visited.has(value)) continue;
    visited.add(value);
    let parsed;
    try { parsed = new URL(value, "https://so.toutiao.com/"); } catch { continue; }
    const host = parsed.hostname.toLowerCase();
    const isSearchJump = ["so.toutiao.com", "sou.toutiao.com"].includes(host)
      && parsed.pathname === "/search/jump";
    const isTrustedTransit = host === "article.zlink.toutiao.com";
    const trusted = entry.trusted || isSearchJump || isTrustedTransit;
    const direct = directToutiaoContentUrl(value, entry.context);
    if (direct && (trusted || hostAllowed(host, ["toutiao.com", "toutiaoimg.cn"])
      || host === "weitoutiao.zjurl.cn")) {
      return direct;
    }
    if (!trusted) continue;

    for (const key of ["h5_url", "url", "target_url", "target"]) {
      for (const nested of parsed.searchParams.getAll(key)) {
        if (nested) queue.push({ value: nested, context: `${entry.context} ${value}`, trusted: true });
      }
    }
    // Some result pages leave nested ampersands HTML-escaped or only partly
    // percent-encoded. Scan the trusted wrapper for known first-party routes;
    // never accept an arbitrary external URL or an unscoped numeric token.
    const expanded = decodeUrlLayer(value);
    const thread = expanded.match(/weitoutiao\.zjurl\.cn\/ugc\/share\/wap\/thread\/(\d{10,})/u);
    if (thread) return canonicalToutiaoPath("w", thread[1]);
    const group = expanded.match(/(?:^|\/)\/?(?:www\.)?toutiao\.com\/group\/(\d{10,})/u)
      || expanded.match(/m\.toutiaoimg\.cn\/group\/(\d{10,})/u);
    if (group) return canonicalToutiaoPath(toutiaoContentKind(expanded), group[1]);
  }
  return null;
}

export function canonicalWeiboResultUrl(rawUrl) {
  let parsed;
  try { parsed = new URL(String(rawUrl || ""), "https://weibo.com/"); } catch { return null; }
  const host = parsed.hostname.toLowerCase();
  if (!hostAllowed(host, ["weibo.com", "weibo.cn"])) return null;
  let match = parsed.pathname.match(/^\/(\d+)\/([A-Za-z0-9]+)(?:\/|$)/u);
  if (match) return `https://weibo.com/${match[1]}/${match[2]}`;
  match = parsed.pathname.match(/^\/(?:status|detail)\/([A-Za-z0-9]+)(?:\/|$)/u);
  if (match && (host === "m.weibo.cn" || host === "weibo.cn")) {
    return `https://m.weibo.cn/status/${match[1]}`;
  }
  match = parsed.pathname.match(/^\/tv\/show\/(\d+:\d+)(?:\/|$)/u);
  if (match && hostAllowed(host, ["weibo.com"])) return `https://weibo.com/tv/show/${match[1]}`;
  return null;
}

export function canonicalTikTokResultUrl(rawUrl) {
  let parsed;
  try { parsed = new URL(String(rawUrl || ""), "https://www.tiktok.com/"); } catch { return null; }
  if (!hostAllowed(parsed.hostname.toLowerCase(), ["tiktok.com"])) return null;
  const match = parsed.pathname.match(/^(\/@[^/]+\/(?:video|photo)\/(\d+))(?:\/|$)/u);
  return match ? `https://www.tiktok.com${match[1]}` : null;
}

function youtubeVideoId(rawUrl) {
  let parsed;
  try { parsed = new URL(String(rawUrl || ""), "https://www.youtube.com/"); } catch { return null; }
  const host = parsed.hostname.toLowerCase();
  if (!hostAllowed(host, ["youtube.com", "youtu.be"])) return null;
  let value = hostAllowed(host, ["youtu.be"])
    ? parsed.pathname.split("/").filter(Boolean)[0]
    : parsed.pathname === "/watch"
      ? parsed.searchParams.get("v")
      : parsed.pathname.match(/^\/(?:shorts|live)\/([A-Za-z0-9_-]{11})(?:\/|$)/u)?.[1];
  value = String(value || "");
  return /^[A-Za-z0-9_-]{11}$/u.test(value) ? value : null;
}

export function canonicalYouTubeResultUrl(rawUrl) {
  const videoId = youtubeVideoId(rawUrl);
  return videoId ? `https://www.youtube.com/watch?v=${videoId}` : null;
}

export function canonicalPlatformResultUrl(platform, rawUrl) {
  if (platform === "toutiao") return canonicalToutiaoResultUrl(rawUrl);
  if (platform === "weibo") return canonicalWeiboResultUrl(rawUrl);
  if (platform === "tiktok") return canonicalTikTokResultUrl(rawUrl);
  if (platform === "youtube") return canonicalYouTubeResultUrl(rawUrl);
  return rawUrl;
}

export function validatePlatformUrl(platform, rawUrl) {
  const spec = platformSpec(platform);
  let parsed;
  try {
    parsed = new URL(rawUrl);
  } catch {
    throw new Error("source_url_invalid");
  }
  if (parsed.protocol !== "https:" || parsed.username || parsed.password) {
    throw new Error("source_url_invalid");
  }
  if (!hostAllowed(parsed.hostname.toLowerCase(), spec.hosts)) {
    throw new Error("source_host_not_allowed");
  }
  parsed.hash = "";
  return parsed.toString();
}

export function sourceTargets(platform, config) {
  const spec = platformSpec(platform);
  const mode = config.source_mode || "home_feed";
  if (mode === "home_feed") {
    return [{ source_mode: mode, search_keyword: null, url: spec.homeUrl }];
  }
  if (mode === "topics") {
    const topics = Array.isArray(config.topics)
      ? config.topics.map((topic) => String(topic).trim()).filter(Boolean)
      : [];
    if (topics.length === 0) throw new Error("source_scope_empty");
    return topics.map((topic) => ({
      source_mode: mode,
      search_keyword: topic,
      url: spec.topicUrl(topic),
    }));
  }
  if (mode === "seed_urls") {
    const seeds = Array.isArray(config.seed_urls) ? config.seed_urls : [];
    if (seeds.length === 0) throw new Error("source_scope_empty");
    return seeds.map((url) => {
      const canonical = canonicalPlatformResultUrl(platform, url);
      return {
        source_mode: mode,
        search_keyword: null,
        url: canonical || validatePlatformUrl(platform, url),
      };
    });
  }
  throw new Error("source_mode_invalid");
}

export function sourceUrls(platform, config) {
  return sourceTargets(platform, config).map((target) => target.url);
}

export function manualVerificationTarget(platform, config, blockedUrl) {
  return validatePlatformUrl(platform, blockedUrl || sourceTargets(platform, config)[0].url);
}

function douyinHomePath(pathname) {
  return pathname === "/" || pathname === "/jingxuan";
}

function decodePathKeyword(value) {
  try { return decodeURIComponent(value); } catch { return value; }
}

function douyinSearchKeyword(pathname) {
  const match = pathname.match(/^(?:\/jingxuan)?\/search\/([^/]+)\/?$/u);
  return match ? decodePathKeyword(match[1]) : null;
}

export function matchesVerificationTarget(platform, targetUrl, currentUrl) {
  if (!targetUrl) return true;
  try {
    const target = new URL(validatePlatformUrl(platform, targetUrl));
    const current = new URL(validatePlatformUrl(platform, currentUrl));
    if (target.origin !== current.origin) return false;
    if (platform === "xiaohongshu"
      && ["/search_result", "/search_result_ai"].includes(target.pathname)
      && ["/search_result", "/search_result_ai"].includes(current.pathname)) {
      const keyword = urlSearchKeyword(target);
      return keyword !== null && keyword === urlSearchKeyword(current);
    }
    if (platform === "douyin") {
      if (douyinHomePath(target.pathname) && douyinHomePath(current.pathname)) return true;
      const targetKeyword = douyinSearchKeyword(target.pathname);
      const currentKeyword = douyinSearchKeyword(current.pathname);
      if (targetKeyword !== null && targetKeyword === currentKeyword) return true;
    }
    return target.pathname === current.pathname
      && [...target.searchParams].every(([name, value]) => current.searchParams.get(name) === value);
  } catch {
    return false;
  }
}

export function matchesManualAccessTarget(platform, targetUrl, currentUrl) {
  if (matchesVerificationTarget(platform, targetUrl, currentUrl)) return true;
  if (!targetUrl) return true;
  try {
    const target = new URL(validatePlatformUrl(platform, targetUrl));
    const current = new URL(validatePlatformUrl(platform, currentUrl));
    return platform === "douyin" && target.origin === current.origin && douyinHomePath(current.pathname)
      && (douyinHomePath(target.pathname) || douyinSearchKeyword(target.pathname) !== null);
  } catch {
    return false;
  }
}

function urlSearchKeyword(url) {
  const value = url.searchParams.get("keyword");
  if (value === null) return null;
  if (url.pathname === "/search_result_ai") {
    try { return decodeURIComponent(value); } catch { return value; }
  }
  return value;
}

export function douyinModalItemId(rawUrl) {
  try {
    const value = new URL(validatePlatformUrl("douyin", rawUrl)).searchParams.get("modal_id");
    return /^\d+$/u.test(value || "") ? value : null;
  } catch {
    return null;
  }
}

export function isDetailUrl(platform, rawUrl) {
  try {
    const normalized = validatePlatformUrl(platform, rawUrl);
    if (platform === "douyin" && douyinModalItemId(normalized)) return true;
    if (platform === "youtube") return canonicalYouTubeResultUrl(normalized) !== null;
    return platformSpec(platform).detailPath.test(new URL(normalized).pathname);
  } catch {
    return false;
  }
}

export function douyinSearchGridItemUrls(itemIds) {
  return canonicalCandidateUrls("douyin", (itemIds || [])
    .map((itemId) => String(itemId || ""))
    .filter((itemId) => /^\d+$/u.test(itemId))
    .map((itemId) => `https://www.douyin.com/video/${itemId}`));
}

export function canonicalCandidateUrls(platform, rawUrls) {
  const seen = new Set();
  const result = [];
  for (const rawUrl of rawUrls) {
    try {
      const candidate = canonicalPlatformResultUrl(platform, rawUrl);
      if (!candidate) continue;
      const normalized = validatePlatformUrl(platform, candidate);
      if (!isDetailUrl(platform, normalized)) continue;
      const identity = platformItemId(platform, normalized);
      if (seen.has(identity)) continue;
      seen.add(identity);
      result.push(normalized);
    } catch {
      // Invalid or off-platform links are ignored as untrusted page input.
    }
  }
  return result;
}

export function platformItemId(platform, rawUrl) {
  const normalized = validatePlatformUrl(platform, rawUrl);
  if (platform === "douyin") {
    const modal = douyinModalItemId(normalized);
    if (modal) return `douyin:${modal}`;
  }
  if (platform === "youtube") {
    const videoId = youtubeVideoId(normalized);
    if (!videoId) throw new Error("source_url_invalid");
    return `youtube:${videoId}`;
  }
  const segments = new URL(normalized).pathname.split("/").filter(Boolean);
  if (platform === "weibo") {
    const value = segments.at(-1) || "";
    if (/^\d+$/u.test(value)) return `weibo:${value}`;
    if (segments.at(-2) !== "show" && /^[A-Za-z0-9]+$/u.test(value)) {
      const alphabet = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
      const decoded = [];
      for (let end = value.length; end > 0; end -= 4) {
        const chunk = value.slice(Math.max(0, end - 4), end);
        let number = 0n;
        for (const character of chunk) {
          const digit = alphabet.indexOf(character);
          if (digit < 0) throw new Error("source_url_invalid");
          number = number * 62n + BigInt(digit);
        }
        decoded.unshift(end > 4 ? number.toString().padStart(7, "0") : number.toString());
      }
      return `weibo:${decoded.join("")}`;
    }
  }
  return `${platform}:${segments.at(-1) || randomUUID()}`;
}
