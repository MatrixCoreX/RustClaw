import { randomUUID } from "node:crypto";

const PLATFORM_SPECS = Object.freeze({
  douyin: {
    homeUrl: "https://www.douyin.com/",
    hosts: ["douyin.com"],
    detailPath: /^\/(?:video|note)\/[A-Za-z0-9_-]+(?:\/|$)/u,
    topicUrl: (topic) => `https://www.douyin.com/search/${encodeURIComponent(topic)}`,
  },
  xiaohongshu: {
    homeUrl: "https://www.xiaohongshu.com/explore",
    hosts: ["xiaohongshu.com"],
    detailPath: /^\/(?:explore|discovery\/item)\/[A-Za-z0-9_-]+(?:\/|$)/u,
    topicUrl: (topic) =>
      `https://www.xiaohongshu.com/search_result?keyword=${encodeURIComponent(topic)}`,
  },
});

export const SUPPORTED_PLATFORMS = Object.freeze(Object.keys(PLATFORM_SPECS));

export function platformSpec(platform) {
  const spec = PLATFORM_SPECS[platform];
  if (!spec) throw new Error("platform_unsupported");
  return spec;
}

function hostAllowed(host, allowed) {
  return allowed.some((domain) => host === domain || host.endsWith(`.${domain}`));
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
    return seeds.map((url) => ({
      source_mode: mode,
      search_keyword: null,
      url: validatePlatformUrl(platform, url),
    }));
  }
  throw new Error("source_mode_invalid");
}

export function sourceUrls(platform, config) {
  return sourceTargets(platform, config).map((target) => target.url);
}

export function isDetailUrl(platform, rawUrl) {
  try {
    const normalized = validatePlatformUrl(platform, rawUrl);
    return platformSpec(platform).detailPath.test(new URL(normalized).pathname);
  } catch {
    return false;
  }
}

export function canonicalCandidateUrls(platform, rawUrls) {
  const seen = new Set();
  const result = [];
  for (const rawUrl of rawUrls) {
    try {
      const normalized = validatePlatformUrl(platform, rawUrl);
      if (!isDetailUrl(platform, normalized) || seen.has(normalized)) continue;
      seen.add(normalized);
      result.push(normalized);
    } catch {
      // Invalid or off-platform links are ignored as untrusted page input.
    }
  }
  return result;
}

export function platformItemId(platform, rawUrl) {
  const normalized = validatePlatformUrl(platform, rawUrl);
  const segments = new URL(normalized).pathname.split("/").filter(Boolean);
  return `${platform}:${segments.at(-1) || randomUUID()}`;
}
