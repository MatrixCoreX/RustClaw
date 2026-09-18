import { browserStageError } from "./browser_diagnostics.mjs";
import { stopsCollection } from "./browser_flow_control.mjs";
import { boundedBrowserOperation } from "./browser_search.mjs";
import { collectOrderedPages } from "./collection_progress.mjs";
import { douyinModalItemId } from "./platforms.mjs";

const CARD_SELECTOR = ".search-result-card";
const MIN_CARD_WIDTH = 120;
const MIN_CARD_HEIGHT = 120;

export function douyinCoverIdentity(rawUrl, fallback) {
  if (typeof rawUrl === "string" && rawUrl) {
    try {
      const url = new URL(rawUrl, "https://www.douyin.com/");
      if (["http:", "https:"].includes(url.protocol)) return `${url.origin}${url.pathname}`;
    } catch {
      // Fall through to the viewport-stable card token.
    }
  }
  return fallback;
}

export function douyinSearchCardCandidates(cards) {
  return (cards || []).flatMap((card) => {
    if (!card?.hasVideoImage || card.hidden) return [];
    if (card.width < MIN_CARD_WIDTH || card.height < MIN_CARD_HEIGHT) return [];
    if (card.bottom <= 0 || card.top >= card.viewportHeight) return [];
    const fallback = `card:${card.index}:${Math.round(card.width)}x${Math.round(card.height)}:${Math.round(card.left)}:${Math.round(card.top)}`;
    return [{ index: card.index, coverKey: douyinCoverIdentity(card.coverUrl, fallback) }];
  });
}

export async function douyinSearchCards(page) {
  const cards = await boundedBrowserOperation(page.evaluate(() => {
    const coverUrl = (card) => {
      const image = card.querySelector("img");
      if (image?.currentSrc || image?.src) return image.currentSrc || image.src;
      const video = card.querySelector(".videoImage");
      if (!video) return "";
      const match = getComputedStyle(video).backgroundImage.match(/url\(["']?([^"')]+)["']?\)/u);
      return match?.[1] || "";
    };
    return [...document.querySelectorAll(".search-result-card")].map((card, index) => {
      const rect = card.getBoundingClientRect();
      const css = getComputedStyle(card);
      return {
        index,
        width: rect.width,
        height: rect.height,
        top: rect.top,
        bottom: rect.bottom,
        left: rect.left,
        viewportHeight: innerHeight,
        hasVideoImage: Boolean(card.querySelector(".videoImage")),
        coverUrl: coverUrl(card),
        hidden: css.visibility === "hidden" || css.display === "none",
      };
    });
  }), 10_000, "search_result_identity");
  return douyinSearchCardCandidates(cards);
}

async function waitForDouyinSearchModal(page, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline && !page.isClosed()) {
    const itemId = douyinModalItemId(page.url());
    if (itemId) return itemId;
    await page.waitForTimeout(150).catch(() => {});
  }
  throw browserStageError("source_unavailable", "search_detail_open");
}

