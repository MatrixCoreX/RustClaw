import fs from "node:fs/promises";
import { constants as fsConstants } from "node:fs";
import path from "node:path";
import { browserStageError, recordBrowserFailure } from "./browser_diagnostics.mjs";

import {
  canonicalCandidateUrls,
  isDetailUrl,
  resolveBrowserMode,
  SUPPORTED_PLATFORMS,
  platformItemId,
  sourceTargets,
  validatePlatformUrl,
} from "./platforms.mjs";

const NAVIGATION_TIMEOUT_MS = 45_000;
const SCREENSHOT_MIN_BYTES = 512;
const INTERACTIVE_LOGIN_TIMEOUT_MS = 10 * 60 * 1000;
const INTERACTIVE_LOGIN_POLL_MS = 1000;
const INTERACTIVE_CHALLENGE_POLL_MS = 1000;

const PLATFORM_AUTH_COOKIE_NAMES = Object.freeze({
  douyin: new Set(["sessionid", "sessionid_ss", "sid_guard", "uid_tt", "uid_tt_ss"]),
  xiaohongshu: new Set(["web_session"]),
  kuaishou: new Set(["kuaishou.server.web_st", "userId"]),
});

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
    ]),
  }),
});

const PLATFORM_CAPTION_SELECTOR_GROUPS = Object.freeze({
  douyin: Object.freeze([
    Object.freeze([
      '[data-e2e="detail-video-info"] h1',
      '[data-e2e="feed-video-desc"]',
      '[data-e2e="video-desc"]',
      '[data-e2e="video-title"]',
      '[data-e2e="feed-video-title"]',
    ]),
  ]),
  xiaohongshu: Object.freeze([
    Object.freeze([
      '[data-testid="note-title"]',
      '#detail-title',
      '.note-content .title',
    ]),
    Object.freeze([
      '[data-testid="note-desc"]',
      '#detail-desc',
      '.note-content .desc',
    ]),
  ]),
  kuaishou: Object.freeze([
    Object.freeze([
      '.short-video-info-container-detail .video-info-title',
      '.video-info-title',
    ]),
  ]),
});

function normalizedPlatformText(value) {
  return String(value || "")
    .replaceAll("\r\n", "\n")
    .replaceAll("\r", "\n")
    .split("\n")
    .map((line) => line
      .replaceAll(/[\t\p{Zs}]+/gu, " ")
      .replaceAll(/\p{Cc}/gu, "")
      .trim())
    .filter(Boolean)
    .join("\n")
    .trim()
    .slice(0, 32_768);
}

async function firstSelectorText(scope, selectors) {
  for (const selector of selectors) {
    const values = await scope.locator(selector).evaluateAll((nodes) =>
      nodes.map((node) => node.innerText || node.textContent || ""),
    ).catch(() => []);
    for (const value of values) {
      const normalized = normalizedPlatformText(value);
      if (normalized) return normalized;
    }
  }
  return "";
}

function appendDistinctText(parts, candidate) {
  if (!candidate || parts.some((part) => part === candidate || part.includes(candidate))) return;
  const containedIndex = parts.findIndex((part) => candidate.includes(part));
  if (containedIndex >= 0) parts[containedIndex] = candidate;
  else parts.push(candidate);
}

export async function capturePlatformCaption(scope, platform, fallback = "") {
  const groups = PLATFORM_CAPTION_SELECTOR_GROUPS[platform] || [];
  const parts = [];
  for (const selectors of groups) {
    appendDistinctText(parts, await firstSelectorText(scope, selectors));
  }
  const fallbackText = normalizedPlatformText(fallback);
  if (parts.length === 0) return fallbackText;
  if (parts.length < groups.length) appendDistinctText(parts, fallbackText);
  return parts.join("\n");
}

function normalizedMetricDisplay(value) {
  const display = String(value || "").replaceAll(/\s+/gu, " ").trim();
  if (
    !display
    || display.length > 32
    || !/\p{Nd}/u.test(display)
    || [...display].some((character) => /\p{Cc}/u.test(character))
  ) {
    return null;
  }
  return display;
}

function exactMetricValue(display) {
  if (!/^(?:0|[1-9]\d*|[1-9]\d{0,2}(?:,\d{3})+)$/u.test(display)) return null;
  const value = Number(display.replaceAll(",", ""));
  return Number.isSafeInteger(value) ? value : null;
}

export async function captureEngagementMetrics(scope, platform, capturedAt) {
  const metrics = {};
  for (const [name, selectors] of Object.entries(ENGAGEMENT_SELECTORS[platform] || {})) {
    let display = null;
    for (const selector of selectors) {
      const candidates = await scope.locator(selector).evaluateAll((nodes) => nodes.map((node) => ({
        machineValue: node.getAttribute("data-count") || node.getAttribute("data-value") || "",
        renderedValue: node.textContent || "",
      })));
      for (const candidate of candidates) {
        display = normalizedMetricDisplay(candidate.machineValue)
          || normalizedMetricDisplay(candidate.renderedValue);
        if (display) break;
      }
      if (display) break;
    }
    if (!display) continue;
    const value = exactMetricValue(display);
    metrics[name] = value == null ? { display } : { display, value };
  }
  return {
    schema_version: 1,
    platform,
    captured_at: capturedAt,
    metrics,
  };
}

export function guiAvailable(environment = process.env, platform = process.platform) {
  if (platform === "darwin") return true;
  if (platform !== "linux") return false;
  return Boolean(environment.DISPLAY || environment.WAYLAND_DISPLAY);
}

async function existingExecutable() {
  const candidates = process.platform === "darwin"
    ? [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/opt/homebrew/bin/chromium",
        "/usr/local/bin/chromium",
      ]
    : [
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/snap/bin/chromium",
      ];
  for (const candidate of candidates) {
    if (await fs.access(candidate).then(() => true).catch(() => false)) return candidate;
  }
  return null;
}

export async function browserCapability() {
  return {
    gui_available: guiAvailable(),
    chromium_executable: await existingExecutable(),
    default_modes: Object.fromEntries(SUPPORTED_PLATFORMS.map(platform => [platform, resolveBrowserMode(platform)])),
    supported_modes: ["visible", "silent"],
    capture_mode: "browser_element_screenshot",
  };
}

export function pacingDelayMs(config = {}, random = Math.random, multiplier = 1) {
  const minimum = Math.max(200, Number(config.pacing_min_delay_ms) || 700);
  const maximum = Math.max(minimum, Number(config.pacing_max_delay_ms) || 1800);
  const sample = Math.min(0.999999, Math.max(0, Number(random()) || 0));
  return Math.round((minimum + (maximum - minimum) * sample) * multiplier);
}

