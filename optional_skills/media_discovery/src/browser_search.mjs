import { assertDocumentResponse, browserStageError } from "./browser_diagnostics.mjs";
import { navigationAccessError } from "./browser_flow_control.mjs";
import { canonicalCandidateUrls, matchesVerificationTarget, platformSpec, validatePlatformUrl } from "./platforms.mjs";

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
  douyin: '[data-aweme-id]:visible, a[href*="/video/"]:visible, a[href*="/note/"]:visible, .search-result-card:visible',
  xiaohongshu: 'section.note-item:visible, a[href*="/explore/"]:visible, a[href*="/search_result/"]:visible',
  kuaishou: '.video-card:visible, .video-list .photo-card:visible, a[href*="/short-video/"]:visible',
  toutiao: 'a[href*="/article/"]:visible, a[href*="/video/"]:visible, a[href*="/w/"]:visible, a[href*="/search/jump"]:visible',
});

async function searchResultCount(page, platform, timeoutMs, stage) {
  if (!["toutiao", "weibo", "tiktok", "youtube"].includes(platform)) {
    return boundedBrowserOperation(page.locator(RESULT_SELECTORS[platform]).count(), timeoutMs, stage);
  }
  const urls = await boundedBrowserOperation(page.locator("a[href]").evaluateAll(nodes =>
    nodes.filter(node => {
      const rect = node.getBoundingClientRect(), style = getComputedStyle(node);
      return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden"
        && style.display !== "none" && Number(style.opacity) !== 0;
    }).map(node => node.href),
  ), timeoutMs, stage);
  return canonicalCandidateUrls(platform, urls).length;
}

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
  const accessError = navigationAccessError(response, stage);
  if (accessError) throw accessError;
  assertDocumentResponse(response, stage);
}

export async function navigateDocument(page, url, stage, timeoutMs = 45_000, waitUntil = "domcontentloaded") {
  const response = await boundedBrowserOperation(
    page.goto(url, { waitUntil, timeout: timeoutMs }), timeoutMs, stage,
  );
  assertNavigationResponse(response, stage);
  return response;
}

function typedTimeout(error) {
  return error?.name === "TimeoutError" || error?.message === "browser_timeout";
}

async function searchTargetPage(page, opened, platform, targetUrl) {
  return [page, ...opened]
    .filter(candidate => !candidate.isClosed())
    .find(candidate => matchesVerificationTarget(platform, targetUrl, candidate.url())) || null;
}

async function navigateSearchUrlFallback(page, source, timeoutMs) {
  await navigateDocument(page, source.url, "search_submit_fallback", timeoutMs, "commit");
  return "platform_search_url_fallback";
}

async function submitKeywordSearch(page, opened, controls, platform, source, timeoutMs) {
  if (platform !== "douyin") {
    await controls.submit.click({ timeout: timeoutMs, noWaitAfter: true });
    return "platform_search_form";
  }

  // Douyin frequently leaves a transparent recommendation layer over the
  // search button on low-powered displays. Enter is the site's normal input
  // action and avoids waiting for a permanently obstructed pointer target.
  try {
    await controls.input.press("Enter", {
      timeout: Math.max(250, Math.min(2_500, Math.floor(timeoutMs / 4))),
      noWaitAfter: true,
    });
  } catch (error) {
    if (!typedTimeout(error)) throw error;
  }
  const enterDeadline = Date.now() + Math.max(250, Math.min(1_500, Math.floor(timeoutMs / 5)));
  while (Date.now() < enterDeadline) {
    if (await searchTargetPage(page, opened, platform, source.url)) return "platform_search_form";
    await page.waitForTimeout(100);
  }

  try {
    await controls.submit.click({
      timeout: Math.max(250, Math.min(2_500, Math.floor(timeoutMs / 4))),
      noWaitAfter: true,
    });
    return "platform_search_form";
  } catch (error) {
    if (!typedTimeout(error)) throw error;
    // A low-powered browser may keep streaming the search shell long after the
    // response and DOM are usable. Commit proves that the first-party document
    // was accepted; the result loop below validates its type and content.
    return navigateSearchUrlFallback(page, source, timeoutMs);
  }
}

export async function openKeywordSearch(page, platform, source, {
  shouldStop = async () => false, checkAccess = async () => null, timeoutMs = 45_000,
  settle = candidate => candidate.waitForTimeout(750),
  prepareHome = null,
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
    const platformConfiguration = platformSpec(platform);
    if (platformConfiguration.topicNavigation === "direct") {
      stage = "search_results_ready";
      const response = await navigateDocument(page, source.url, stage, timeoutMs);
      assertNavigationResponse(response, stage);
      await settle(page);
      await check(page);
      const resultDeadline = Date.now() + timeoutMs;
      let count = 0;
      while (Date.now() < resultDeadline && count === 0) {
        await check(page);
        count = await searchResultCount(page, platform, resultDeadline - Date.now(), stage);
        if (count === 0) await page.waitForTimeout(200);
      }
      if (count === 0) throw browserStageError("search_results_unavailable", stage);
      return {
        page,
        source: { ...source, url: validatePlatformUrl(platform, page.url()) },
        method: "platform_search_url",
      };
    }
    await navigateDocument(page, platformConfiguration.homeUrl, stage, timeoutMs);
    if (prepareHome) {
      await prepareHome(page);
      await check(page);
    }
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
    const inputMatches = await controls.input.inputValue({ timeout: timeoutMs }) === source.search_keyword;
    if (!inputMatches && platform !== "douyin") {
      throw browserStageError("search_input_mismatch", stage);
    }
    responses.clear();
    let submissionMethod;
    if (inputMatches) {
      // A textarea may insert a newline on Enter. Submit the site's search control.
      await check(page);
      await settle(page);
      submissionMethod = await submitKeywordSearch(
        page, opened, controls, platform, source, timeoutMs,
      );
    } else {
      submissionMethod = await navigateSearchUrlFallback(page, source, timeoutMs);
    }
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
        const onSearchTarget = matchesVerificationTarget(platform, source.url, observedUrl);
        // Record verified navigation before later DOM/access observations. A
        // late platform challenge or an empty hydrated shell must not be
        // misclassified as an unsubmitted search.
        if (onSearchTarget) matched = true;
        if (onSearchTarget) {
          assertNavigationResponse(responses.get(destination), stage);
          const contentType = await boundedBrowserOperation(destination.evaluate(() => document.contentType),
            resultDeadline - Date.now(), stage);
          assertDocumentResponse({ headers: () => ({ "content-type": contentType }) }, stage);
        }
        await check(destination);
        if (onSearchTarget) {
          const count = await searchResultCount(destination, platform,
            resultDeadline - Date.now(), stage);
          if (count > 0) return {
            page: destination,
            source: { ...source, url: validatePlatformUrl(platform, destination.url()) },
            method: submissionMethod,
          };
        }
      } catch (error) {
        // During submit/navigation the DOM context can disappear between reads.
        // Retry observations within the deadline, never resubmit the search.
        if (destination.isClosed()) throw browserStageError("interactive_verification_cancelled", stage);
        if (error.discovery_stage || error.name !== "Error") throw error;
      }
      await page.waitForTimeout(200);
    }
    await check(destination);
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
