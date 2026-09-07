import fs from "node:fs/promises";
import { constants as fsConstants } from "node:fs";
import path from "node:path";

import {
  canonicalCandidateUrls,
  isDetailUrl,
  platformItemId,
  sourceTargets,
  validatePlatformUrl,
} from "./platforms.mjs";
import { recognizeScreenshot } from "./recognition.mjs";

const NAVIGATION_TIMEOUT_MS = 45_000;
const SCREENSHOT_MIN_BYTES = 512;

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
    default_mode: "silent",
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

export function detailNavigationError(platform, requestedUrl, currentUrl, loginFormPresent) {
  if (!isDetailUrl(platform, requestedUrl) || isDetailUrl(platform, currentUrl)) return null;
  const current = new URL(currentUrl);
  if (platform === "xiaohongshu" && current.pathname === "/explore") return "login_required";
  return loginFormPresent ? "login_required" : "challenge_required";
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
      await screenshotLocator(scope.locator("img").nth(candidate.index), screenshotPath);
      temporaryPaths.push(screenshotPath);
      const recognition = await recognizeScreenshot(
        screenshotPath,
        config.recognition_mode || "ocr_reviewed",
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
        recognized_text: recognition.text,
        raw_recognized_text: recognition.raw_text,
        recognition,
        image_url: candidate.source,
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

async function screenshotLocator(locator, targetPath) {
  await fs.mkdir(path.dirname(targetPath), { recursive: true });
  const temporary = `${targetPath}.tmp-${process.pid}`;
  await locator.screenshot({ path: temporary, type: "png", timeout: NAVIGATION_TIMEOUT_MS });
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

const VIDEO_COVER_SELECTORS = Object.freeze({
  douyin: Object.freeze({
    rendered_video_frame: Object.freeze([
      '[data-e2e="video-player"] video:visible',
      'video:visible',
    ]),
    rendered_poster_image: Object.freeze([
      '[data-e2e="video-poster"] img:visible',
      '[data-e2e="video-cover"] img:visible',
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
    const x = Math.min(window.innerWidth - 1, Math.max(0, rect.left + rect.width / 2));
    const y = Math.min(window.innerHeight - 1, Math.max(0, rect.top + rect.height / 2));
    const topNode = document.elementFromPoint(x, y);
    return topNode === node || node.contains(topNode);
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
  const recognitionMode = config.recognition_mode || "ocr_reviewed";
  if (metadata.hasVideo) {
    const screenshotPath = path.join(temporaryRoot, `${itemId.replaceAll(":", "_")}-video.png`);
    const cover = await renderedVideoCover(page, platform);
    if (!cover) throw new Error("media_element_not_found");
    await freezeVideoIfPresent(cover.locator);
    await screenshotLocator(cover.locator, screenshotPath);
    const recognition = await recognizeScreenshot(screenshotPath, recognitionMode);
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
        recognized_text: recognition.text,
        raw_recognized_text: recognition.raw_text,
        recognition,
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
  await screenshotLocator(cover.locator, screenshotPath);
  const recognition = await recognizeScreenshot(
    screenshotPath,
    config.recognition_mode || "ocr_reviewed",
  );
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
      recognized_text: recognition.text,
      raw_recognized_text: recognition.raw_text,
      recognition,
      cover_screenshot_path: coverScreenshotPath,
      cover_capture_source: cover.source,
      video_page_url: pageUrl,
      discovered_at: discoveredAt,
      engagement,
    }],
    temporaryPaths: [screenshotPath],
  };
}

async function collectDouyinHomeFeed(
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
  for (let scroll = 0; scroll <= maxScrolls && handled < limit; scroll += 1) {
    const cards = await page.locator("[data-aweme-id]").evaluateAll((nodes) =>
      nodes.map((node, index) => {
        const rect = node.getBoundingClientRect();
        return {
          index,
          itemId: node.getAttribute("data-aweme-id") || "",
          visible: rect.width >= 180 && rect.height >= 120 && rect.bottom > 0 && rect.top < window.innerHeight,
        };
      }),
    );
    for (const card of cards) {
      if (handled >= limit || (await shouldStop())) break;
      if (!card.visible || !/^\d+$/u.test(card.itemId) || seen.has(card.itemId)) continue;
      seen.add(card.itemId);
      try {
        await pacingWait(page, config, 0.5);
        const result = await collectDouyinFeedCard(
          page,
          root,
          runId,
          page.locator("[data-aweme-id]").nth(card.index),
          card.itemId,
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
  await screenshotLocator(cover.locator, screenshotPath);
  const recognition = await recognizeScreenshot(
    screenshotPath,
    config.recognition_mode || "ocr_reviewed",
  );
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
      recognized_text: recognition.text,
      raw_recognized_text: recognition.raw_text,
      recognition,
      cover_screenshot_path: coverScreenshotPath,
      cover_capture_source: cover.source,
      video_page_url: itemUrl,
      discovered_at: discoveredAt,
      engagement,
    }],
    temporaryPaths: [screenshotPath],
  };
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
  await page.waitForFunction(
    () => [...document.querySelectorAll('.video-card a[href*="/short-video/"]')]
      .some((anchor) => {
        try {
          return /^\/short-video\/[A-Za-z0-9_-]{8,}(?:\/|$)/u.test(new URL(anchor.href).pathname);
        } catch {
          return false;
        }
      }),
    undefined,
    { timeout: NAVIGATION_TIMEOUT_MS },
  ).catch(() => {});
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
  const browserMode = config.browser_mode || "silent";
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
  try {
    for (const discoverySource of sourceTargets(platform, config)) {
      const sourceUrl = discoverySource.url;
      if (handled >= limit || (await shouldStop())) break;
      await page.goto(sourceUrl, { waitUntil: "domcontentloaded", timeout: NAVIGATION_TIMEOUT_MS });
      await pacingWait(page, config, 1.25);
      if (platform === "douyin" && (config.source_mode || "home_feed") === "home_feed") {
        handled += await collectDouyinHomeFeed(
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
          if (["login_required", "challenge_required", "rate_limited"].includes(String(error?.message))) {
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
  } finally {
    await context.close().catch(() => {});
  }
}
