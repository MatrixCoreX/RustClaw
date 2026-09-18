import { browserStageError } from "./browser_diagnostics.mjs";
import { stopsCollection } from "./browser_flow_control.mjs";
import { boundedBrowserOperation } from "./browser_search.mjs";
import { collectOrderedPages } from "./collection_progress.mjs";
import { isDetailUrl } from "./platforms.mjs";

export const KUAISHOU_LOGIN_MODAL = ".login-popup .login-modal:visible, .login-modal.login-modal-v2:visible";
export const KUAISHOU_LOAD_MORE_LOGIN = ".video-list .loading-more .login-link:visible";
export const KUAISHOU_CARD_CLICK_SELECTORS = Object.freeze([".cover:visible", "img.cover-img:visible"]);

export async function kuaishouLoginModalVisible(page) {
  return await page.locator(KUAISHOU_LOGIN_MODAL).count() > 0;
}

export async function kuaishouLoadMoreLoginVisible(page) {
  return await page.locator(KUAISHOU_LOAD_MORE_LOGIN).count() > 0;
}

export async function kuaishouAccessError(page) {
  if (await kuaishouLoginModalVisible(page)) return "login_required";
  if (await kuaishouLoadMoreLoginVisible(page)
    && await page.locator(".video-list .photo-card:visible").count() === 0) {
    return "login_required";
  }
  return null;
}

export async function revealKuaishouLoginSurface(page) {
  if (await kuaishouLoginModalVisible(page)) return true;
  const login = page.locator(KUAISHOU_LOAD_MORE_LOGIN).first();
  if (!await login.isVisible().catch(() => false)) return false;
  await login.click({ timeout: 5000 }).catch(() => {});
  const deadline = Date.now() + 4000;
  while (Date.now() < deadline) {
    if (await kuaishouLoginModalVisible(page)) return true;
    await page.waitForTimeout(200);
  }
  return false;
}

export async function kuaishouSearchCards(page) {
  return boundedBrowserOperation(page.evaluate(() => {
    const key = raw => {
      try { const url = new URL(raw); return ["http:", "https:"].includes(url.protocol) ? url.pathname : null; }
      catch { return null; }
    };
    // Only associate already-rendered covers with the page's loaded public posts.
    // No cache-key decoding, private API calls, or credential fields are needed.
    const photos = Object.values(window.INIT_STATE || {})
      .flatMap(entry => Array.isArray(entry?.feeds) ? entry.feeds : [])
      .map(entry => entry?.photo).filter(photo => /^[A-Za-z0-9_-]{8,64}$/.test(photo?.id || ""));
    return [...document.querySelectorAll(".video-list .photo-card")].flatMap((card, index) => {
      const rect = card.getBoundingClientRect(), css = getComputedStyle(card);
      if (rect.width < 140 || rect.height < 120 || rect.bottom <= 0 || rect.top >= innerHeight
        || css.visibility === "hidden" || css.display === "none") return [];
      const cover = card.querySelector("img.cover-img");
      const coverKey = key(cover?.currentSrc || cover?.src);
      if (!coverKey) return [];
      const ids = new Set(photos.filter(photo => [photo.coverUrl, photo.overrideCoverUrl].some(url => key(url) === coverKey)).map(photo => photo.id));
      if (ids.size !== 1) return [];
      return [{ index, itemId: [...ids][0] }];
    });
  }), 10_000, "search_result_identity");
}

export async function screenshotKuaishouSearchTile(card) {
  const image = card.locator("img.cover-img:visible").first();
  const target = await image.count() ? image : card;
  return target.screenshot({ type: "png", timeout: 10_000 });
}

export async function clickKuaishouSearchCard(card) {
  for (const selector of KUAISHOU_CARD_CLICK_SELECTORS) {
    const target = card.locator(selector).first();
    if (!await target.isVisible().catch(() => false)) continue;
    try {
      await target.click({ timeout: 3_000 });
      return selector;
    } catch {
      // A leftover cover wrapper can be visible but not the live click target.
    }
  }
  await card.click({ timeout: 15_000 });
  return "card";
}

