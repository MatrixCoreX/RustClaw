import fs from "node:fs/promises";
import { constants as fsConstants } from "node:fs";
import path from "node:path";
import { browserStageError, recordBrowserFailure } from "./browser_diagnostics.mjs";
import { createManualConfirmation } from "./manual_handoff.mjs";
import { launchPlatformBrowser } from "./browser_environment.mjs";
import { capturePublication } from "./publication.mjs";
import { assertNavigationResponse, boundedBrowserOperation, normalizeBrowserError, openKeywordSearch } from "./browser_search.mjs";
import { assertBrowserFlow, observePlatformBackpressure, pacingDelayMs, stopsCollection } from "./browser_flow_control.mjs";
export { pacingDelayMs } from "./browser_flow_control.mjs";
import { withSearchResult } from "./browser_search_results.mjs";
import {
  collectKuaishouSearchResults,
  kuaishouAccessError,
  kuaishouLoadMoreLoginVisible,
  revealKuaishouLoginSurface,
} from "./browser_kuaishou_search.mjs";
import { collectDouyinSearchResults } from "./browser_douyin_search.mjs";
import { imageSourceIdentity, identityDigest } from "./media_identity.mjs";
import { collectOrderedPages, optionalLimit, pageProgress } from "./collection_progress.mjs";

import {
  canonicalCandidateUrls,
  isDetailUrl,
  manualVerificationTarget,
  matchesManualAccessTarget,
  resolveBrowserMode,
  SUPPORTED_PLATFORMS,
  platformItemId,
  sourceTargets,
  validatePlatformUrl,
} from "./platforms.mjs";

const NAVIGATION_TIMEOUT_MS = 45_000;
const SCREENSHOT_MIN_BYTES = 512;
const SCREENSHOT_MIN_BYTES_PER_PIXEL = 0.03;

export function screenshotLooksBlankFromBytes(buffer) {
  if (!Buffer.isBuffer(buffer) || buffer.length < SCREENSHOT_MIN_BYTES) return true;
  if (buffer.length < 24 || buffer[0] !== 0x89 || buffer.subarray(1, 4).toString("ascii") !== "PNG") {
    return buffer.length < SCREENSHOT_MIN_BYTES;
  }
  const width = buffer.readUInt32BE(16);
  const height = buffer.readUInt32BE(20);
  const pixels = width * height;
  if (!Number.isSafeInteger(pixels) || pixels < 1) return true;
  return buffer.length / pixels < SCREENSHOT_MIN_BYTES_PER_PIXEL;
}

export async function screenshotLooksBlank(filePath) {
  return screenshotLooksBlankFromBytes(await fs.readFile(filePath));
}

export async function awaitPaintedVideoFrame(locator, timeoutMs = 5_000) {
  const page = locator.page();
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline && !page.isClosed()) {
    const painted = await locator.evaluate((node) => {
      if (!(node instanceof HTMLVideoElement)) return true;
      if (!node.currentSrc && !node.src) return false;
      node.muted = true;
      node.playsInline = true;
      if (node.paused) node.play().catch(() => {});
      if (node.readyState < 2 || node.videoWidth < 8 || node.videoHeight < 8) return false;
      const canvas = document.createElement("canvas");
      canvas.width = 24;
      canvas.height = 24;
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      if (!ctx) return node.currentTime > 0;
      ctx.drawImage(node, 0, 0, 24, 24);
      const pixels = ctx.getImageData(0, 0, 24, 24).data;
      let min = 255;
      let max = 0;
      for (let index = 0; index < pixels.length; index += 4) {
        const luma = (pixels[index] + pixels[index + 1] + pixels[index + 2]) / 3;
        if (luma < min) min = luma;
        if (luma > max) max = luma;
      }
      return (max - min) >= 10;
    }).catch(() => false);
    if (painted) return true;
    await page.waitForTimeout(150).catch(() => {});
  }
  return false;
}
export const INTERACTIVE_LOGIN_TIMEOUT_MS = 10 * 60 * 1000;
const INTERACTIVE_LOGIN_POLL_MS = 1000;
const INTERACTIVE_CHALLENGE_POLL_MS = 1000;

