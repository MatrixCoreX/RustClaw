import { browserStageError } from "./browser_diagnostics.mjs";
import { assertNavigationResponse, boundedBrowserOperation } from "./browser_search.mjs";
import { isDetailUrl, matchesVerificationTarget, platformItemId, validatePlatformUrl } from "./platforms.mjs";

export async function withSearchResult(page, platform, itemUrl, source, collect, {
  shouldStop = async () => false, checkAccess = async () => null, timeoutMs = 45_000,
} = {}) {
  const stage = "search_detail_open";
  const links = page.locator("a[href]:visible");
  const hrefs = await links.evaluateAll(nodes => nodes.map(node => node.href));
  const index = hrefs.findIndex(href => isDetailUrl(platform, href)
    && platformItemId(platform, href) === platformItemId(platform, itemUrl));
  if (index < 0) throw browserStageError("source_unavailable", stage);
  if (await shouldStop()) throw browserStageError("collection_stopped", stage);
  const opened = [];
  const responses = new Map();
  const listeners = new Map();
  const observe = candidate => {
    const listener = response => {
      if (response.request().isNavigationRequest() && response.frame() === candidate.mainFrame()) responses.set(candidate, response);
    };
    candidate.on("response", listener);
    listeners.set(candidate, listener);
  };
  const onPopup = popup => { opened.push(popup); observe(popup); };
  observe(page);
  page.on("popup", onPopup);
  let destination = page;
  let failure;
  try {
    await links.nth(index).click({ timeout: timeoutMs, noWaitAfter: true });
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (await shouldStop()) throw browserStageError("collection_stopped", stage);
      destination = opened.find(candidate => !candidate.isClosed()) || page;
      if (destination.isClosed()) throw browserStageError("interactive_verification_cancelled", stage);
      const url = destination.url();
      if (url && url !== "about:blank") {
        validatePlatformUrl(platform, url);
        const response = responses.get(destination);
        // A response event can arrive before the page commits its new URL.
        // Do not restore history while that navigation is still in flight.
        if (response && response.url() !== url) {
          await page.waitForTimeout(150);
          continue;
        }
        assertNavigationResponse(response, stage);
        const access = await checkAccess(destination).then(code => ({ code }), error => {
          if (error.name === "Error" && !error.discovery_stage) return null;
          throw error;
        });
        if (!access) {
          await page.waitForTimeout(150);
          continue;
        }
        if (access.code) throw browserStageError(access.code, stage);
        if (isDetailUrl(platform, url) && platformItemId(platform, url) === platformItemId(platform, itemUrl)) {
          await boundedBrowserOperation(destination.waitForLoadState("domcontentloaded", { timeout: deadline - Date.now() }),
            deadline - Date.now(), stage);
          return await collect(destination);
        }
      }
      await page.waitForTimeout(150);
    }
    throw browserStageError("search_detail_unavailable", stage);
  } catch (error) {
    failure = error;
    error.discovery_target_url ||= itemUrl;
    throw error;
  } finally {
    page.off("popup", onPopup);
    for (const [candidate, listener] of listeners) candidate.off("response", listener);
    // Keep access barriers in place for the caller's diagnostic/manual handoff.
    const blocked = ["login_required", "challenge_required", "network_access_restricted", "rate_limited",
      "collection_stopped", "interactive_verification_cancelled"].includes(failure?.message);
    if (!blocked && !page.isClosed()) {
      for (const popup of opened) await popup.close().catch(() => {});
      if (!matchesVerificationTarget(platform, source.url, page.url())) {
        try {
          const response = await page.goBack({ waitUntil: "domcontentloaded", timeout: timeoutMs });
          assertNavigationResponse(response, "search_results_restore");
          if (!matchesVerificationTarget(platform, source.url, page.url())) throw new Error("search_results_restore_failed");
        } catch {
          throw browserStageError("search_results_restore_failed", "search_results_restore");
        }
      }
    }
  }
}