async function pacingWait(page, config, multiplier = 1) {
  await page.waitForTimeout(pacingDelayMs(config, Math.random, multiplier));
}

async function pacedScroll(page, config) {
  const fraction = 0.62 + Math.random() * 0.28;
  await page.evaluate((scrollFraction) => {
    window.scrollBy(0, Math.max(360, window.innerHeight * scrollFraction));
  }, fraction);
  await pacingWait(page, config);
}

export function renderedCardMediaKind({ visibleVideoCount, visibleImageCount, hasImageCarousel }) {
  if (visibleVideoCount > 0) return "video";
  return visibleImageCount > 1 || hasImageCarousel ? "image" : "video";
}

export function xiaohongshuFeedCardMediaKind(hasPlayControl) {
  return hasPlayControl ? "video" : "image";
}

export function detailNavigationError(platform, requestedUrl, currentUrl, loginFormPresent) {
  if (!isDetailUrl(platform, requestedUrl) || isDetailUrl(platform, currentUrl)) return null;
  const current = new URL(currentUrl);
  if (platform === "xiaohongshu" && current.pathname === "/explore") return "login_required";
  return loginFormPresent ? "login_required" : "challenge_required";
}

export function platformAccessError(platform, currentUrl, captchaFrameUrls = []) {
  let current;
  try {
    current = new URL(currentUrl);
  } catch {
    return "challenge_required";
  }
  if (
    platform === "xiaohongshu"
    && current.pathname === "/website-login/error"
    && current.searchParams.has("error_code")
  ) {
    if (current.searchParams.get("error_code") === "300012") return "network_access_restricted";
    return "challenge_required";
  }
  if (
    platform === "douyin"
    && captchaFrameUrls.some((value) => {
      try {
        return new URL(value).pathname.includes("/verifycenter/captcha/");
      } catch {
        return false;
      }
    })
  ) {
    return "challenge_required";
  }
  return null;
}

export async function currentPlatformAccessError(page, platform) {
  const captchaFrameUrls = await page.locator("iframe[src]").evaluateAll((frames) =>
    frames.map((frame) => frame.src || ""),
  );
  const accessError = platformAccessError(platform, page.url(), captchaFrameUrls);
  if (accessError) return accessError;
  if (platform === "xiaohongshu"
    && await page.locator('.login-modal.reds-modal-open .login-container:visible').count()) {
    return "login_required";
  }
  if (await page.locator('input[type="password"]:visible, input[type="tel"]:visible').count()) {
    return "login_required";
  }
  return null;
}

export async function waitForPlatformFeed(page, platform, config, shouldStop, timeoutMs = NAVIGATION_TIMEOUT_MS) {
  const deadline = Date.now() + timeoutMs;
  const selector = platform === "xiaohongshu"
    ? "section.note-item[data-note-id]" : '.video-card a[href*="/short-video/"]';
  while (Date.now() < deadline) {
    if (await shouldStop()) throw browserStageError("collection_stopped", "feed_ready");
    const accessError = await accessErrorAfterExplicitVisibleWait(page, platform, config, shouldStop);
    if (accessError) throw browserStageError(accessError, "feed_ready");
    const ready = await page.locator(selector).evaluateAll((nodes, platform) => nodes.some(node => {
      if (platform === "xiaohongshu") return /^[A-Za-z0-9_-]+$/u.test(node.getAttribute("data-note-id") || "");
      try { return /^\/short-video\/[A-Za-z0-9_-]{8,}(?:\/|$)/u.test(new URL(node.href).pathname); }
      catch { return false; }
    }), platform);
    if (ready) return;
    await page.waitForTimeout(250);
  }
  throw browserStageError("selector_drift", "feed_ready");
}

export async function accessErrorAfterExplicitVisibleWait(page, platform, config = {}, shouldStop = async () => false) {
  let accessError = await currentPlatformAccessError(page, platform);
  const interactive = code => ["challenge_required", "login_required"].includes(code);
  if (!interactive(accessError) || config.browser_mode !== "visible") return accessError;
  const configuredMinutes = Math.max(1, Number(config.max_run_minutes) || 10);
  const deadline = Date.now() + Math.min(INTERACTIVE_LOGIN_TIMEOUT_MS, configuredMinutes * 60 * 1000);
  while (Date.now() < deadline && !page.isClosed()) {
    if (await shouldStop()) throw browserStageError("collection_stopped", "manual_verification");
    await page.waitForTimeout(INTERACTIVE_CHALLENGE_POLL_MS).catch(() => {});
    if (page.isClosed()) break;
    accessError = await currentPlatformAccessError(page, platform);
    if (!accessError) return null;
    if (!interactive(accessError)) return accessError;
  }
  return page.isClosed() ? "interactive_verification_cancelled" : "interactive_verification_timeout";
}

async function withVisibleAccess(page, platform, config, shouldStop, operation) {
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const accessError = await accessErrorAfterExplicitVisibleWait(page, platform, config, shouldStop);
    if (accessError) throw new Error(accessError);
    try {
      return await operation();
    } catch (error) {
      if (attempt > 0 || config.browser_mode !== "visible"
        || !["login_required", "challenge_required"].includes(error.message)) throw error;
    }
  }
}

async function platformAuthenticationPresent(context, platform) {
  const expected = PLATFORM_AUTH_COOKIE_NAMES[platform];
  if (!expected) return false;
  const cookies = await context.cookies().catch(() => []);
  return cookies.some((cookie) => expected.has(cookie.name) && Boolean(cookie.value));
}

export async function waitForManualAccess({ page, context, platform, errorCode, timeoutMs,
  shouldStop = async () => false }) {
  const deadline = Date.now() + Math.max(INTERACTIVE_LOGIN_POLL_MS, timeoutMs);
  let readyPolls = 0;
  while (Date.now() < deadline) {
    if (await shouldStop()) return { ready: false, error_code: "collection_stopped" };
    if (page.isClosed()) return { ready: false, error_code: "interactive_verification_cancelled" };
    const accessError = await currentPlatformAccessError(page, platform).catch(error => {
      if (page.isClosed()) return "interactive_verification_cancelled";
      throw error;
    });
    if (accessError === "interactive_verification_cancelled") return { ready: false, error_code: accessError };
    if (accessError === "network_access_restricted") return { ready: false, error_code: accessError };
    let ready = false;
    try {
      validatePlatformUrl(platform, page.url());
      const selector = {
        douyin: '[data-aweme-id], [data-e2e="video-detail"], video',
        xiaohongshu: 'section.note-item[data-note-id], #detail-title, #detail-desc',
        kuaishou: '.video-card, .short-video-info-container-detail, video',
      }[platform];
      ready = !accessError && await page.locator(selector).count() > 0
        && (errorCode !== "login_required" || await platformAuthenticationPresent(context, platform));
    } catch {
      // User navigation can be transient; only a verified platform document is ready.
    }
    readyPolls = ready ? readyPolls + 1 : 0;
    if (readyPolls >= 2) return { ready: true, reason_code: "interactive_access_ready" };
    await page.waitForTimeout(INTERACTIVE_LOGIN_POLL_MS).catch(() => {});
  }
  return { ready: false, error_code: "interactive_verification_timeout" };
}

