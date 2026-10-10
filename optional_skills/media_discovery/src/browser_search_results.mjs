import { browserStageError } from "./browser_diagnostics.mjs";
import { stopsCollection } from "./browser_flow_control.mjs";
import { assertNavigationResponse, boundedBrowserOperation } from "./browser_search.mjs";
import {
  canonicalPlatformResultUrl,
  isDetailUrl,
  matchesVerificationTarget,
  platformItemId,
  validatePlatformUrl,
} from "./platforms.mjs";

export async function dismissYouTubeConsent(page, timeoutMs = 5_000) {
  // The custom-element host itself has a zero-sized box on the live site even
  // while its child dialog intercepts the page, so visibility must be checked
  // on the actionable descendants rather than on the host.
  const lightbox = page.locator("ytd-consent-bump-v2-lightbox#lightbox").first();
  if (!await lightbox.count()) return true;
  // YouTube renders the privacy-minimizing Reject option first and Accept
  // second as the two filled monochrome actions. Avoid language matching so
  // this remains stable across locales; do not choose when the structure is
  // incomplete or ambiguous.
  const choices = lightbox.locator('button[class*="Filled"][class*="Mono"]:visible');
  let choiceCount = await choices.count();
  if (choiceCount === 0) {
    const activeSurface = lightbox.locator(".body:visible, .eom-buttons:visible");
    if (!await activeSurface.count()) return true;
    await choices.first().waitFor({ state: "visible", timeout: Math.min(timeoutMs, 1_000) }).catch(() => {});
    choiceCount = await choices.count();
  }
  if (choiceCount < 2) return false;
  await choices.first().click({ timeout: timeoutMs });
  return choices.first().waitFor({ state: "hidden", timeout: timeoutMs }).then(() => true, () => false);
}

export async function dismissTikTokConsent(page, timeoutMs = 5_000) {
  const surface = page.locator([
    "#onetrust-banner-sdk:visible",
    '[data-e2e="cookie-banner"]:visible',
    '[data-testid="cookie-banner"]:visible',
  ].join(", ")).first();
  if (!await surface.count()) return true;

  // Prefer the privacy-minimizing action when TikTok's consent manager exposes
  // an explicit structural identifier. Fall back to the explicit accept action
  // only when rejecting optional cookies is unavailable. Never choose by button
  // order or localized prose.
  const selectors = [
    "#onetrust-reject-all-handler:visible",
    '[data-e2e="cookie-banner-reject"]:visible',
    '[data-testid="cookie-banner-reject"]:visible',
    "#onetrust-accept-btn-handler:visible",
    '[data-e2e="cookie-banner-accept"]:visible',
    '[data-testid="cookie-banner-accept"]:visible',
  ];
  let action = null;
  for (const selector of selectors) {
    const candidate = page.locator(selector).first();
    if (await candidate.count() && await candidate.isVisible()) {
      action = candidate;
      break;
    }
  }
  if (!action) return false;
  await action.click({ timeout: timeoutMs });
  return surface.waitFor({ state: "hidden", timeout: timeoutMs }).then(() => true, () => false);
}

export function searchResultLinkIndex(platform, hrefs, itemUrl) {
  const expected = platformItemId(platform, itemUrl);
  return hrefs.findIndex(href => {
    const candidate = canonicalPlatformResultUrl(platform, href);
    return candidate && isDetailUrl(platform, candidate)
      && platformItemId(platform, candidate) === expected;
  });
}

async function navigateToutiaoDetail(page, itemUrl, timeoutMs, stage) {
  try {
    return await boundedBrowserOperation(
      page.goto(itemUrl, { waitUntil: "domcontentloaded", timeout: timeoutMs }),
      timeoutMs,
      stage,
    );
  } catch (error) {
    // The desktop site may replace its initial document with the same item plus
    // a `wid` query. Chromium reports the superseded request as ERR_ABORTED even
    // though the exact requested item is loading normally.
    if (!String(error?.message || "").includes("net::ERR_ABORTED")) throw error;
    const deadline = Date.now() + timeoutMs;
    const expected = platformItemId("toutiao", itemUrl);
    while (Date.now() < deadline) {
      try {
        if (isDetailUrl("toutiao", page.url())
          && platformItemId("toutiao", page.url()) === expected) return null;
      } catch { /* Observe the next committed document. */ }
      await page.waitForTimeout(100);
    }
    throw error;
  }
}

async function navigateStableDetail(page, platform, itemUrl, timeoutMs, stage) {
  if (platform === "toutiao") return navigateToutiaoDetail(page, itemUrl, timeoutMs, stage);
  return boundedBrowserOperation(
    page.goto(itemUrl, { waitUntil: "domcontentloaded", timeout: timeoutMs }),
    timeoutMs,
    stage,
  );
}

export async function withSearchResult(page, platform, itemUrl, source, collect, {
  shouldStop = async () => false, checkAccess = async () => null, timeoutMs = 45_000,
} = {}) {
  const stage = "search_detail_open";
  if (platform === "youtube" && !await dismissYouTubeConsent(page)) {
    throw browserStageError("challenge_required", stage);
  }
  if (platform === "tiktok" && !await dismissTikTokConsent(page)) {
    throw browserStageError("challenge_required", stage);
  }
  const links = page.locator("a[href]:visible");
  const hrefs = await links.evaluateAll(nodes => nodes.map(node => node.href));
  const index = searchResultLinkIndex(platform, hrefs, itemUrl);
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
    // TikTok search cards are often covered by transparent interaction layers:
    // their anchors are structurally visible and have a stable canonical href,
    // but Playwright's actionability check can wait until timeout. Once the
    // exact same-origin detail identity has been validated above, navigate to
    // that URL directly. This does not bypass an access barrier; the detail
    // response and rendered page still pass the normal checks below.
    if (platform === "toutiao" || platform === "tiktok") {
      const response = await navigateStableDetail(page, platform, itemUrl, timeoutMs, stage);
      if (response) assertNavigationResponse(response, stage);
      const access = await checkAccess(page);
      if (access) throw browserStageError(access, stage);
      if (!isDetailUrl(platform, page.url())
        || platformItemId(platform, page.url()) !== platformItemId(platform, itemUrl)) {
        throw browserStageError("search_detail_unavailable", stage);
      }
      return await collect(page);
    }
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
    const blocked = stopsCollection(failure);
    if (!blocked && !page.isClosed()) {
      for (const popup of opened) await popup.close().catch(() => {});
      if (!matchesVerificationTarget(platform, source.url, page.url())) {
        try {
          const response = platform === "toutiao" || platform === "tiktok"
            ? await page.goto(source.url, { waitUntil: "domcontentloaded", timeout: timeoutMs })
            : await page.goBack({ waitUntil: "domcontentloaded", timeout: timeoutMs });
          assertNavigationResponse(response, "search_results_restore");
          if (!matchesVerificationTarget(platform, source.url, page.url())) throw new Error("search_results_restore_failed");
        } catch (error) {
          if (stopsCollection(error)) throw error;
          throw browserStageError("search_results_restore_failed", "search_results_restore");
        }
      }
    }
  }
}
