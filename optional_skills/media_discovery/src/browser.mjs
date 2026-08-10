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
    default_mode: "visible",
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
  return { ...metadata, canonical: canonicalUrl };
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
        browser_mode: config.browser_mode || "visible",
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

async function visibleVideoIndex(page) {
  const candidates = await page.locator("video").evaluateAll((videos) =>
    videos.map((video, index) => {
      const rect = video.getBoundingClientRect();
      return { index, area: rect.width * rect.height, visible: rect.width >= 180 && rect.height >= 120 };
    }),
  );
  return candidates.filter((candidate) => candidate.visible).sort((left, right) => right.area - left.area)[0]?.index;
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

async function renderedVideoCover(scope) {
  const visibleVideo = scope.locator("video:visible").first();
  if ((await visibleVideo.count()) > 0) return visibleVideo;
  const visibleImage = scope.locator("img:visible").first();
  if ((await visibleImage.count()) > 0) return visibleImage;
  return scope;
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
  const temporaryRoot = path.join(root, "tmp", runId);
  const recognitionMode = config.recognition_mode || "ocr_reviewed";
  if (metadata.hasVideo) {
    const screenshotPath = path.join(temporaryRoot, `${itemId.replaceAll(":", "_")}-video.png`);
    const videoIndex = await visibleVideoIndex(page);
    if (Number.isInteger(videoIndex)) {
      const video = page.locator("video").nth(videoIndex);
      await freezeVideoIfPresent(video);
      await screenshotLocator(video, screenshotPath);
    } else {
      const covers = await visibleImageCandidates(page, 1);
      if (covers.length === 0) throw new Error("media_element_not_found");
      await screenshotLocator(page.locator("img").nth(covers[0].index), screenshotPath);
    }
    const recognition = await recognizeScreenshot(screenshotPath, recognitionMode);
    const coverScreenshotPath = await persistVideoCover(root, platform, itemId, screenshotPath);
    return {
      records: [{
        kind: "video",
        dedup_key: `${itemId}:video`,
        platform,
        browser_mode: config.browser_mode || "visible",
        source_mode: discoverySource.source_mode,
        search_keyword: discoverySource.search_keyword || "",
        discovery_source_url: discoverySource.url,
        item_id: itemId,
        title: metadata.title,
        platform_text: metadata.description,
        recognized_text: recognition.text,
        raw_recognized_text: recognition.raw_text,
        recognition,
        cover_screenshot_path: coverScreenshotPath,
        video_page_url: metadata.canonical,
        discovered_at: discoveredAt,
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
    platformText: metadata.description,
    sourcePageUrl: metadata.canonical,
    discoverySource,
    config,
    discoveredAt,
  });
}

async function collectDouyinFeedCard(page, root, runId, locator, itemId, config, discoverySource) {
  const card = await locator.evaluate((node) => ({
    title:
      node.getAttribute("aria-label") ||
      node.querySelector("img")?.getAttribute("alt") ||
      "",
    platformText: node.innerText || "",
  }));
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
      platformText: card.platformText,
      sourcePageUrl: pageUrl,
      discoverySource,
      config,
      discoveredAt: new Date().toISOString(),
    });
  }
  const screenshotPath = path.join(root, "tmp", runId, `douyin_${itemId}-video.png`);
  const cover = await renderedVideoCover(locator);
  await freezeVideoIfPresent(cover);
  await screenshotLocator(cover, screenshotPath);
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
      browser_mode: config.browser_mode || "visible",
      source_mode: discoverySource.source_mode,
      search_keyword: discoverySource.search_keyword || "",
      discovery_source_url: discoverySource.url,
      item_id: `douyin:${itemId}`,
      title: card.title,
      platform_text: card.platformText,
      recognized_text: recognition.text,
      raw_recognized_text: recognition.raw_text,
      recognition,
      cover_screenshot_path: coverScreenshotPath,
      video_page_url: pageUrl,
      discovered_at: new Date().toISOString(),
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

export async function collectPlatform({ root, runId, platform, config, limit, shouldStop, onPage, onFailure }) {
  const browserMode = config.browser_mode || "visible";
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
      const candidateBudget = Math.min(100, Math.max(limit * 3, limit + 5));
      const candidates = await discoverCandidates(
        page,
        platform,
        sourceUrl,
        config.max_scrolls_per_source || 10,
        candidateBudget,
        shouldStop,
        config,
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