export async function waitForInteractiveLogin({
  root,
  platform,
  config,
  timeoutMs = INTERACTIVE_LOGIN_TIMEOUT_MS,
  errorCode = "login_required",
  shouldStop = async () => false,
}) {
  if (await shouldStop()) return { ready: false, error_code: "collection_stopped" };
  if (!guiAvailable()) return { ready: false, error_code: "display_unavailable" };
  const executablePath = await existingExecutable();
  if (!executablePath) return { ready: false, error_code: "browser_missing" };

  const { chromium } = await import("playwright");
  const profile = path.join(root, "browser-profile", platform);
  await fs.mkdir(profile, { recursive: true });
  const context = await chromium.launchPersistentContext(profile, {
    executablePath,
    headless: false,
    viewport: { width: 1280, height: 900 },
    args: process.platform === "linux" && process.env.WAYLAND_DISPLAY
      ? ["--ozone-platform=wayland"]
      : [],
  });
  const page = context.pages()[0] || (await context.newPage());
  page.setDefaultTimeout(NAVIGATION_TIMEOUT_MS);
  const loginTarget = sourceTargets(platform, {
    ...config,
    source_mode: "home_feed",
    topics: [],
    seed_urls: [],
  })[0]?.url;
  try {
    if (loginTarget) {
      await page.goto(loginTarget, {
        waitUntil: "domcontentloaded",
        timeout: NAVIGATION_TIMEOUT_MS,
      }).catch(() => {});
    }
    return await waitForManualAccess({ page, context, platform, errorCode, timeoutMs, shouldStop });
  } finally {
    await context.close().catch(() => {});
  }
}

export async function discoverCandidates(page, platform, sourceUrl, maxScrolls, limit, shouldStop, config) {
  const discovered = [];
  if (isDetailUrl(platform, sourceUrl)) discovered.push(validatePlatformUrl(platform, sourceUrl));
  for (let scroll = 0; scroll <= maxScrolls && discovered.length < limit; scroll += 1) {
    const links = await page.locator("a[href]").evaluateAll((anchors) =>
      anchors.map((anchor) => anchor.href).filter((href) => typeof href === "string"),
    );
    discovered.push(...canonicalCandidateUrls(platform, links));
    if (platform === "douyin") {
      const itemIds = await page.locator("[data-aweme-id]").evaluateAll((nodes) =>
        nodes.map((node) => node.getAttribute("data-aweme-id")).filter((value) => /^\d+$/u.test(value || "")),
      );
      discovered.push(...itemIds.map((itemId) => `https://www.douyin.com/video/${itemId}`));
    }
    const unique = canonicalCandidateUrls(platform, discovered);
    discovered.length = 0;
    discovered.push(...unique);
    if (discovered.length >= limit || scroll === maxScrolls || (await shouldStop())) break;
    await pacedScroll(page, config);
  }
  return discovered.slice(0, limit);
}

export async function candidatesForDiscoverySource(
  page,
  platform,
  sourceUrl,
  config,
  limit,
  shouldStop,
) {
  if ((config.source_mode || "home_feed") === "seed_urls" && isDetailUrl(platform, sourceUrl)) {
    return [validatePlatformUrl(platform, sourceUrl)];
  }
  const candidateBudget = Math.min(100, Math.max(limit * 3, limit + 5));
  return discoverCandidates(
    page,
    platform,
    sourceUrl,
    config.max_scrolls_per_source || 10,
    candidateBudget,
    shouldStop,
    config,
  );
}

export async function activeDouyinFeedEntry(page, excludedItemIds = []) {
  const excluded = new Set(excludedItemIds);
  const entries = await page.locator("[data-aweme-id]").evaluateAll((nodes) =>
    nodes.map((node, index) => {
      const rect = node.getBoundingClientRect();
      const overlapWidth = Math.max(0, Math.min(rect.right, window.innerWidth) - Math.max(rect.left, 0));
      const overlapHeight = Math.max(0, Math.min(rect.bottom, window.innerHeight) - Math.max(rect.top, 0));
      const visibleArea = overlapWidth * overlapHeight;
      const centerDistance = Math.abs((rect.top + rect.bottom) / 2 - window.innerHeight / 2);
      return {
        index,
        itemId: node.getAttribute("data-aweme-id") || "",
        visibleArea,
        centerDistance,
      };
    }),
  ).catch(() => []);
  entries.sort((left, right) =>
    right.visibleArea - left.visibleArea || left.centerDistance - right.centerDistance);
  const observed = new Set();
  for (const entry of entries) {
    if (
      entry.visibleArea <= 0
      || !/^\d+$/u.test(entry.itemId)
      || excluded.has(entry.itemId)
      || observed.has(entry.itemId)
    ) {
      continue;
    }
    observed.add(entry.itemId);
    return entry;
  }
  try {
    const itemId = platformItemId("douyin", validatePlatformUrl("douyin", page.url())).split(":").at(-1);
    if (/^\d+$/u.test(itemId || "") && !excluded.has(itemId)) {
      return { index: null, itemId, visibleArea: 0, centerDistance: 0 };
    }
  } catch {
    // A recommendation page is not itself a detail item.
  }
  return null;
}

async function waitForDouyinFeedEntry(page, excludedItemIds = [], timeoutMs = 10_000, config = {}, shouldStop = async () => false) {
  let deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline && !page.isClosed()) {
    if (await shouldStop()) throw browserStageError("collection_stopped", "feed_ready");
    const accessStarted = Date.now();
    const accessError = await accessErrorAfterExplicitVisibleWait(page, "douyin", config, shouldStop);
    deadline += Date.now() - accessStarted;
    if (accessError) throw browserStageError(accessError, "feed_ready");
    const entry = await activeDouyinFeedEntry(page, excludedItemIds);
    if (entry) return entry;
    await page.waitForTimeout(200).catch(() => {});
  }
  return null;
}