export async function waitForKuaishouSearchOverlay(page, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  const slide = page.locator(".swiper-feed:visible .swiper-slide-active").first();
  while (Date.now() < deadline && !page.isClosed()) {
    if (isDetailUrl("kuaishou", page.url())) return null;
    if (await slide.isVisible().catch(() => false)) return slide;
    const marked = await page.evaluate(() => {
      document.querySelectorAll("[data-discovery-kuaishou-overlay]").forEach(node => {
        node.removeAttribute("data-discovery-kuaishou-overlay");
      });
      const media = [...document.querySelectorAll("video")].filter(node => {
        if (node.closest(".video-list .photo-card")) return false;
        const rect = node.getBoundingClientRect();
        const style = getComputedStyle(node);
        return rect.width >= 140 && rect.height >= 120
          && style.visibility !== "hidden" && style.display !== "none";
      }).sort((left, right) => {
        const a = left.getBoundingClientRect();
        const b = right.getBoundingClientRect();
        return (b.width * b.height) - (a.width * a.height);
      })[0];
      if (!media) return false;
      const host = media.closest(".swiper-feed, .swiper-slide-active") || media.parentElement;
      if (!host) return false;
      host.setAttribute("data-discovery-kuaishou-overlay", "1");
      return true;
    }).catch(() => false);
    if (marked && await page.locator("[data-discovery-kuaishou-overlay='1']").count()) {
      return page.locator("[data-discovery-kuaishou-overlay='1']").first();
    }
    await page.waitForTimeout(150).catch(() => {});
  }
  throw browserStageError("media_element_not_found", "search_detail_open");
}

async function closeKuaishouSearchOverlay(page, scope) {
  const close = scope
    ? scope.locator(".close.circle-btn, .close").first()
    : page.locator(".swiper-feed:visible .close.circle-btn, .swiper-feed:visible .close").first();
  if (await close.isVisible().catch(() => false)) {
    await close.click({ timeout: 5000 }).catch(() => {});
  } else {
    await page.keyboard.press("Escape").catch(() => {});
  }
  await page.locator(".swiper-feed:visible").waitFor({ state: "hidden", timeout: 5000 }).catch(() => {});
  await page.evaluate(() => {
    document.querySelectorAll("[data-discovery-kuaishou-overlay]").forEach(node => {
      node.removeAttribute("data-discovery-kuaishou-overlay");
    });
  }).catch(() => {});
}

function remainingKuaishouCards(cards, seen, completed) {
  return cards.some(card => !seen.has(card.itemId) && !completed.has(`kuaishou:${card.itemId}`));
}

async function kuaishouSearchBlockedByLogin(page, seen, completed) {
  if (!await kuaishouLoadMoreLoginVisible(page) && !await kuaishouLoginModalVisible(page)) return false;
  return !remainingKuaishouCards(await kuaishouSearchCards(page), seen, completed);
}

async function raiseKuaishouLoginBarrier(page, checkAccess, stage, seen, completed) {
  await revealKuaishouLoginSurface(page);
  const blocked = await checkAccess(page);
  if (blocked) throw browserStageError(blocked, stage);
  if (await kuaishouSearchBlockedByLogin(page, seen, completed)) {
    throw browserStageError("login_required", stage);
  }
}

export async function collectKuaishouSearchResults({ page, config, limit, shouldStop,
  checkAccess, settle, scrollPage, collect, onPage, onFailure,
  completed = new Set(), detailed = false }) {
  const seen = new Set();
  let lastError;
  const outcome = await collectOrderedPages({
    readCandidates: async () => {
      const cards = await kuaishouSearchCards(page);
      if (!remainingKuaishouCards(cards, seen, completed)
        && await kuaishouSearchBlockedByLogin(page, seen, completed)) {
        await raiseKuaishouLoginBarrier(page, checkAccess, "search_results_access", seen, completed);
      }
      const access = await checkAccess(page);
      if (access) throw browserStageError(access, "search_results_access");
      return kuaishouSearchCards(page);
    },
    identity: candidate => `kuaishou:${candidate.itemId}`,
    completed, limit, shouldStop, maxScrolls: config.max_scrolls_per_source,
    collect: async candidate => {
      seen.add(candidate.itemId);
      let scope;
      try {
        const current = (await kuaishouSearchCards(page)).find(card => card.itemId === candidate.itemId);
        if (!current) throw browserStageError("source_unavailable", "search_detail_open");
        await settle(page);
        const card = page.locator(".video-list .photo-card").nth(current.index);
        const fallbackCoverPng = await screenshotKuaishouSearchTile(card).catch(() => null);
        await clickKuaishouSearchCard(card);
        scope = await waitForKuaishouSearchOverlay(page);
        const access = await checkAccess(page);
        if (access) throw browserStageError(access, "search_detail_access");
        const url = `https://www.kuaishou.com/short-video/${candidate.itemId}`;
        const result = await collect(page, url, scope, { fallbackCoverPng });
        await onPage(result);
      } catch (error) {
        lastError = error;
        throw error;
      } finally {
        const blocked = await checkAccess(page);
        if (!blocked) await closeKuaishouSearchOverlay(page, scope);
      }
    },
    scroll: async () => {
      await scrollPage(page);
      if (await kuaishouSearchBlockedByLogin(page, seen, completed)) {
        await raiseKuaishouLoginBarrier(page, checkAccess, "search_more_results", seen, completed);
      }
    },
    onFailure, stopsCollection,
  });
  if (!detailed && outcome.handled === 0 && !await shouldStop()) {
    throw lastError || browserStageError("selector_drift", "search_result_identity");
  }
  return detailed ? outcome : outcome.handled;
}
