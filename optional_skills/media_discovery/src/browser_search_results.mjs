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
    // The current web component host has a zero-sized box while its open
    // shadow tree paints the visible fixed banner.
    "tiktok-cookie-banner",
  ].join(", ")).first();
  if (!await surface.count()) return true;

  // Accept TikTok's banner so the persisted platform profile records the
  // choice and the fixed consent surface cannot cover captured media. Prefer
  // stable structural identifiers; the exact accessible names cover the
  // current web component, which does not expose the older data attributes.
  const selectors = [
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
  if (!action) {
    const namedAccept = surface.getByRole("button", {
      name: /^(?:accept|allow)(?: all| all cookies| cookies)?$|^(?:全部接受|接受全部|全部同意|同意全部|允许全部)$/iu,
    }).first();
    if (await namedAccept.count() && await namedAccept.isVisible()) action = namedAccept;
  }
  if (!action && await surface.evaluate(element => element.localName === "tiktok-cookie-banner")) {
    // TikTok can mount an inert custom-element shell after the persisted
    // consent choice has already been applied. Its host and shadow root have
    // no rendered controls, so treating the shell itself as an unresolved
    // banner incorrectly blocks otherwise usable search results.
    const consentAlreadyStored = (await page.context().cookies()).some(cookie => (
      cookie.name === "cookie-consent"
      && Boolean(cookie.value)
      && /(?:^|\.)tiktok\.com$/u.test(cookie.domain)
    ));
    const renderedShadowControl = await surface.evaluate(element => {
      const root = element.shadowRoot;
      if (!root) return false;
      return [...root.querySelectorAll("button, [role='button'], a, input")].some(node => {
        const style = getComputedStyle(node);
        return style.visibility !== "hidden" && style.display !== "none"
          && node.getClientRects().length > 0;
      });
    });
    if (consentAlreadyStored && !renderedShadowControl) return true;
  }
  if (!action) return false;
  // TikTok's consent handler starts follow-up network work that can keep
  // Playwright's implicit navigation wait open even after the choice has been
  // applied. The action disappearing is the reliable completion signal.
  await action.click({ timeout: timeoutMs, noWaitAfter: true });
  return action.waitFor({ state: "hidden", timeout: timeoutMs }).then(() => true, () => false);
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

async function clickSearchResult(link, page, settle, timeoutMs, allowObstructionFallback = false) {
  const actionTimeoutMs = allowObstructionFallback ? Math.min(timeoutMs, 1_500) : timeoutMs;
  try {
    // Use the browser's normal input path. Scrolling the exact validated link
    // into view, hovering it, and pausing before the click keeps navigation
    // deliberate without synthesizing fingerprints or bypassing site gates.
    await link.scrollIntoViewIfNeeded({ timeout: actionTimeoutMs });
    await link.hover({ timeout: actionTimeoutMs });
    await settle(page);
    await link.click({ timeout: actionTimeoutMs, noWaitAfter: true });
    return true;
  } catch (error) {
    if (allowObstructionFallback && error?.name === "TimeoutError") return false;
    throw error;
  }
}

export async function withSearchResult(page, platform, itemUrl, source, collect, {
  shouldStop = async () => false, checkAccess = async () => null, settle = async () => {}, timeoutMs = 45_000,
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
    if (platform === "toutiao") {
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
    const clicked = await clickSearchResult(
      links.nth(index), page, settle, timeoutMs, platform === "tiktok",
    );
    // TikTok occasionally paints a transparent interaction layer over an
    // otherwise valid result anchor. Prefer the real click above; only when the
    // browser proves it is obstructed, open the already validated same-origin
    // identity. Normal access/rate-limit checks still run on the detail page.
    if (!clicked) {
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