async function douyinRecommendationWebUrl(page, entry) {
  const card = page.locator("[data-aweme-id]").nth(entry.index);
  const expectedPath = `/video/${entry.itemId}`;
  const cardLinks = card.locator('a[href*="/video/"]');
  const cardLinkCount = Math.min(await cardLinks.count(), 12);
  for (let index = 0; index < cardLinkCount; index += 1) {
    const link = cardLinks.nth(index);
    const href = await link.getAttribute("href").catch(() => null);
    if (!href) continue;
    try {
      const candidate = new URL(validatePlatformUrl("douyin", new URL(href, page.url()).href));
      if (candidate.pathname === expectedPath || candidate.pathname.startsWith(`${expectedPath}/`)) {
        return `${candidate.origin}${candidate.pathname}`;
      }
    } catch {
      // Ignore malformed page-owned links and continue with structural fallbacks.
    }
  }
  // A recommendation card may launch the desktop app. Its machine ID also
  // identifies the platform's HTTPS detail page, without invoking that handler.
  return `https://www.douyin.com${expectedPath}`;
}

export async function openDouyinRecommendationDetail(page, config = {}, shouldStop = async () => false) {
  if (isDetailUrl("douyin", page.url())) {
    return {
      page,
      entry: await waitForDouyinFeedEntry(page, [], 10_000, config, shouldStop),
    };
  }
  const recommendation = await waitForDouyinFeedEntry(page, [], NAVIGATION_TIMEOUT_MS, config, shouldStop);
  if (!recommendation || recommendation.index == null) throw browserStageError("selector_drift", "recommendation_ready");
  const targetUrl = await douyinRecommendationWebUrl(page, recommendation);
  const detailPath = `/video/${recommendation.itemId}`;
  const destination = page;
  const response = await destination.goto(targetUrl, { waitUntil: "domcontentloaded", timeout: NAVIGATION_TIMEOUT_MS });
  if (response && [401, 403, 429].includes(response.status())) {
    throw browserStageError(response.status() === 429 ? "rate_limited" : "challenge_required", "detail_navigation");
  }
  destination.setDefaultTimeout(NAVIGATION_TIMEOUT_MS);
  await destination.waitForLoadState("domcontentloaded", { timeout: NAVIGATION_TIMEOUT_MS }).catch(() => {});
  await pacingWait(destination, config, 1.25);
  const accessError = await accessErrorAfterExplicitVisibleWait(destination, "douyin", config, shouldStop);
  if (accessError) throw new Error(accessError);
  let currentUrl;
  try {
    currentUrl = validatePlatformUrl("douyin", destination.url());
  } catch {
    throw new Error("challenge_required");
  }
  const navigationError = detailNavigationError(
    "douyin",
    `https://www.douyin.com${detailPath}`,
    currentUrl,
    (await destination.locator('input[type="password"], input[type="tel"]').count()) > 0,
  );
  if (navigationError) throw browserStageError(navigationError, "detail_navigation");
  const entry = await waitForDouyinFeedEntry(destination, [], 10_000, config, shouldStop);
  return {
    page: destination,
    entry: entry || {
      index: null,
      itemId: recommendation.itemId,
      visibleArea: 0,
      centerDistance: 0,
    },
  };
}

export async function advanceDouyinDetailFeed(page, previousItemId, config = {}, shouldStop = async () => false) {
  if (await shouldStop()) throw browserStageError("collection_stopped", "next_item");
  const nextControl = page.locator('[data-e2e="video-switch-next-arrow"]').first();
  if (await nextControl.isVisible() && await nextControl.isEnabled()) {
    await nextControl.click({ timeout: NAVIGATION_TIMEOUT_MS });
    const next = await waitForDouyinFeedEntry(page, [previousItemId], 10_000, config, shouldStop);
    if (next) return next;
    throw browserStageError("selector_drift", "next_item");
  }
  const viewport = page.viewportSize() || { width: 1280, height: 900 };
  for (let attempt = 0; attempt < 3; attempt += 1) {
    await page.mouse.move(
      Math.round(viewport.width * (0.46 + Math.random() * 0.08)),
      Math.round(viewport.height * (0.46 + Math.random() * 0.08)),
    );
    await page.mouse.wheel(0, Math.round(viewport.height * (0.82 + Math.random() * 0.18)));
    await pacingWait(page, config, 0.75);
    const next = await waitForDouyinFeedEntry(page, [previousItemId], 4_000, config, shouldStop);
    if (next) return next;
    await page.keyboard.press("ArrowDown").catch(() => {});
    await pacingWait(page, config, 0.5);
    const keyboardNext = await waitForDouyinFeedEntry(page, [previousItemId], 2_000, config, shouldStop);
    if (keyboardNext) return keyboardNext;
  }
  return null;
}

async function pageMetadata(page, platform, requestedUrl) {
  const metadata = await page.evaluate(() => {
    const meta = (selector) => document.querySelector(selector)?.getAttribute("content")?.trim() || "";
    const canonical = document.querySelector('link[rel="canonical"]')?.href || location.href;
    return {
      canonical,
      title: meta('meta[property="og:title"]') || document.title || "",
      description:
        meta('meta[property="og:description"]') || meta('meta[name="description"]') || "",
      hasVideo: document.querySelectorAll("video").length > 0,
    };
  });
  let canonicalUrl = requestedUrl;
  try {
    canonicalUrl = validatePlatformUrl(platform, metadata.canonical);
  } catch {
    // Keep the already validated requested URL when a page supplies an invalid canonical value.
  }
  return {
    ...metadata,
    canonical: canonicalUrl,
    platformText: await capturePlatformCaption(page, platform, metadata.description),
  };
}

async function visibleImageCandidates(page, maximum) {
  const candidates = await page.locator("img").evaluateAll((images) =>
    images.map((image, index) => {
      const rect = image.getBoundingClientRect();
      const style = window.getComputedStyle(image);
      return {
        index,
        width: rect.width,
        height: rect.height,
        area: rect.width * rect.height,
        visible:
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          Number.parseFloat(style.opacity || "1") > 0.01 &&
          rect.width >= 180 &&
          rect.height >= 180 &&
          rect.bottom > 0 &&
          rect.right > 0 &&
          rect.top < window.innerHeight &&
          rect.left < window.innerWidth,
        source: image.currentSrc || image.src || "",
      };
    }),
  );
  return candidates
    .filter((candidate) => candidate.visible)
    .sort((left, right) => right.area - left.area)
    .filter((candidate, index, values) =>
      candidate.source && values.findIndex((value) => value.source === candidate.source) === index)
    .slice(0, maximum)
    .sort((left, right) => left.index - right.index);
}

const CAROUSEL_NEXT_SELECTORS = Object.freeze({
  douyin: [
    '[data-e2e="arrow-right"]:visible',
    '[data-e2e*="slide-right"]:visible',
    '.swiper-button-next:not(.swiper-button-disabled):visible',
    '[class*="carousel"] [class*="next"]:visible',
  ],
  xiaohongshu: [
    '.swiper-button-next:not(.swiper-button-disabled):visible',
    '[class*="arrow-controller"][class*="right"]:visible',
    '[class*="carousel"] [class*="next"]:visible',
    '[class*="swiper"] [class*="next"]:visible',
  ],
});

