const { chromium } = require('playwright');
const fs = require('fs/promises');
const path = require('path');

const PAGE_TIMEOUT_MS = 45000;
const SEARCH_SETTLE_MS = 2000;
const EXTRACT_SETTLE_MS = 1200;
const EXCERPT_LIMIT = 500;
const VALID_WAIT_UNTIL = new Set(['domcontentloaded', 'load', 'networkidle']);

function normalizeWhitespace(text) {
    return (text || '').replace(/\s+/g, ' ').trim();
}

function buildExcerpt(text, limit = EXCERPT_LIMIT) {
    const normalized = normalizeWhitespace(text);
    if (normalized.length <= limit) {
        return normalized;
    }
    return normalized.slice(0, limit).trimEnd() + '...';
}

function parseDomain(rawUrl) {
    try {
        return new URL(rawUrl).hostname;
    } catch {
        return 'browser_web';
    }
}

function sanitizeForFilename(raw) {
    const base = (raw || 'page').toString().toLowerCase();
    return base.replace(/[^a-z0-9._-]+/g, '_').replace(/^_+|_+$/g, '').slice(0, 80) || 'page';
}

function buildScreenshotPath(screenshotDir, url, index) {
    const host = sanitizeForFilename(parseDomain(url));
    const ts = Date.now();
    return path.join(screenshotDir, `bw_${host}_${index}_${ts}.png`);
}

function compactErrorMessage(err) {
    const message = (err && err.message ? err.message : String(err || '')).trim();
    if (!message) {
        return 'unknown browser error';
    }
    const first = message.split('\n').find((line) => line.trim().length > 0);
    return first ? first.trim() : message;
}

