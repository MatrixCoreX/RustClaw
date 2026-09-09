import { assertDocumentResponse, browserStageError } from "./browser_diagnostics.mjs";
import { matchesVerificationTarget, platformSpec, validatePlatformUrl } from "./platforms.mjs";

const SEARCH_CONTROLS = Object.freeze({
  douyin: [
    { input: 'input[data-e2e="searchbar-input"]', submit: '[data-e2e="searchbar-button"]' },
    { input: 'input[data-e2e="search-input"]', submit: '[data-e2e="search-button"]' },
  ],
  xiaohongshu: [
    { input: '#search-input-in-feeds', submit: '.textarea-container:has(#search-input-in-feeds) .submit-button-wrapper' },
    { input: '#search-input-ai', submit: '.textarea-container:has(#search-input-ai) .submit-button-wrapper' },
    { input: '#search-input', submit: '.input-box:has(#search-input) .search-icon' },
  ],
  kuaishou: [
    { input: '.search-container .search input.input', submit: '.search-container .search .search-text' },
    { input: 'input.search-input', submit: 'button.search-button' },
  ],
});

const RESULT_SELECTORS = Object.freeze({
  douyin: '[data-aweme-id]:visible, a[href*="/video/"]:visible, a[href*="/note/"]:visible',
  xiaohongshu: 'section.note-item:visible, a[href*="/explore/"]:visible, a[href*="/search_result/"]:visible',
  kuaishou: '.video-card:visible, .video-list .photo-card:visible, a[href*="/short-video/"]:visible',
});

export function normalizeBrowserError(error, stage) {
  if (error?.name === "TimeoutError") return browserStageError("browser_timeout", stage);
  return error;
}

export async function boundedBrowserOperation(operation, timeoutMs, stage) {
  let timer;
  try {
    return await Promise.race([
      operation,
      new Promise((_, reject) => { timer = setTimeout(() => reject(browserStageError("browser_timeout", stage)), Math.max(1, timeoutMs)); }),
    ]);
  } catch (error) {
    throw normalizeBrowserError(error, stage);
  } finally {
    clearTimeout(timer);
  }
}

export function assertNavigationResponse(response, stage) {
  if (response && [404, 410].includes(response.status())) throw browserStageError("source_unavailable", stage);
  if (response && [401, 403, 429].includes(response.status())) {
    throw browserStageError(response.status() === 429 ? "rate_limited" : "challenge_required", stage);
  }
  assertDocumentResponse(response, stage);
}

export async function navigateDocument(page, url, stage, timeoutMs = 45_000) {
  const response = await boundedBrowserOperation(
    page.goto(url, { waitUntil: "domcontentloaded", timeout: timeoutMs }), timeoutMs, stage,
  );
  assertNavigationResponse(response, stage);
  return response;
}

export async function openKeywordSearch(page, platform, source, {
  shouldStop = async () => false, checkAccess = async () => null, timeoutMs = 45_000,
  settle = candidate => candidate.waitForTimeout(750),
} = {}) {
  let stage = "search_home_navigation";
  let destination = page;
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
  page.on("popup", onPopup);
  observe(page);
  const check = async candidate => {
    if (await shouldStop()) throw browserStageError("collection_stopped", stage);
    const errorCode = await checkAccess(candidate);
    if (errorCode) throw browserStageError(errorCode, stage);
  };
  try {
    await check(page);
    await navigateDocument(page, platformSpec(platform).homeUrl, stage, timeoutMs);
    stage = "search_input_ready";
    const inputDeadline = Date.now() + timeoutMs;
    let controls;
    while (Date.now() < inputDeadline) {
      await check(page);
      for (const spec of SEARCH_CONTROLS[platform] || []) {
        const input = page.locator(`${spec.input}:visible`).first();
        if (await boundedBrowserOperation(input.isVisible(), inputDeadline - Date.now(), stage)
          && await boundedBrowserOperation(input.isEditable(), inputDeadline - Date.now(), stage)) {
          controls = { input, submit: page.locator(`${spec.submit}:visible`).first() };
          break;
        }
      }
      if (controls) break;
      await page.waitForTimeout(200);
    }
    if (!controls) throw browserStageError("search_control_unavailable", stage);
    stage = "search_submit";
    await settle(page);
    await controls.input.fill(source.search_keyword, { timeout: timeoutMs });
    if (await controls.input.inputValue({ timeout: timeoutMs }) !== source.search_keyword) {
      throw browserStageError("search_input_mismatch", stage);
    }
    // A textarea may insert a newline on Enter. Submit the site's search control.
    await check(page);
    await settle(page);
    responses.clear();
    await controls.submit.click({ timeout: timeoutMs, noWaitAfter: true });
    stage = "search_results_ready";
    const resultDeadline = Date.now() + timeoutMs;
    let matched = false;
    while (Date.now() < resultDeadline) {
      if (await shouldStop()) throw browserStageError("collection_stopped", stage);
      const candidates = [page, ...opened].filter(candidate => !candidate.isClosed());
      destination = candidates.find(candidate => matchesVerificationTarget(platform, source.url, candidate.url()))
        || opened.find(candidate => !candidate.isClosed()) || page;
      if (destination.url() === "about:blank") {
        await page.waitForTimeout(200);
        continue;
      }
      const observedUrl = destination.url();
      try {
        validatePlatformUrl(platform, observedUrl);
        assertNavigationResponse(responses.get(destination), stage);
        const loaded = await destination.waitForLoadState("domcontentloaded", {
          timeout: Math.max(1, Math.min(2000, resultDeadline - Date.now())),
        }).then(() => true, error => {
          if (error.name === "TimeoutError") return false;
          throw error;
        });
        if (!loaded) continue;
        const contentType = await boundedBrowserOperation(destination.evaluate(() => document.contentType),
          resultDeadline - Date.now(), stage);
        assertDocumentResponse({ headers: () => ({ "content-type": contentType }) }, stage);
        await check(destination);
        if (matchesVerificationTarget(platform, source.url, destination.url())) {
          matched = true;
          const count = await boundedBrowserOperation(destination.locator(RESULT_SELECTORS[platform]).count(),
            resultDeadline - Date.now(), stage);
          if (count > 0) return { page: destination, source: { ...source, url: validatePlatformUrl(platform, destination.url()) } };
        }
      } catch (error) {
        // During submit/navigation the DOM context can disappear between reads.
        // Retry observations within the deadline, never resubmit the search.
        if (destination.isClosed()) throw browserStageError("interactive_verification_cancelled", stage);
        if (error.discovery_stage || error.name !== "Error") throw error;
      }
      await page.waitForTimeout(200);
    }
    throw browserStageError(matched ? "search_results_unavailable" : "search_not_submitted", stage);
  } catch (cause) {
    const error = normalizeBrowserError(cause, stage);
    error.discovery_stage ||= stage;
    try { error.discovery_target_url = validatePlatformUrl(platform, destination.url()); } catch { /* Keep the caller's validated source. */ }
    throw error;
  } finally {
    page.off("popup", onPopup);
    for (const [candidate, listener] of listeners) candidate.off("response", listener);
  }
}