async function nextCarouselControl(scope, platform) {
  for (const selector of CAROUSEL_NEXT_SELECTORS[platform] || []) {
    const control = scope.locator(selector).first();
    if ((await control.count()) === 0) continue;
    if (!(await control.isEnabled().catch(() => false))) continue;
    return control;
  }
  return null;
}

async function clickNextCarousel(scope, platform, config) {
  const control = await nextCarouselControl(scope, platform);
  if (!control) return false;
  await control.click({ timeout: 5000 }).catch(() => {});
  const page = typeof scope.page === "function" ? scope.page() : scope;
  await pacingWait(page, config, 0.5);
  return true;
}

export async function collectRenderedImages({
  scope,
  root,
  runId,
  platform,
  itemId,
  title,
  platformText,
  sourcePageUrl,
  discoverySource,
  config,
  discoveredAt,
  engagement,
}) {
  const maximum = Math.min(100, config.max_images_per_post || 100);
  const records = [];
  const temporaryPaths = [];
  const observedSources = new Set();
  let unchangedTurns = 0;
  while (records.length < maximum) {
    const candidates = await visibleImageCandidates(scope, maximum - records.length);
    let added = 0;
    for (const candidate of candidates) {
      if (observedSources.has(candidate.source)) continue;
      const position = records.length + 1;
      const screenshotPath = path.join(
        root,
        "tmp",
        runId,
        `${itemId.replaceAll(":", "_")}-image-${String(position).padStart(3, "0")}.png`,
      );
      await screenshotLocator(scope.locator("img").nth(candidate.index), screenshotPath, platform);
      temporaryPaths.push(screenshotPath);
      const imageScreenshotPath = await persistImageScreenshot(
        root,
        platform,
        itemId,
        position,
        screenshotPath,
      );
      observedSources.add(candidate.source);
      records.push({
        kind: "image",
        dedup_key: `${itemId}:image:${position}`,
        platform,
        browser_mode: config.browser_mode || "silent",
        source_mode: discoverySource?.source_mode || config.source_mode || "home_feed",
        search_keyword: discoverySource?.search_keyword || "",
        discovery_source_url: discoverySource?.url || sourcePageUrl,
        item_id: itemId,
        image_sequence: position,
        title,
        platform_text: platformText,
        image_url: candidate.source,
        image_screenshot_path: imageScreenshotPath,
        cover_screenshot_path: imageScreenshotPath,
        source_page_url: sourcePageUrl,
        discovered_at: discoveredAt,
        engagement,
      });
      added += 1;
      if (records.length >= maximum) break;
    }
    unchangedTurns = added === 0 ? unchangedTurns + 1 : 0;
    if (records.length >= maximum || unchangedTurns >= 2 || !(await clickNextCarousel(scope, platform, config))) break;
  }
  if (records.length === 0) throw new Error("media_element_not_found");
  if (records.length >= maximum && (await nextCarouselControl(scope, platform))) {
    records.at(-1).collection_truncated = true;
  }
  return { records, temporaryPaths };
}

export async function screenshotLocator(locator, targetPath, platform) {
  const page = locator.page();
  const assertCaptureReady = async () => {
    const accessError = await currentPlatformAccessError(page, platform);
    if (accessError) throw new Error(accessError);
    if (!await locatorIsUsableCover(locator, 1)) throw new Error("screenshot_obscured");
  };
  await locator.scrollIntoViewIfNeeded();
  await assertCaptureReady();
  await fs.mkdir(path.dirname(targetPath), { recursive: true });
  const temporary = `${targetPath}.tmp-${process.pid}`;
  try {
    await locator.screenshot({ path: temporary, type: "png", timeout: NAVIGATION_TIMEOUT_MS });
    await assertCaptureReady();
  } catch (error) {
    await fs.unlink(temporary).catch(() => {});
    throw error;
  }
  const stat = await fs.stat(temporary);
  if (stat.size < SCREENSHOT_MIN_BYTES) {
    await fs.unlink(temporary).catch(() => {});
    throw new Error("screenshot_empty");
  }
  await fs.rename(temporary, targetPath);
  return targetPath;
}

async function persistVideoCover(root, platform, itemId, temporaryPath) {
  const token = `${platform}_${itemId}`.replaceAll(/[^A-Za-z0-9._-]/gu, "_").slice(0, 180);
  const relativePath = path.posix.join("video_covers", `${token}.png`);
  const targetPath = path.join(root, "exports", ...relativePath.split("/"));
  await fs.mkdir(path.dirname(targetPath), { recursive: true });
  await fs.copyFile(temporaryPath, targetPath, fsConstants.COPYFILE_EXCL).catch((error) => {
    if (error?.code !== "EEXIST") throw error;
  });
  return relativePath;
}

async function persistImageScreenshot(root, platform, itemId, position, temporaryPath) {
  const token = `${platform}_${itemId}`.replaceAll(/[^A-Za-z0-9._-]/gu, "_").slice(0, 170);
  const suffix = String(position).padStart(3, "0");
  const relativePath = path.posix.join("images", `${token}_${suffix}.png`);
  const targetPath = path.join(root, "exports", ...relativePath.split("/"));
  await fs.mkdir(path.dirname(targetPath), { recursive: true });
  await fs.copyFile(temporaryPath, targetPath, fsConstants.COPYFILE_EXCL).catch((error) => {
    if (error?.code !== "EEXIST") throw error;
  });
  return relativePath;
}

const VIDEO_COVER_SELECTORS = Object.freeze({
  douyin: Object.freeze({
    rendered_video_frame: Object.freeze([
      '[data-e2e="video-player"] video:visible',
      'video:visible',
    ]),
    rendered_poster_image: Object.freeze([
      '[data-e2e="video-poster"] img:visible',
      '[data-e2e="video-cover"] img:visible',
      '.discover-video-card-img:visible',
    ]),
  }),
  xiaohongshu: Object.freeze({
    rendered_video_frame: Object.freeze([
      '.video-player video:visible',
      'video:visible',
    ]),
    rendered_poster_image: Object.freeze([
      '.video-player img:visible',
      '.note-slider img:visible',
      '.swiper img:visible',
      'a.cover img:visible',
    ]),
  }),
  kuaishou: Object.freeze({
    rendered_video_frame: Object.freeze([
      '.player-video:visible',
      'video:visible',
    ]),
    rendered_poster_image: Object.freeze([
      '.video-card .poster-img:visible',
      '.poster .poster-img:visible',
    ]),
  }),
});