const PLATFORM_AUTH_COOKIE_NAMES = Object.freeze({
  douyin: new Set(["sessionid", "sessionid_ss", "sid_guard", "uid_tt", "uid_tt_ss"]),
  xiaohongshu: new Set(["web_session"]),
  kuaishou: new Set(["kuaishou.server.web_st", "kuaishou.server.webday7_st", "userId"]),
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
      '.photo-btns .like-btn',
    ]),
    comments: Object.freeze(['.interactive-item.comment-item .item-count', '[data-testid="comment-count"]', '.photo-btns .commentPanel']),
    favorites: Object.freeze(['.interactive-item.collect-item .item-count', '[data-testid="collect-count"]', '.photo-btns .favorite']),
    shares: Object.freeze(['.interactive-item.share-item .item-count', '[data-testid="share-count"]']),
    views: Object.freeze(['.video-info-content:has(.play-icon) .info-text', '[data-testid="play-count"]']),
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
      '.caption',
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

function genericDouyinListingTitle(value) {
  const text = normalizedPlatformText(value);
  return !text || /发现更多精彩视频/u.test(text) || /抖音搜索/u.test(text);
}

export function douyinRecordCopy({ authorTitle = "", listingTitle = "" } = {}) {
  const author = normalizedPlatformText(authorTitle);
  const listing = normalizedPlatformText(listingTitle);
  return {
    title: author || (genericDouyinListingTitle(listing) ? "" : listing),
    platform_text: "",
  };
}