function sanitizeExtractedText(raw) {
    const text = (raw || '').replace(/\r/g, '\n');
    const lines = text
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line.length > 0);

    const filtered = lines.filter((line) => {
        if (line.length > 2000 && !line.includes(' ')) return false;
        if (/^\s*[.#][\w-]+\s*\{/.test(line)) return false;
        if (/^(const|let|var|function)\s+[\w$]+/.test(line)) return false;
        if (/=>\s*\{?/.test(line) && line.length < 140) return false;
        if (/^\s*@media\b/.test(line)) return false;
        if (/^\s*<\/?(script|style)/i.test(line)) return false;
        if (/;\s*$/.test(line) && /[{}()=]/.test(line) && line.length < 220) return false;
        return true;
    });

    return normalizeWhitespace(filtered.join('\n'));
}

function buildWaitOrder(preferred) {
    const order = [];
    const push = (v) => {
        if (!v || !VALID_WAIT_UNTIL.has(v)) return;
        if (!order.includes(v)) order.push(v);
    };
    push(preferred);
    push('domcontentloaded');
    push('load');
    push('networkidle');
    return order.length > 0 ? order : ['domcontentloaded', 'load'];
}

async function navigateWithFallback(page, url, preferredWaitUntil) {
    const attempts = buildWaitOrder(preferredWaitUntil);
    let lastError = null;

    for (const waitUntil of attempts) {
        try {
            await page.goto(url, { waitUntil, timeout: PAGE_TIMEOUT_MS });
            return { ok: true, waitUntil, attempts: attempts.length };
        } catch (err) {
            lastError = err;
        }
    }

    return {
        ok: false,
        waitUntil: null,
        attempts: attempts.length,
        error: lastError,
    };
}

async function main() {
    let inputData = '';
    process.stdin.setEncoding('utf8');

    for await (const chunk of process.stdin) {
        inputData += chunk;
    }

    if (!inputData.trim()) {
        process.stderr.write('Error: No input data received\n');
        process.exit(1);
    }

    let input;
    try {
        input = JSON.parse(inputData.trim());
    } catch (e) {
        process.stderr.write(`Error: Failed to parse input JSON: ${e.message}\n`);
        process.exit(1);
    }

    const { action } = input;

    try {
        let result;
        switch (action) {
            case 'openExtract':
                result = await openExtract(input);
                break;
            case 'searchPage':
                result = await searchPage(input);
                break;
            case 'searchExtract':
                result = await searchExtract(input);
                break;
            default:
                throw new Error(`Unknown action: ${action}`);
        }

        process.stdout.write(JSON.stringify(result) + '\n');
    } catch (error) {
        process.stderr.write(`Error: ${error.message}\n`);
        process.exit(1);
    }
}

async function createBrowserContext() {
    const browser = await chromium.launch({
        headless: true,
        args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage'],
    });

    const context = await browser.newContext({
        userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36',
        viewport: { width: 1440, height: 1024 },
        locale: 'en-US',
    });

    context.setDefaultNavigationTimeout(PAGE_TIMEOUT_MS);
    context.setDefaultTimeout(PAGE_TIMEOUT_MS);
    return { browser, context };
}

async function extractPage(page, url, waitUntil, options = {}) {
    const nav = await navigateWithFallback(page, url, waitUntil);

    if (!nav.ok) {
        throw new Error(`navigation failed after ${nav.attempts} attempts: ${compactErrorMessage(nav.error)}`);
    }

    await page.waitForTimeout(EXTRACT_SETTLE_MS);

    const extracted = await page.evaluate(() => {
        const title = document.title || '';
        const clone = document.body ? document.body.cloneNode(true) : null;
        if (clone) {
            clone.querySelectorAll('script, style, noscript, svg').forEach((el) => el.remove());
        }
        const bodyText = clone ? (clone.innerText || '') : '';
        return {
            title,
            text: bodyText,
        };
    });

    const finalUrl = page.url();
    const text = sanitizeExtractedText(extracted.text);

    let screenshotPath = null;
    if (options.saveScreenshot && options.screenshotPath) {
        await fs.mkdir(path.dirname(options.screenshotPath), { recursive: true });
        await page.screenshot({ path: options.screenshotPath, fullPage: true });
        screenshotPath = options.screenshotPath;
    }

    return {
        url,
        final_url: finalUrl,
        title: normalizeWhitespace(extracted.title),
        text,
        content_excerpt: buildExcerpt(text),
        source: parseDomain(finalUrl || url),
        published_at: null,
        fetch_method: 'browser',
        nav_wait_until: nav.waitUntil,
        screenshot_path: screenshotPath,
    };
}

async function openExtract(input) {
    const { urls, waitUntil = 'domcontentloaded', saveScreenshot = true, screenshotDir = 'image/browser_web' } = input;

    if (!urls || !Array.isArray(urls) || urls.length === 0) {
        throw new Error('urls array is required and must not be empty');
    }

    const { browser, context } = await createBrowserContext();
    const items = [];

    try {
        for (const url of urls) {
            const page = await context.newPage();
            try {
                const screenshotPath = buildScreenshotPath(screenshotDir, url, items.length + 1);
                const item = await extractPage(page, url, waitUntil, {
                    saveScreenshot,
                    screenshotPath,
                });
                items.push(item);
            } catch (error) {
                items.push({
                    url,
                    final_url: url,
                    title: '',
                    text: '',
                    content_excerpt: '',
                    source: parseDomain(url),
                    published_at: null,
                    fetch_method: 'unavailable',
                    error: compactErrorMessage(error),
                });
            } finally {
                await page.close().catch(() => {});
            }
        }
    } finally {
        await browser.close().catch(() => {});
    }

    const successCount = items.filter((item) => item.fetch_method === 'browser').length;
    const failureCount = items.length - successCount;
    const citations = items.filter((item) => item.final_url || item.url).map((item) => item.final_url || item.url);
    const summary = failureCount > 0
        ? `Extracted ${successCount} page(s); ${failureCount} page(s) failed browser extraction`
        : `Extracted ${successCount} page(s) using browser`;

    return {
        items,
        summary,
        citations,
    };
}

async function searchPage(input) {
    const { query, engine = 'google', topK = 5 } = input;

    if (!query || typeof query !== 'string' || query.trim() === '') {
        throw new Error('query is required and must be a non-empty string');
    }

    if (engine !== 'google') {
        throw new Error(`Unsupported engine: ${engine}; only 'google' is supported`);
    }

    const { browser, context } = await createBrowserContext();
    const page = await context.newPage();
    const searchUrl = `https://www.google.com/search?hl=en&q=${encodeURIComponent(query)}`;

    try {
        const nav = await navigateWithFallback(page, searchUrl, 'domcontentloaded');
        if (!nav.ok) {
            throw new Error(`google search page navigation failed: ${compactErrorMessage(nav.error)}`);
        }

        await page.waitForTimeout(SEARCH_SETTLE_MS);

        const items = await page.evaluate((k) => {
            const out = [];
            const seen = new Set();
            const anchors = Array.from(document.querySelectorAll('a[href]'));

            for (const anchor of anchors) {
                if (out.length >= k) break;
                const h3 = anchor.querySelector('h3');
                const href = anchor.getAttribute('href');
                if (!h3 || !href) continue;

                let resolved = null;
                try {
                    const url = new URL(href, 'https://www.google.com');
                    if (url.hostname === 'www.google.com' && url.pathname === '/url') {
                        resolved = url.searchParams.get('q') || url.searchParams.get('url');
                    } else if (url.protocol === 'http:' || url.protocol === 'https:') {
                        resolved = url.toString();
                    }
                } catch {
                    resolved = null;
                }

                if (!resolved || seen.has(resolved)) continue;
                if (!/^https?:\/\//i.test(resolved)) continue;

                const block = anchor.closest('div[data-snc], div.g, div[data-hveid], div.MjjYud') || anchor.parentElement;
                const snippetNode = block
                    ? block.querySelector('div[data-sncf="1"], div.VwiC3b, span.aCOpRe, div.yXK7lf, div[style*="-webkit-line-clamp"]')
                    : null;
                const title = (h3.innerText || '').trim();
                if (!title) continue;

                out.push({
                    title,
                    url: resolved,
                    snippet: snippetNode ? sanitizeExtractedText(snippetNode.innerText || '') : '',
                    source: 'google',
                });
                seen.add(resolved);
            }

            return out;
        }, topK);

        const citations = items.map((item) => item.url);
        const summary = items.length > 0
            ? `Found ${items.length} search result(s) for "${query}"`
            : `No search results extracted from Google for "${query}"`;

        return {
            items,
            summary,
            citations,
        };
    } finally {
        await browser.close().catch(() => {});
    }
}

async function searchExtract(input) {
    const { query, engine = 'google', topK = 5, extractTopN = 3, waitUntil = 'domcontentloaded' } = input;

    const searchResult = await searchPage({ query, engine, topK });
    if (!searchResult.items || searchResult.items.length === 0) {
        return {
            items: [],
            summary: `No search results found for "${query}"`,
            citations: [],
        };
    }

    const urlsToExtract = searchResult.items.slice(0, extractTopN).map((item) => item.url);
    const extractResult = await openExtract({ urls: urlsToExtract, waitUntil });

    return {
        items: extractResult.items,
        summary: `Searched Google for "${query}" and extracted ${extractResult.items.length} page(s)`,
        citations: extractResult.citations,
    };
}

main().catch((error) => {
    process.stderr.write(`Fatal error: ${error.message}\n`);
    process.exit(1);
});
