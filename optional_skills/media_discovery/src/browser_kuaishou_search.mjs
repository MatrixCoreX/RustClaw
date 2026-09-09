import { browserStageError } from "./browser_diagnostics.mjs";
import { boundedBrowserOperation } from "./browser_search.mjs";

export async function kuaishouSearchCards(page) {
  return boundedBrowserOperation(page.evaluate(() => {
    const key = raw => {
      try { const url = new URL(raw); return ["http:", "https:"].includes(url.protocol) ? url.pathname : null; }
      catch { return null; }
    };
    // Only associate already-rendered covers with the page's loaded public posts.
    // No cache-key decoding, private API calls, or credential fields are needed.
    const photos = Object.values(window.INIT_STATE || {}).slice(0, 200)
      .flatMap(entry => Array.isArray(entry?.feeds) ? entry.feeds.slice(0, 1000) : [])
      .map(entry => entry?.photo).filter(photo => /^[A-Za-z0-9_-]{8,64}$/.test(photo?.id || ""));
    return [...document.querySelectorAll(".video-list .photo-card")].slice(0, 500).flatMap((card, index) => {
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

export async function collectKuaishouSearchResults({ page, config, limit, shouldStop,
  checkAccess, settle, scrollPage, collect, onPage, onFailure }) {
  const seen = new Set();
  let handled = 0;
  let lastError;
  for (let scroll = 0; scroll <= (config.max_scrolls_per_source || 10) && handled < limit; scroll += 1) {
    const access = await checkAccess(page);
    if (access) throw browserStageError(access, "search_results_access");
    for (const candidate of await kuaishouSearchCards(page)) {
      if (handled >= limit || await shouldStop()) break;
      if (seen.has(candidate.itemId)) continue;
      seen.add(candidate.itemId);
      let scope;
      try {
        const current = (await kuaishouSearchCards(page)).find(card => card.itemId === candidate.itemId);
        if (!current) throw browserStageError("source_unavailable", "search_detail_open");
        await settle(page);
        await page.locator(".video-list .photo-card").nth(current.index).locator(".cover").click({ timeout: 15_000 });
        scope = page.locator(".swiper-feed:visible .swiper-slide-active").first();
        await scope.waitFor({ state: "visible", timeout: 15_000 });
        const access = await checkAccess(page);
        if (access) throw browserStageError(access, "search_detail_access");
        const url = `https://www.kuaishou.com/short-video/${candidate.itemId}`;
        const result = await collect(page, url, scope);
        await onPage(result);
        handled += 1;
      } catch (error) {
        lastError = error;
        await onFailure?.(error);
        if (["login_required", "challenge_required", "network_access_restricted", "rate_limited", "collection_stopped"].includes(error.message)) throw error;
      } finally {
        const blocked = await checkAccess(page);
        if (!blocked && scope && await scope.isVisible()) {
          await scope.locator(".close.circle-btn").click({ timeout: 5000 });
          await page.locator(".swiper-feed:visible").waitFor({ state: "hidden", timeout: 5000 });
        }
      }
    }
    if (handled >= limit || await shouldStop() || scroll === (config.max_scrolls_per_source || 10)) break;
    await scrollPage(page);
    const login = page.locator(".video-list .loading-more .login-link:visible").first();
    if (await login.isVisible() && !(await kuaishouSearchCards(page)).some(card => !seen.has(card.itemId))) {
      throw browserStageError("login_required", "search_more_results");
    }
  }
  if (handled === 0 && !await shouldStop()) throw lastError || browserStageError("selector_drift", "search_result_identity");
  return handled;
}