async function locatorIsUsableCover(locator, minimumWidth = 180) {
  return locator.evaluate((node, widthFloor) => {
    const rect = node.getBoundingClientRect();
    if (rect.width < widthFloor || rect.height < 120) return false;
    const card = node.closest("[data-aweme-id], [data-note-id]");
    const left = Math.max(0, rect.left), top = Math.max(0, rect.top);
    const right = Math.min(window.innerWidth, rect.right), bottom = Math.min(window.innerHeight, rect.bottom);
    if (right <= left || bottom <= top) return false;
    return [0.15, 0.5, 0.85].every(xRatio => [0.15, 0.5, 0.85].every(yRatio => {
      const topNode = document.elementFromPoint(left + (right - left) * xRatio, top + (bottom - top) * yRatio);
      return topNode === node || node.contains(topNode)
        || (card && topNode instanceof Element && topNode.closest("[data-aweme-id], [data-note-id]") === card);
    }));
  }, minimumWidth).catch(() => false);
}

export async function renderedVideoCover(scope, platform) {
  for (const [source, selectors] of Object.entries(VIDEO_COVER_SELECTORS[platform] || {})) {
    for (const selector of selectors) {
      const candidates = scope.locator(selector);
      const count = Math.min(await candidates.count(), 8);
      for (let index = 0; index < count; index += 1) {
        const locator = candidates.nth(index);
        const minimumWidth = platform === "kuaishou" ? 140 : 180;
        if (await locatorIsUsableCover(locator, minimumWidth)) return { locator, source };
      }
    }
  }
  return null;
}

async function freezeVideoIfPresent(locator) {
  await locator.evaluate((node) => {
    if (node instanceof HTMLVideoElement) node.pause();
  }).catch(() => {});
}

async function collectPage(page, root, runId, platform, itemUrl, config, discoverySource) {
  const response = await page.goto(itemUrl, {
    waitUntil: "domcontentloaded",
    timeout: NAVIGATION_TIMEOUT_MS,
  });
  if (response && [401, 403, 429].includes(response.status())) {
    throw new Error(response.status() === 429 ? "rate_limited" : "challenge_required");
  }
  await pacingWait(page, config, 1.25);
  const accessError = await currentPlatformAccessError(page, platform);
  if (accessError) throw browserStageError(accessError, "detail_access");
  let currentUrl = "";
  try {
    currentUrl = validatePlatformUrl(platform, page.url());
  } catch {
    throw new Error("challenge_required");
  }
  const navigationError = detailNavigationError(
    platform,
    itemUrl,
    currentUrl,
    (await page.locator('input[type="password"], input[type="tel"]').count()) > 0,
  );
  if (navigationError) throw new Error(navigationError);
  const metadata = await pageMetadata(page, platform, itemUrl);
  const itemId = platformItemId(platform, metadata.canonical);
  const discoveredAt = new Date().toISOString();
  const engagement = await captureEngagementMetrics(page, platform, discoveredAt);
  const temporaryRoot = path.join(root, "tmp", runId);
  if (metadata.hasVideo) {
    const screenshotPath = path.join(temporaryRoot, `${itemId.replaceAll(":", "_")}-video.png`);
    const cover = await renderedVideoCover(page, platform);
    if (!cover) throw new Error("media_element_not_found");
    await freezeVideoIfPresent(cover.locator);
    await screenshotLocator(cover.locator, screenshotPath, platform);
    const coverScreenshotPath = await persistVideoCover(root, platform, itemId, screenshotPath);
    return {
      records: [{
        kind: "video",
        dedup_key: `${itemId}:video`,
        platform,
        browser_mode: config.browser_mode || "silent",
        source_mode: discoverySource.source_mode,
        search_keyword: discoverySource.search_keyword || "",
        discovery_source_url: discoverySource.url,
        item_id: itemId,
        title: metadata.title,
        platform_text: metadata.platformText,
        cover_screenshot_path: coverScreenshotPath,
        cover_capture_source: cover.source,
        video_page_url: metadata.canonical,
        discovered_at: discoveredAt,
        engagement,
      }],
      temporaryPaths: [screenshotPath],
    };
  }
  return collectRenderedImages({
    scope: page,
    root,
    runId,
    platform,
    itemId,
    title: metadata.title,
    platformText: metadata.platformText,
    sourcePageUrl: metadata.canonical,
    discoverySource,
    config,
    discoveredAt,
    engagement,
  });
}

async function collectDouyinFeedCard(page, root, runId, locator, itemId, config, discoverySource) {
  const card = await locator.evaluate((node) => ({
    title:
      node.getAttribute("aria-label") ||
      node.querySelector("img")?.getAttribute("alt") ||
      "",
  }));
  const platformText = await capturePlatformCaption(locator, "douyin", card.title);
  const discoveredAt = new Date().toISOString();
  const engagement = await captureEngagementMetrics(locator, "douyin", discoveredAt);
  const pageUrl = `https://www.douyin.com/video/${itemId}`;
  const visibleVideoCount = await locator.locator("video:visible").count();
  const visibleImages = await visibleImageCandidates(locator, 3);
  const hasImageCarousel = Boolean(await nextCarouselControl(locator, "douyin"));
  if (renderedCardMediaKind({
    visibleVideoCount,
    visibleImageCount: visibleImages.length,
    hasImageCarousel,
  }) === "image") {
    return collectRenderedImages({
      scope: locator,
      root,
      runId,
      platform: "douyin",
      itemId: `douyin:${itemId}`,
      title: card.title,
      platformText,
      sourcePageUrl: pageUrl,
      discoverySource,
      config,
      discoveredAt,
      engagement,
    });
  }
  const screenshotPath = path.join(root, "tmp", runId, `douyin_${itemId}-video.png`);
  const cover = await renderedVideoCover(locator, "douyin");
  if (!cover) throw new Error("media_element_not_found");
  await freezeVideoIfPresent(cover.locator);
  await screenshotLocator(cover.locator, screenshotPath, "douyin");
  const coverScreenshotPath = await persistVideoCover(root, "douyin", itemId, screenshotPath);
  return {
    records: [{
      kind: "video",
      dedup_key: `douyin:${itemId}:video`,
      platform: "douyin",
      browser_mode: config.browser_mode || "silent",
      source_mode: discoverySource.source_mode,
      search_keyword: discoverySource.search_keyword || "",
      discovery_source_url: discoverySource.url,
      item_id: `douyin:${itemId}`,
      title: card.title,
      platform_text: platformText,
      cover_screenshot_path: coverScreenshotPath,
      cover_capture_source: cover.source,
      video_page_url: pageUrl,
      discovered_at: discoveredAt,
      engagement,
    }],
    temporaryPaths: [screenshotPath],
  };
}