async function firstSelectorText(scope, selectors, options = {}) {
  for (const selector of selectors) {
    const values = await scope.locator(selector).evaluateAll((nodes, exclude) =>
      nodes.flatMap((node) => {
        if (exclude && node.closest(exclude)) return [];
        const rect = node.getBoundingClientRect();
        const style = getComputedStyle(node);
        if (rect.width <= 0 || rect.height <= 0 || style.display === "none" || style.visibility === "hidden") {
          return [];
        }
        return [node.innerText || node.textContent || ""];
      }), options.excludeClosest || "").catch(() => []);
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
  const excludeClosest = platform === "douyin" ? ".search-result-card" : "";
  const parts = [];
  for (const selectors of groups) {
    appendDistinctText(parts, await firstSelectorText(scope, selectors, { excludeClosest }));
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
      const candidates = await scope.locator(selector).evaluateAll((nodes, platformName) => nodes.filter(node => {
        const rect = node.getBoundingClientRect();
        const style = getComputedStyle(node);
        return rect.width > 0 && rect.height > 0 && style.display !== "none" && style.visibility !== "hidden"
          && !node.closest('[data-comment-id], .comments-container')
          && !(platformName === "douyin" && node.closest(".search-result-card"));
      }).map((node) => ({
        machineValue: node.getAttribute("data-count") || node.getAttribute("data-value") || "",
        renderedValue: node instanceof HTMLElement ? node.innerText : node.textContent || "",
      })), platform);
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

async function pacingWait(page, config, multiplier = 1) {
  assertBrowserFlow(page);
  await page.waitForTimeout(pacingDelayMs(config, Math.random, multiplier));
  assertBrowserFlow(page);
}

async function pacedScroll(page, config) {
  assertBrowserFlow(page, "feed_scroll");
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
  if (/^\/404\/?$/u.test(current.pathname)) return "source_unavailable";
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
  assertBrowserFlow(page, "access_check");
  const captchaFrameUrls = await boundedBrowserOperation(page.locator("iframe[src]").evaluateAll((frames) =>
    frames.map((frame) => frame.src || ""),
  ), 10_000, "access_check");
  const accessError = platformAccessError(platform, page.url(), captchaFrameUrls);
  if (accessError) return accessError;
  if (platform === "xiaohongshu"
    && await page.locator('.login-modal.reds-modal-open .login-container:visible').count()) {
    return "login_required";
  }
  if (platform === "kuaishou") {
    const kuaishouError = await kuaishouAccessError(page);
    if (kuaishouError) return kuaishouError;
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
  if (platform === "kuaishou"
    && await kuaishouLoadMoreLoginVisible(page)
    && await page.locator(".video-list .photo-card:visible").count() === 0) {
    await revealKuaishouLoginSurface(page).catch(() => {});
  }
  let accessError = await currentPlatformAccessError(page, platform);
  const interactive = code => ["challenge_required", "login_required"].includes(code);
  if (!interactive(accessError) || config.browser_mode !== "visible") return accessError;
  const configuredMinutes = Math.max(1, Number(config.max_run_minutes) || 10);
  const deadline = Date.now() + Math.min(INTERACTIVE_LOGIN_TIMEOUT_MS, configuredMinutes * 60 * 1000);
  let readyPolls = 0;
  while (Date.now() < deadline && !page.isClosed()) {
    if (await shouldStop()) throw browserStageError("collection_stopped", "manual_verification");
    await page.waitForTimeout(INTERACTIVE_CHALLENGE_POLL_MS).catch(() => {});
    if (page.isClosed()) break;
    accessError = await currentPlatformAccessError(page, platform);
    if (!accessError) {
      readyPolls += 1;
      if (readyPolls >= 2) return null;
      continue;
    }
    readyPolls = 0;
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
  confirmation, targetUrl, shouldStop = async () => false }) {
  if (!confirmation?.readAction) throw new Error("manual_confirmation_required");
  const deadline = Date.now() + Math.max(INTERACTIVE_LOGIN_POLL_MS, timeoutMs);
  let readyPolls = 0;
  while (Date.now() < deadline) {
    if (await shouldStop()) return { ready: false, error_code: "collection_stopped" };
    if (page.isClosed()) return { ready: false, error_code: "interactive_verification_cancelled" };
    const action = await confirmation.readAction();
    if (action === "pause") return { ready: false, error_code: "interactive_verification_cancelled" };
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
        douyin: '[data-aweme-id], [data-e2e="video-detail"], video, a[href*="/video/"], a[href*="/note/"], .search-result-card',
        xiaohongshu: 'section.note-item[data-note-id], #detail-title, #detail-desc, a[href*="/explore/"]',
        kuaishou: '.video-card, .video-list .photo-card, .short-video-info-container-detail, video, a[href*="/short-video/"]',
      }[platform];
      const visibleSelector = selector.split(",").map(part => `${part.trim()}:visible`).join(",");
      ready = action === "continue" && !accessError && await page.locator(visibleSelector).count() > 0
        && matchesManualAccessTarget(platform, targetUrl, page.url())
        && (errorCode !== "login_required" || await platformAuthenticationPresent(context, platform));
    } catch {
      // User navigation can be transient; only a verified platform document is ready.
    }
    readyPolls = ready ? readyPolls + 1 : 0;
    if (readyPolls >= 2) return { ready: true, reason_code: "interactive_access_ready", user_confirmed: true };
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
  targetUrl,
  shouldStop = async () => false,
  locale = process.env.LC_MESSAGES || process.env.LANG || "en",
  onOpened = async () => {},
}) {
  if (await shouldStop()) return { ready: false, error_code: "collection_stopped" };
  if (!guiAvailable()) return { ready: false, error_code: "display_unavailable" };
  const executablePath = await existingExecutable();
  if (!executablePath) return { ready: false, error_code: "browser_missing" };

  const { chromium } = await import("playwright");
  const context = await launchPlatformBrowser({ chromium, root, platform, executablePath, headless: false });
  try {
    const page = context.pages()[0] || (await context.newPage());
    page.setDefaultTimeout(NAVIGATION_TIMEOUT_MS);
    const loginTarget = manualVerificationTarget(platform, config, targetUrl);
    if (loginTarget) {
      await page.goto(loginTarget, {
        waitUntil: "domcontentloaded",
        timeout: NAVIGATION_TIMEOUT_MS,
      }).catch(() => {});
    }
    if (platform === "kuaishou") await revealKuaishouLoginSurface(page).catch(() => {});
    const confirmation = await createManualConfirmation(context, page, { platform, locale });
    await onOpened();
    return await waitForManualAccess({ page, context, platform, errorCode, timeoutMs,
      confirmation, shouldStop, targetUrl: loginTarget });
  } finally {
    await context.close().catch(() => {});
  }
}

export async function discoverCandidates(page, platform, sourceUrl, maxScrolls, limit, shouldStop, config) {
  const discovered = [];
  const progressing = pageProgress();
  if (isDetailUrl(platform, sourceUrl)) discovered.push(validatePlatformUrl(platform, sourceUrl));
  for (let scroll = 0; scroll <= optionalLimit(maxScrolls) && discovered.length < limit; scroll += 1) {
    const links = await visibleDiscoveryCandidates(page, platform);
    if (!progressing(links)) break;
    discovered.push(...links);
    const unique = canonicalCandidateUrls(platform, discovered);
    discovered.length = 0;
    discovered.push(...unique);
    if (discovered.length >= limit || scroll === optionalLimit(maxScrolls) || (await shouldStop())) break;
    await pacedScroll(page, config);
  }
  return discovered.slice(0, limit);
}

async function visibleDiscoveryCandidates(page, platform) {
  const selector = platform === "douyin" ? "a[href], [data-aweme-id]" : "a[href]";
  const links = await page.locator(selector).evaluateAll((nodes) => nodes.flatMap(node => {
      const rect = node.getBoundingClientRect(), style = getComputedStyle(node);
      if (rect.width <= 0 || rect.height <= 0 || style.visibility === "hidden"
        || style.display === "none" || Number(style.opacity) === 0) return [];
      const itemId = node.getAttribute("data-aweme-id");
      return [node.href, /^\d+$/u.test(itemId || "") ? `https://www.douyin.com/video/${itemId}` : null]
        .filter(value => typeof value === "string");
  }));
  return canonicalCandidateUrls(platform, links);
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
  const candidateBudget = limit;
  return discoverCandidates(
    page,
    platform,
    sourceUrl,
    config.max_scrolls_per_source,
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
  assertNavigationResponse(response, "detail_navigation");
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

async function douyinFallbackCover(scope) {
  for (const selector of ["video:visible", "canvas:visible"]) {
    const nodes = scope.locator(selector);
    const count = Math.min(await nodes.count(), 8);
    for (let index = 0; index < count; index += 1) {
      const locator = nodes.nth(index);
      const box = await locator.boundingBox();
      if (box && box.width >= 180 && box.height >= 120) {
        return { locator, source: "rendered_video_frame" };
      }
    }
  }
  return null;
}

async function largeVisibleVideo(scope) {
  const videos = scope.locator("video:visible");
  const count = Math.min(await videos.count(), 8);
  for (let index = 0; index < count; index += 1) {
    const box = await videos.nth(index).boundingBox();
    if (box && box.width >= 180 && box.height >= 120) return true;
  }
  return false;
}

async function pageMetadata(page, platform, requestedUrl, scope = page) {
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
  const authorTitle = platform === "douyin"
    ? await capturePlatformCaption(scope, "douyin", "")
    : "";
  const copy = platform === "douyin"
    ? douyinRecordCopy({ authorTitle, listingTitle: metadata.title })
    : null;
  return {
    ...metadata,
    canonical: isDetailUrl(platform, canonicalUrl)
      && platformItemId(platform, canonicalUrl) === platformItemId(platform, requestedUrl) ? canonicalUrl : requestedUrl,
    hasVideo: await largeVisibleVideo(scope),
    title: copy ? copy.title : scope === page
      ? metadata.title
      : await firstSelectorText(scope, PLATFORM_CAPTION_SELECTOR_GROUPS[platform]?.[0] || []),
    platformText: copy ? copy.platform_text
      : await capturePlatformCaption(scope, platform, scope === page ? metadata.description : ""),
  };
}

async function visibleImageCandidates(page, maximum) {
  const candidates = await page.locator("img").evaluateAll((images) =>
    images.map((image, index) => {
      const rect = image.getBoundingClientRect();
      const style = window.getComputedStyle(image);
      let left = Math.max(0, rect.left), right = Math.min(innerWidth, rect.right);
      let top = Math.max(0, rect.top), bottom = Math.min(innerHeight, rect.bottom);
      for (let parent = image.parentElement; parent; parent = parent.parentElement) {
        const bounds = parent.getBoundingClientRect(), css = getComputedStyle(parent);
        if (["hidden", "clip", "scroll", "auto"].includes(css.overflowX)) {
          left = Math.max(left, bounds.left); right = Math.min(right, bounds.right);
        }
        if (["hidden", "clip", "scroll", "auto"].includes(css.overflowY)) {
          top = Math.max(top, bounds.top); bottom = Math.min(bottom, bounds.bottom);
        }
      }
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
          right - left >= 180 && bottom - top >= 180,
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
  const page = typeof scope.page === "function" ? scope.page() : scope;
  assertBrowserFlow(page, "carousel_next");
  const control = await nextCarouselControl(scope, platform);
  if (!control) return false;
  await control.click({ timeout: 5000 }).catch(() => {});
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
  publication = {},
}) {
  const maximum = optionalLimit(config.max_images_per_post);
  const records = [];
  const temporaryPaths = [];
  const observedSources = new Set();
  let unchangedTurns = 0;
  while (records.length < maximum) {
    const candidates = await visibleImageCandidates(scope, maximum - records.length);
    let added = 0;
    for (const candidate of candidates) {
      const identity = imageSourceIdentity(platform, candidate.source);
      if (observedSources.has(identity)) continue;
      const imageDigest = identityDigest(identity);
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
        imageDigest,
        screenshotPath,
      );
      observedSources.add(identity);
      records.push({
        kind: "image",
        dedup_key: `${itemId}:image:${imageDigest}`,
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
        ...publication,
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

export async function screenshotLocator(locator, targetPath, platform, mediaReadyTimeoutMs = 10_000, options = {}) {
  const page = locator.page();
  const assertCaptureReady = async () => {
    const accessError = await currentPlatformAccessError(page, platform);
    if (accessError) throw new Error(accessError);
    if (!options.allowObscured && !await locatorIsUsableCover(locator, 1, platform)) throw new Error("screenshot_obscured");
  };
  await locator.scrollIntoViewIfNeeded();
  const mediaDeadline = Date.now() + mediaReadyTimeoutMs;
  while (true) {
    await assertCaptureReady();
    const imageState = await locator.evaluate(node => node instanceof HTMLImageElement
      ? { complete: node.complete, ready: node.naturalWidth > 0 && node.naturalHeight > 0 } : null);
    if (!imageState || (imageState.complete && imageState.ready)) break;
    if (imageState.complete || Date.now() >= mediaDeadline) throw new Error("media_not_ready");
    await page.waitForTimeout(Math.min(200, Math.max(1, mediaDeadline - Date.now())));
  }
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
  try {
    await fs.copyFile(temporaryPath, targetPath, fsConstants.COPYFILE_EXCL);
  } catch (error) {
    if (error?.code !== "EEXIST") throw error;
    if (await screenshotLooksBlank(targetPath) && !await screenshotLooksBlank(temporaryPath)) {
      await fs.copyFile(temporaryPath, targetPath);
    }
  }
  return relativePath;
}

async function persistImageScreenshot(root, platform, itemId, imageDigest, temporaryPath) {
  const token = `${platform}_${itemId}`.replaceAll(/[^A-Za-z0-9._-]/gu, "_").slice(0, 170);
  const relativePath = path.posix.join("images", `${token}_${imageDigest}.png`);
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

async function locatorIsUsableCover(locator, minimumWidth = 180, platform = "") {
  return locator.evaluate((node, args) => {
    const widthFloor = args.widthFloor;
    const platformName = args.platform;
    const rect = node.getBoundingClientRect();
    if (rect.width < widthFloor || rect.height < 120) return false;
    const card = node.closest("[data-aweme-id], [data-note-id]");
    const player = node.matches("video.kplayer-video")
      ? node.closest(".swiper-feed .swiper-slide-active .video-container") : null;
    const douyinPlayer = node.closest('[data-e2e="video-player"], .xgplayer, xg-video-container');
    const left = Math.max(0, rect.left), top = Math.max(0, rect.top);
    const right = Math.min(window.innerWidth, rect.right), bottom = Math.min(window.innerHeight, rect.bottom);
    if (right <= left || bottom <= top) return false;
    const hit = (xRatio, yRatio) => {
      const topNode = document.elementFromPoint(left + (right - left) * xRatio, top + (bottom - top) * yRatio);
      if (!(topNode instanceof Element)) return false;
      if (topNode === node || node.contains(topNode)) return true;
      if (card && topNode.closest("[data-aweme-id], [data-note-id]") === card) return true;
      if (player && topNode.closest(".video-container") === player
        && topNode.closest(".video-interact-panel, .volume-control-wrapper")) return true;
      if (douyinPlayer && (douyinPlayer.contains(topNode) || Boolean(topNode.closest(
        '[data-e2e="video-player"], [data-e2e^="video-"], [data-e2e="feed-active-video"], .xgplayer, .xg-controls, xg-controls',
      )))) return true;
      if (platformName === "douyin" && node.parentElement?.contains(topNode)) {
        const style = getComputedStyle(topNode);
        const background = style.backgroundColor.replaceAll(" ", "");
        if (style.opacity === "0" || background === "transparent" || background === "rgba(0,0,0,0)") return true;
      }
      return false;
    };
    if ([0.15, 0.5, 0.85].every(xRatio => [0.15, 0.5, 0.85].every(yRatio => hit(xRatio, yRatio)))) return true;
    return (platformName === "douyin" || douyinPlayer || card) && hit(0.5, 0.5);
  }, { widthFloor: minimumWidth, platform }).catch(() => false);
}

export async function renderedVideoCover(scope, platform) {
  for (const [source, selectors] of Object.entries(VIDEO_COVER_SELECTORS[platform] || {})) {
    for (const selector of selectors) {
      const candidates = scope.locator(selector);
      const count = Math.min(await candidates.count(), 8);
      for (let index = 0; index < count; index += 1) {
        const locator = candidates.nth(index);
        const minimumWidth = platform === "kuaishou" ? 140 : 180;
        if (await locatorIsUsableCover(locator, minimumWidth, platform)) return { locator, source };
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

async function capturePaintedCover({
  scope, platform, screenshotPath, root, itemId, fallbackCoverPng = null,
}) {
  const tryLocator = async (locator, source) => {
    if (!locator) return null;
    const isVideo = await locator.evaluate((node) => node instanceof HTMLVideoElement).catch(() => false);
    if (isVideo && !await awaitPaintedVideoFrame(locator)) return null;
    if (isVideo) await freezeVideoIfPresent(locator);
    try {
      await screenshotLocator(locator, screenshotPath, platform, 10_000, {
        allowObscured: platform === "douyin",
      });
    } catch {
      return null;
    }
    if (await screenshotLooksBlank(screenshotPath)) return null;
    return {
      coverScreenshotPath: await persistVideoCover(root, platform, itemId, screenshotPath),
      source,
    };
  };
  const rendered = await renderedVideoCover(scope, platform);
  let captured = rendered ? await tryLocator(rendered.locator, rendered.source) : null;
  if (!captured && platform === "douyin" && !(rendered?.source === "rendered_video_frame")) {
    const fallback = await douyinFallbackCover(scope);
    captured = fallback ? await tryLocator(fallback.locator, fallback.source) : null;
  }
  if (!captured && fallbackCoverPng && !screenshotLooksBlankFromBytes(Buffer.from(fallbackCoverPng))) {
    await fs.mkdir(path.dirname(screenshotPath), { recursive: true });
    await fs.writeFile(screenshotPath, fallbackCoverPng);
    captured = {
      coverScreenshotPath: await persistVideoCover(root, platform, itemId, screenshotPath),
      source: "search_tile",
    };
  }
  return captured;
}

async function collectPage(page, root, runId, platform, itemUrl, config, discoverySource) {
  const response = await page.goto(itemUrl, {
    waitUntil: "domcontentloaded",
    timeout: NAVIGATION_TIMEOUT_MS,
  });
  assertNavigationResponse(response, "detail_navigation");
  return collectOpenedPage(page, root, runId, platform, itemUrl, config, discoverySource);
}

async function collectOpenedPage(page, root, runId, platform, itemUrl, config, discoverySource, scopeOverride = null, coverOptions = {}) {
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
  if (navigationError && !scopeOverride) throw new Error(navigationError);
  const detail = scopeOverride || (platform === "xiaohongshu" ? page.locator(".note-container:visible").first() : null);
  if (detail) await detail.waitFor({ state: "visible", timeout: NAVIGATION_TIMEOUT_MS });
  const scope = detail || page;
  const metadata = await pageMetadata(page, platform, itemUrl, scope);
  const sourceUrl = scopeOverride ? itemUrl : metadata.canonical;
  const itemId = platformItemId(platform, sourceUrl);
  const discoveredAt = new Date().toISOString();
  const engagement = await captureEngagementMetrics(scope, platform, discoveredAt);
  const publication = await capturePublication(scope, platform, itemId);
  const temporaryRoot = path.join(root, "tmp", runId);
  const screenshotPath = path.join(temporaryRoot, `${itemId.replaceAll(":", "_")}-video.png`);
  const cover = (metadata.hasVideo || coverOptions.fallbackCoverPng)
    ? await capturePaintedCover({
      scope,
      platform,
      screenshotPath,
      root,
      itemId,
      fallbackCoverPng: coverOptions.fallbackCoverPng,
    })
    : null;
  if (cover) {
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
          cover_screenshot_path: cover.coverScreenshotPath,
          cover_capture_source: cover.source,
          video_page_url: sourceUrl,
          discovered_at: discoveredAt,
          ...publication,
          engagement,
        }],
        temporaryPaths: [screenshotPath],
      };
  }
  return collectRenderedImages({
    scope,
    root,
    runId,
    platform,
    itemId,
    title: metadata.title,
    platformText: metadata.platformText,
    sourcePageUrl: sourceUrl,
    discoverySource,
    config,
    discoveredAt,
    engagement,
    publication,
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
  const publication = await capturePublication(locator, "douyin", itemId);
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
      publication,
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
      ...publication,
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
  completed = new Set(),
) {
  const seen = new Set();
  const progressing = pageProgress();
  let handled = 0;
  let lastError = null;
  const maxScrolls = optionalLimit(config.max_scrolls_per_source);
  const opened = await openDouyinRecommendationDetail(page, config, shouldStop);
  const detailPage = opened.page;
  let current = opened.entry;
  for (let scroll = 0; scroll <= maxScrolls && handled < limit && current; scroll += 1) {
    if (await shouldStop()) break;
    if (!progressing([current.itemId])) break;
    if (!seen.has(current.itemId) && !completed.has(`douyin:${current.itemId}`)) {
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
        if (stopsCollection(error)) throw error;
      }
    }
    if (handled >= limit || scroll === maxScrolls || (await shouldStop())) break;
    current = await advanceDouyinDetailFeed(detailPage, current.itemId, config, shouldStop);
  }
  if (handled === 0 && !(await shouldStop()) && (lastError || completed.size === 0)) {
    throw lastError || new Error("selector_drift");
  }
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
  const publication = await capturePublication(locator, "kuaishou", itemId);
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
      ...publication,
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
  const publication = await capturePublication(locator, "xiaohongshu", itemId);
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
      publication,
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
      ...publication,
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
  completed = new Set(),
) {
  const seen = new Set();
  const progressing = pageProgress();
  let handled = 0;
  let lastError = null;
  const maxScrolls = optionalLimit(config.max_scrolls_per_source);
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
    if (!progressing(cards.filter(card => card.visible).map(card => card.itemId))) break;
    for (const card of cards) {
      if (handled >= limit || (await shouldStop())) break;
      if (!/^[A-Za-z0-9_-]+$/u.test(card.itemId) || !card.visible || seen.has(card.itemId)
        || completed.has(`xiaohongshu:${card.itemId}`)) continue;
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
        if (stopsCollection(error)) throw error;
      }
    }
    if (handled >= limit || scroll === maxScrolls || (await shouldStop())) break;
    await pacedScroll(page, config);
  }
  if (handled === 0 && !(await shouldStop()) && (lastError || completed.size === 0)) {
    throw lastError || new Error("selector_drift");
  }
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
  completed = new Set(),
) {
  const seen = new Set();
  const progressing = pageProgress();
  let handled = 0;
  let lastError = null;
  const maxScrolls = optionalLimit(config.max_scrolls_per_source);
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
    if (!progressing(cards.filter(card => card.visible).map(card => card.itemUrl))) break;
    for (const card of cards) {
      if (handled >= limit || (await shouldStop())) break;
      if (!card.visible || !isDetailUrl("kuaishou", card.itemUrl) || seen.has(card.itemUrl)
        || completed.has(platformItemId("kuaishou", card.itemUrl))) continue;
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
        if (stopsCollection(error)) throw error;
      }
    }
    if (handled >= limit || scroll === maxScrolls || (await shouldStop())) break;
    await pacedScroll(page, config);
  }
  if (handled === 0 && !(await shouldStop()) && (lastError || completed.size === 0)) {
    throw lastError || new Error("selector_drift");
  }
  return handled;
}

export async function collectPlatform({ root, runId, platform, config, limit, shouldStop, onPage, onFailure, onSearch,
  completedPosts = new Set() }) {
  const browserMode = resolveBrowserMode(platform, config.browser_mode);
  config = { ...config, browser_mode: browserMode };
  if (browserMode === "visible" && !guiAvailable()) throw new Error("display_unavailable");
  const executablePath = await existingExecutable();
  if (!executablePath) throw new Error("browser_missing");
  const { chromium } = await import("playwright");
  const context = await launchPlatformBrowser({ chromium, root, platform, executablePath,
    headless: browserMode === "silent" });
  const stopObserving = observePlatformBackpressure(context, platform);
  let page = context.pages()[0] || (await context.newPage());
  page.setDefaultTimeout(NAVIGATION_TIMEOUT_MS);
  let handled = 0;
  let lastError = null;
  const outcomes = [];
  let stage = "source_navigation";
  let verificationTarget;
  try {
    for (let discoverySource of sourceTargets(platform, config)) {
      const sourceUrl = discoverySource.url;
      verificationTarget = sourceUrl;
      if (handled >= limit || (await shouldStop())) break;
      stage = "source_navigation";
      if (discoverySource.source_mode === "topics") {
        const opened = await openKeywordSearch(page, platform, discoverySource, {
          shouldStop,
          checkAccess: candidate => accessErrorAfterExplicitVisibleWait(candidate, platform, config, shouldStop),
          settle: candidate => pacingWait(candidate, config),
        });
        page = opened.page;
        discoverySource = opened.source;
        verificationTarget = discoverySource.url;
        page.setDefaultTimeout(NAVIGATION_TIMEOUT_MS);
        await onSearch?.({ platform, keyword: discoverySource.search_keyword,
          result_url: discoverySource.url, method: "platform_search_form", result_ready: true });
      } else {
        const response = await page.goto(sourceUrl, { waitUntil: "domcontentloaded", timeout: NAVIGATION_TIMEOUT_MS });
        assertNavigationResponse(response, stage);
      }
      await pacingWait(page, config, 1.25);
      stage = "source_access";
      const accessError = await accessErrorAfterExplicitVisibleWait(page, platform, config, shouldStop);
      if (accessError) throw new Error(accessError);
      stage = "collect_items";
      if (platform === "douyin" && discoverySource.source_mode === "topics"
        && await page.locator(".search-result-card:visible .videoImage").count()) {
        const outcome = await collectDouyinSearchResults({ page, config,
          limit: limit - handled, shouldStop, onPage, onFailure,
          completed: completedPosts, detailed: true,
          checkAccess: candidate => accessErrorAfterExplicitVisibleWait(candidate, platform, config, shouldStop),
          settle: candidate => pacingWait(candidate, config),
          scrollPage: candidate => pacedScroll(candidate, config),
          collect: (opened, url, scope, coverOptions) => collectOpenedPage(opened, root, runId, platform, url, config, discoverySource, scope, coverOptions),
        });
        outcomes.push(outcome);
        handled += outcome.handled;
        continue;
      }
      if (platform === "kuaishou" && discoverySource.source_mode === "topics"
        && (await page.locator(".video-list .photo-card").count()
          || await kuaishouLoadMoreLoginVisible(page))) {
        const outcome = await collectKuaishouSearchResults({ page, config,
          limit: limit - handled, shouldStop, onPage, onFailure,
          completed: completedPosts, detailed: true,
          checkAccess: candidate => accessErrorAfterExplicitVisibleWait(candidate, platform, config, shouldStop),
          settle: candidate => pacingWait(candidate, config),
          scrollPage: candidate => pacedScroll(candidate, config),
          collect: (opened, url, scope, coverOptions) => collectOpenedPage(opened, root, runId, platform, url, config, discoverySource, scope, coverOptions),
        });
        outcomes.push(outcome);
        handled += outcome.handled;
        continue;
      }
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
          completedPosts,
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
          completedPosts,
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
          completedPosts,
        );
        continue;
      }
      if (discoverySource.source_mode === "topics") {
        const outcome = await collectOrderedPages({
          readCandidates: async () => {
            const access = await accessErrorAfterExplicitVisibleWait(page, platform, config, shouldStop);
            if (access) throw browserStageError(access, "search_results_access");
            return visibleDiscoveryCandidates(page, platform);
          },
          identity: url => platformItemId(platform, url),
          completed: completedPosts,
          collect: async candidate => {
            verificationTarget = candidate;
            await pacingWait(page, config, 0.5);
            const result = await withSearchResult(page, platform, candidate, discoverySource,
              opened => collectOpenedPage(opened, root, runId, platform, candidate, config, discoverySource),
              { shouldStop, checkAccess: opened => currentPlatformAccessError(opened, platform) });
            await onPage(result);
          },
          scroll: () => pacedScroll(page, config),
          shouldStop, limit: limit - handled, maxScrolls: config.max_scrolls_per_source,
          onFailure, stopsCollection,
        });
        outcomes.push(outcome);
        handled += outcome.handled;
        continue;
      }
      const candidates = await candidatesForDiscoverySource(
        page,
        platform,
        discoverySource.url,
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
          verificationTarget = candidate;
          await pacingWait(page, config, 0.5);
          const result = discoverySource.source_mode === "topics"
            ? await withSearchResult(page, platform, candidate, discoverySource,
              opened => collectOpenedPage(opened, root, runId, platform, candidate, config, discoverySource),
              { shouldStop, checkAccess: opened => currentPlatformAccessError(opened, platform) })
            : await collectPage(page, root, runId, platform, candidate, config, discoverySource);
          await onPage(result);
          handled += 1;
        } catch (error) {
          lastError = error;
          await onFailure?.(error);
          if (stopsCollection(error)) throw error;
        }
      }
    }
    if (
      handled === 0 &&
      !(await shouldStop()) && outcomes.length === 0 && completedPosts.size === 0
    ) {
      throw lastError || new Error("selector_drift");
    }
    assertBrowserFlow(page, "collection_complete");
    return { handled, sources: outcomes,
      stop_reason: handled >= limit ? "target_reached" : await shouldStop() ? "collection_stopped"
        : outcomes.at(-1)?.stop_reason || "source_exhausted" };
  } catch (cause) {
    const error = normalizeBrowserError(cause, stage);
    error.discovery_target_url ||= verificationTarget;
    if (error.message !== "collection_stopped") {
      await recordBrowserFailure(page, { root, runId, platform, stage, error }).catch(() => {});
    }
    throw error;
  } finally {
    stopObserving();
    await context.close().catch(() => {});
  }
}