export async function waitForDouyinSearchOverlay(page, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  const known = page.locator("#overlay:visible, [data-e2e=\"video-detail\"]:visible, #sliderVideo:visible")
    .filter({ has: page.locator("video:visible, canvas:visible, [data-e2e=\"video-desc\"]:visible") });
  while (Date.now() < deadline && !page.isClosed()) {
    if (await known.count()) return known.first();
    const marked = await page.evaluate(() => {
      document.querySelectorAll("[data-discovery-overlay]").forEach((node) => {
        node.removeAttribute("data-discovery-overlay");
      });
      const media = [...document.querySelectorAll("video, canvas")].filter((node) => {
        const rect = node.getBoundingClientRect();
        const style = getComputedStyle(node);
        return rect.width >= 180 && rect.height >= 120
          && style.visibility !== "hidden"
          && style.display !== "none"
          && !node.closest(".search-result-card");
      }).sort((left, right) => {
        const a = left.getBoundingClientRect();
        const b = right.getBoundingClientRect();
        return (b.width * b.height) - (a.width * a.height);
      })[0];
      if (!media) return false;
      let node = media.parentElement;
      while (node && node !== document.body) {
        const rect = node.getBoundingClientRect();
        const style = getComputedStyle(node);
        const covers = rect.width >= innerWidth * 0.4 && rect.height >= innerHeight * 0.4;
        const positioned = ["fixed", "absolute", "sticky"].includes(style.position);
        if (node.id === "overlay" || node.getAttribute("data-e2e") === "video-detail" || node.id === "sliderVideo"
          || covers || (positioned && rect.width >= 240 && rect.height >= 180)) {
          node.setAttribute("data-discovery-overlay", "1");
          return true;
        }
        node = node.parentElement;
      }
      media.parentElement?.setAttribute("data-discovery-overlay", "1");
      return Boolean(media.parentElement);
    }).catch(() => false);
    if (marked && await page.locator("[data-discovery-overlay=\"1\"]").count()) {
      return page.locator("[data-discovery-overlay=\"1\"]").first();
    }
    await page.waitForTimeout(150).catch(() => {});
  }
  throw browserStageError("media_element_not_found", "search_detail_open");
}

async function closeDouyinSearchModal(page) {
  await page.evaluate(() => {
    document.querySelectorAll("[data-discovery-overlay]").forEach((node) => {
      node.removeAttribute("data-discovery-overlay");
    });
  }).catch(() => {});
  if (!douyinModalItemId(page.url())) return;
  await page.keyboard.press("Escape").catch(() => {});
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (!douyinModalItemId(page.url())) return;
    await page.waitForTimeout(100).catch(() => {});
  }
  await page.goBack({ waitUntil: "domcontentloaded", timeout: 15_000 }).catch(() => {});
}

export async function collectDouyinSearchResults({ page, config, limit, shouldStop,
  checkAccess, settle, scrollPage, collect, onPage, onFailure,
  completed = new Set(), detailed = false }) {
  let lastError;
  const outcome = await collectOrderedPages({
    readCandidates: async () => {
      const access = await checkAccess(page);
      if (access) throw browserStageError(access, "search_results_access");
      return douyinSearchCards(page);
    },
    identity: (candidate) => candidate.coverKey,
    completed, limit, shouldStop, maxScrolls: config.max_scrolls_per_source,
    collect: async (candidate) => {
      try {
        const current = (await douyinSearchCards(page)).find((card) => card.coverKey === candidate.coverKey);
        if (!current) throw browserStageError("source_unavailable", "search_detail_open");
        await settle(page);
        const tile = page.locator(CARD_SELECTOR).nth(current.index).locator(".videoImage").first();
        const fallbackCoverPng = await tile.screenshot({ type: "png", timeout: 10_000 });
        await tile.click({ timeout: 15_000 });
        const itemId = await waitForDouyinSearchModal(page);
        const access = await checkAccess(page);
        if (access) throw browserStageError(access, "search_detail_access");
        if (completed.has(`douyin:${itemId}`)) return { skipped: true };
        await page.locator("video:visible").first().waitFor({ state: "visible", timeout: 15_000 }).catch(() => {});
        const overlay = await waitForDouyinSearchOverlay(page);
        const result = await collect(page, `https://www.douyin.com/video/${itemId}`, overlay, { fallbackCoverPng });
        await onPage(result);
        return undefined;
      } catch (error) {
        lastError = error;
        throw error;
      } finally {
        const blocked = await checkAccess(page);
        if (!blocked) await closeDouyinSearchModal(page);
      }
    },
    scroll: () => scrollPage(page),
    onFailure, stopsCollection,
  });
  if (!detailed && outcome.handled === 0 && !await shouldStop()) {
    throw lastError || browserStageError("selector_drift", "search_result_identity");
  }
  return detailed ? outcome : outcome.handled;
}