async function collectDouyinRecommendationFeed(
  page,
  root,
  runId,
  config,
  discoverySource,
  limit,
  shouldStop,
  onPage,
  onFailure,
) {
  const seen = new Set();
  let handled = 0;
  let lastError = null;
  const maxScrolls = config.max_scrolls_per_source || 10;
  const opened = await openDouyinRecommendationDetail(page, config, shouldStop);
  const detailPage = opened.page;
  let current = opened.entry;
  for (let scroll = 0; scroll <= maxScrolls && handled < limit && current; scroll += 1) {
    if (await shouldStop()) break;
    if (!seen.has(current.itemId)) {
      seen.add(current.itemId);
      try {
        await pacingWait(detailPage, config, 0.5);
        const scope = current.index == null
          ? detailPage.locator("body")
          : detailPage.locator("[data-aweme-id]").nth(current.index);
        const result = await collectDouyinFeedCard(
          detailPage,
          root,
          runId,
          scope,
          current.itemId,
          config,
          discoverySource,
        );
        await onPage(result);
        handled += 1;
      } catch (error) {
        lastError = error;
        await onFailure?.(error);
      }
    }
    if (handled >= limit || scroll === maxScrolls || (await shouldStop())) break;
    current = await advanceDouyinDetailFeed(detailPage, current.itemId, config, shouldStop);
  }
  if (handled === 0 && !(await shouldStop())) throw lastError || new Error("selector_drift");
  return handled;
}

async function collectKuaishouFeedCard(
  root,
  runId,
  locator,
  itemUrl,
  config,
  discoverySource,
) {
  const card = await locator.evaluate((node) => ({
    title:
      node.querySelector(".video-info-title")?.textContent ||
      node.querySelector("img")?.getAttribute("alt") ||
      "",
  }));
  const itemId = platformItemId("kuaishou", itemUrl);
  const platformText = await capturePlatformCaption(locator, "kuaishou", card.title);
  const discoveredAt = new Date().toISOString();
  const engagement = await captureEngagementMetrics(locator, "kuaishou", discoveredAt);
  const screenshotPath = path.join(
    root,
    "tmp",
    runId,
    `${itemId.replaceAll(":", "_")}-video.png`,
  );
  const cover = await renderedVideoCover(locator, "kuaishou");
  if (!cover) throw new Error("media_element_not_found");
  await screenshotLocator(cover.locator, screenshotPath, "kuaishou");
  const coverScreenshotPath = await persistVideoCover(root, "kuaishou", itemId, screenshotPath);
  return {
    records: [{
      kind: "video",
      dedup_key: `${itemId}:video`,
      platform: "kuaishou",
      browser_mode: config.browser_mode || "silent",
      source_mode: discoverySource.source_mode,
      search_keyword: discoverySource.search_keyword || "",
      discovery_source_url: discoverySource.url,
      item_id: itemId,
      title: normalizedPlatformText(card.title),
      platform_text: platformText,
      cover_screenshot_path: coverScreenshotPath,
      cover_capture_source: cover.source,
      video_page_url: itemUrl,
      discovered_at: discoveredAt,
      engagement,
    }],
    temporaryPaths: [screenshotPath],
  };
}

async function collectXiaohongshuFeedCard(
  root,
  runId,
  locator,
  itemId,
  config,
  discoverySource,
) {
  const card = await locator.evaluate((node) => ({
    title: node.querySelector(".title")?.textContent || "",
    hasPlayControl: Boolean(
      node.querySelector('.play-icon, use[href="#play-s"], use[xlink\\:href="#play-s"]'),
    ),
  }));
  const canonicalItemId = `xiaohongshu:${itemId}`;
  const pageUrl = `https://www.xiaohongshu.com/explore/${itemId}`;
  const title = normalizedPlatformText(card.title);
  const platformText = await capturePlatformCaption(locator, "xiaohongshu", title);
  const discoveredAt = new Date().toISOString();
  const engagement = await captureEngagementMetrics(locator, "xiaohongshu", discoveredAt);
  if (xiaohongshuFeedCardMediaKind(card.hasPlayControl) === "image") {
    return collectRenderedImages({
      scope: locator,
      root,
      runId,
      platform: "xiaohongshu",
      itemId: canonicalItemId,
      title,
      platformText,
      sourcePageUrl: pageUrl,
      discoverySource,
      config,
      discoveredAt,
      engagement,
    });
  }
  const screenshotPath = path.join(
    root,
    "tmp",
    runId,
    `${canonicalItemId.replaceAll(":", "_")}-video.png`,
  );
  const cover = await renderedVideoCover(locator, "xiaohongshu");
  if (!cover) throw new Error("media_element_not_found");
  await screenshotLocator(cover.locator, screenshotPath, "xiaohongshu");
  const coverScreenshotPath = await persistVideoCover(
    root,
    "xiaohongshu",
    canonicalItemId,
    screenshotPath,
  );
  return {
    records: [{
      kind: "video",
      dedup_key: `${canonicalItemId}:video`,
      platform: "xiaohongshu",
      browser_mode: config.browser_mode || "silent",
      source_mode: discoverySource.source_mode,
      search_keyword: discoverySource.search_keyword || "",
      discovery_source_url: discoverySource.url,
      item_id: canonicalItemId,
      title,
      platform_text: platformText,
      cover_screenshot_path: coverScreenshotPath,
      cover_capture_source: cover.source,
      video_page_url: pageUrl,
      discovered_at: discoveredAt,
      engagement,
    }],
    temporaryPaths: [screenshotPath],
  };
}

export async function collectXiaohongshuHomeFeed(
  page,
  root,
  runId,
  config,
  discoverySource,
  limit,
  shouldStop,
  onPage,
  onFailure,
) {
  const seen = new Set();
  let handled = 0;
  let lastError = null;
  const maxScrolls = config.max_scrolls_per_source || 10;
  await waitForPlatformFeed(page, "xiaohongshu", config, shouldStop);
  for (let scroll = 0; scroll <= maxScrolls && handled < limit; scroll += 1) {
    const accessError = await accessErrorAfterExplicitVisibleWait(page, "xiaohongshu", config, shouldStop);
    if (accessError) throw browserStageError(accessError, "feed_scroll");
    const cards = await page.locator("section.note-item[data-note-id]").evaluateAll((nodes) =>
      nodes.map((node) => {
        const rect = node.getBoundingClientRect();
        return {
          itemId: node.getAttribute("data-note-id") || "",
          visible: rect.width >= 140 && rect.height >= 120 && rect.bottom > 0 && rect.top < window.innerHeight,
        };
      }),
    );
    for (const card of cards) {
      if (handled >= limit || (await shouldStop())) break;
      if (!/^[A-Za-z0-9_-]+$/u.test(card.itemId) || !card.visible || seen.has(card.itemId)) continue;
      seen.add(card.itemId);
      try {
        await pacingWait(page, config, 0.5);
        const result = await withVisibleAccess(page, "xiaohongshu", config, shouldStop, () => collectXiaohongshuFeedCard(
          root,
          runId,
          page.locator(`section.note-item[data-note-id="${card.itemId}"]`),
          card.itemId,
          config,
          discoverySource,
        ));
        await onPage(result);
        handled += 1;
      } catch (error) {
        lastError = error;
        await onFailure?.(error);
        if (["login_required", "challenge_required", "network_access_restricted", "collection_stopped",
          "interactive_verification_cancelled", "interactive_verification_timeout"].includes(error.message)) throw error;
      }
    }
    if (handled >= limit || scroll === maxScrolls || (await shouldStop())) break;
    await pacedScroll(page, config);
  }
  if (handled === 0 && !(await shouldStop())) throw lastError || new Error("selector_drift");
  return handled;
}

async function collectKuaishouHomeFeed(
  page,
  root,
  runId,
  config,
  discoverySource,
  limit,
  shouldStop,
  onPage,
  onFailure,
) {
  const seen = new Set();
  let handled = 0;
  let lastError = null;
  const maxScrolls = config.max_scrolls_per_source || 10;
  await waitForPlatformFeed(page, "kuaishou", config, shouldStop);
  for (let scroll = 0; scroll <= maxScrolls && handled < limit; scroll += 1) {
    const cards = await page.locator(".video-card").evaluateAll((nodes) =>
      nodes.map((node, index) => {
        const rect = node.getBoundingClientRect();
        return {
          index,
          itemUrl: node.querySelector('a[href*="/short-video/"]')?.href || "",
          visible: rect.width >= 140 && rect.height >= 120 && rect.bottom > 0 && rect.top < window.innerHeight,
        };
      }),
    );
    for (const card of cards) {
      if (handled >= limit || (await shouldStop())) break;
      if (!card.visible || !isDetailUrl("kuaishou", card.itemUrl) || seen.has(card.itemUrl)) continue;
      seen.add(card.itemUrl);
      try {
        await pacingWait(page, config, 0.5);
        const result = await collectKuaishouFeedCard(
          root,
          runId,
          page.locator(".video-card").nth(card.index),
          validatePlatformUrl("kuaishou", card.itemUrl),
          config,
          discoverySource,
        );
        await onPage(result);
        handled += 1;
      } catch (error) {
        lastError = error;
        await onFailure?.(error);
      }
    }
    if (handled >= limit || scroll === maxScrolls || (await shouldStop())) break;
    await pacedScroll(page, config);
  }
  if (handled === 0 && !(await shouldStop())) throw lastError || new Error("selector_drift");
  return handled;
}

export async function collectPlatform({ root, runId, platform, config, limit, shouldStop, onPage, onFailure }) {
  const browserMode = resolveBrowserMode(platform, config.browser_mode);
  config = { ...config, browser_mode: browserMode };
  if (browserMode === "visible" && !guiAvailable()) throw new Error("display_unavailable");
  const executablePath = await existingExecutable();
  if (!executablePath) throw new Error("browser_missing");
  const { chromium } = await import("playwright");
  const profile = path.join(root, "browser-profile", platform);
  await fs.mkdir(profile, { recursive: true });
  const context = await chromium.launchPersistentContext(profile, {
    executablePath,
    headless: browserMode === "silent",
    viewport: { width: 1280, height: 900 },
    args: browserMode === "visible" && process.platform === "linux" && process.env.WAYLAND_DISPLAY
      ? ["--ozone-platform=wayland"]
      : [],
  });
  const page = context.pages()[0] || (await context.newPage());
  page.setDefaultTimeout(NAVIGATION_TIMEOUT_MS);
  let handled = 0;
  let lastError = null;
  let stage = "source_navigation";
  try {
    for (const discoverySource of sourceTargets(platform, config)) {
      const sourceUrl = discoverySource.url;
      if (handled >= limit || (await shouldStop())) break;
      stage = "source_navigation";
      const response = await page.goto(sourceUrl, { waitUntil: "domcontentloaded", timeout: NAVIGATION_TIMEOUT_MS });
      if (response && [401, 403, 429].includes(response.status())) {
        throw browserStageError(response.status() === 429 ? "rate_limited" : "challenge_required", stage);
      }
      await pacingWait(page, config, 1.25);
      stage = "source_access";
      const accessError = await accessErrorAfterExplicitVisibleWait(page, platform, config, shouldStop);
      if (accessError) throw new Error(accessError);
      stage = "collect_items";
      if (platform === "douyin" && (config.source_mode || "home_feed") === "home_feed") {
        handled += await collectDouyinRecommendationFeed(
          page,
          root,
          runId,
          config,
          discoverySource,
          limit - handled,
          shouldStop,
          onPage,
          onFailure,
        );
        continue;
      }
      if (platform === "kuaishou" && (config.source_mode || "home_feed") === "home_feed") {
        handled += await collectKuaishouHomeFeed(
          page,
          root,
          runId,
          config,
          discoverySource,
          limit - handled,
          shouldStop,
          onPage,
          onFailure,
        );
        continue;
      }
      if (platform === "xiaohongshu" && (config.source_mode || "home_feed") === "home_feed") {
        handled += await collectXiaohongshuHomeFeed(
          page,
          root,
          runId,
          config,
          discoverySource,
          limit - handled,
          shouldStop,
          onPage,
          onFailure,
        );
        continue;
      }
      const candidates = await candidatesForDiscoverySource(
        page,
        platform,
        sourceUrl,
        config,
        limit - handled,
        shouldStop,
      );
      if (candidates.length === 0 && (await page.locator('input[type="password"]').count()) > 0) {
        throw new Error("login_required");
      }
      for (const candidate of candidates) {
        if (handled >= limit || (await shouldStop())) break;
        try {
          await pacingWait(page, config, 0.5);
          const result = await collectPage(
            page,
            root,
            runId,
            platform,
            candidate,
            config,
            discoverySource,
          );
          await onPage(result);
          handled += 1;
        } catch (error) {
          lastError = error;
          await onFailure?.(error);
          if (["login_required", "challenge_required", "network_access_restricted", "rate_limited"].includes(String(error?.message))) {
            throw error;
          }
        }
      }
    }
    if (
      handled === 0 &&
      (config.source_mode || "home_feed") === "home_feed" &&
      !(await shouldStop())
    ) {
      throw lastError || new Error("selector_drift");
    }
    return { handled };
  } catch (error) {
    if (error.message !== "collection_stopped") {
      await recordBrowserFailure(page, { root, runId, platform, stage, error }).catch(() => {});
    }
    throw error;
  } finally {
    await context.close().catch(() => {});
  }
}
