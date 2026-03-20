const fs = require('fs/promises');
const fsSync = require('fs');
const path = require('path');
const crypto = require('crypto');

const PAGE_TIMEOUT_MS = 45000;
const SEARCH_SETTLE_MS = 2000;
const EXTRACT_SETTLE_MS = 1200;
const EXCERPT_LIMIT = 500;
const VALID_WAIT_UNTIL = new Set(['domcontentloaded', 'load', 'networkidle']);
const DEFAULT_MIN_CONTENT_CHARS = 200;
const DEFAULT_MAX_TEXT_CHARS = 12000;
const DEFAULT_WAIT_MAP_PATH = path.join(process.cwd(), 'configs', 'browser_web_wait_map.json');
const DEFAULT_SYSTEM_CHROMIUM_PATHS = ['/usr/bin/chromium', '/usr/bin/chromium-browser'];
const DEFAULT_CAPTURE_ROOT = path.join(process.cwd(), 'data', 'web_capture');
const DEFAULT_CHUNK_CHARS = 3000;
const DEFAULT_MAX_CAPTURE_IMAGES = 12;
const DEFAULT_IMAGE_FETCH_TIMEOUT_MS = 15000;
const DEFAULT_IMAGE_FETCH_MAX_BYTES = 6 * 1024 * 1024;

class SkillError extends Error {
    constructor(code, message, meta = null) {
        super(message);
        this.name = 'SkillError';
        this.code = code;
        this.meta = meta;
    }
}

let playwright = null;

function normalizeWhitespace(text) {
    return (text || '').replace(/\s+/g, ' ').trim();
}

function buildExcerpt(text, limit = EXCERPT_LIMIT) {
    const normalized = normalizeWhitespace(text);
    if (normalized.length <= limit) {
        return normalized;
    }
    return `${normalized.slice(0, limit).trimEnd()}...`;
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

function guessExtFromContentType(contentType) {
    const ct = (contentType || '').toLowerCase();
    if (ct.includes('image/jpeg')) return '.jpg';
    if (ct.includes('image/png')) return '.png';
    if (ct.includes('image/webp')) return '.webp';
    if (ct.includes('image/gif')) return '.gif';
    if (ct.includes('image/svg+xml')) return '.svg';
    if (ct.includes('image/avif')) return '.avif';
    return '.img';
}

function toIsoDate(ts = new Date()) {
    return ts.toISOString().slice(0, 10);
}

function createRunId(ts = new Date()) {
    const compact = ts.toISOString().replace(/[-:.TZ]/g, '');
    return `run_${compact}_${Math.random().toString(36).slice(2, 8)}`;
}

function ensureWithinRoot(root, target) {
    const absRoot = path.resolve(root);
    const absTarget = path.resolve(target);
    if (absTarget !== absRoot && !absTarget.startsWith(`${absRoot}${path.sep}`)) {
        throw new SkillError('EMPTY_CONTENT', `capture path escapes root: ${target}`);
    }
    return absTarget;
}

function toPosixRel(base, target) {
    return path.relative(base, target).split(path.sep).join('/');
}

function normalizeSourceTag(raw) {
    return sanitizeForFilename(raw || 'default_source');
}

async function appendJsonl(filePath, obj) {
    await fs.appendFile(filePath, `${JSON.stringify(obj)}\n`, 'utf8');
}

function sha256Hex(text) {
    return crypto.createHash('sha256').update(text || '', 'utf8').digest('hex');
}

function chunkTextByChars(text, maxChars) {
    const clean = normalizeWhitespace(text || '');
    if (!clean) return [];
    const limit = Math.max(500, Number(maxChars) || DEFAULT_CHUNK_CHARS);
    const chunks = [];
    for (let i = 0; i < clean.length; i += limit) {
        chunks.push(clean.slice(i, i + limit));
    }
    return chunks;
}

async function downloadImageToFile(url, outputPath) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), DEFAULT_IMAGE_FETCH_TIMEOUT_MS);
    try {
        const response = await fetch(url, {
            method: 'GET',
            signal: controller.signal,
            headers: {
                'user-agent': 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36',
                'accept': 'image/*,*/*;q=0.8',
            },
        });
        if (!response.ok) {
            throw new Error(`http ${response.status}`);
        }
        const contentType = response.headers.get('content-type') || '';
        if (!contentType.toLowerCase().includes('image/')) {
            throw new Error(`not image content-type: ${contentType || 'unknown'}`);
        }
        const arrayBuffer = await response.arrayBuffer();
        if (arrayBuffer.byteLength > DEFAULT_IMAGE_FETCH_MAX_BYTES) {
            throw new Error(`image too large: ${arrayBuffer.byteLength} bytes`);
        }
        const ext = guessExtFromContentType(contentType);
        const finalPath = outputPath.endsWith(ext) ? outputPath : `${outputPath}${ext}`;
        await fs.writeFile(finalPath, Buffer.from(arrayBuffer));
        return { ok: true, path: finalPath, bytes: arrayBuffer.byteLength, contentType };
    } catch (err) {
        return { ok: false, error: compactErrorMessage(err) };
    } finally {
        clearTimeout(timer);
    }
}

async function initCaptureStorage(options = {}) {
    const enabled = options.enableCapture !== false;
    if (!enabled) {
        return { enabled: false };
    }

    const source = normalizeSourceTag(options.captureSource || options.source || 'browser_web');
    const now = new Date();
    const date = toIsoDate(now);
    const runId = sanitizeForFilename(options.runId || createRunId(now));
    const root = path.resolve(options.captureRoot || DEFAULT_CAPTURE_ROOT);
    const runRoot = ensureWithinRoot(root, path.join(root, source, date, runId));

    const dirs = {
        runRoot,
        rawHtml: path.join(runRoot, 'raw', 'html'),
        processedText: path.join(runRoot, 'processed', 'text'),
        images: path.join(runRoot, 'assets', 'images'),
        meta: path.join(runRoot, 'meta'),
        index: path.join(runRoot, 'index'),
    };

    await Promise.all([
        fs.mkdir(dirs.rawHtml, { recursive: true }),
        fs.mkdir(dirs.processedText, { recursive: true }),
        fs.mkdir(dirs.images, { recursive: true }),
        fs.mkdir(dirs.meta, { recursive: true }),
        fs.mkdir(dirs.index, { recursive: true }),
    ]);

    const files = {
        manifest: path.join(dirs.meta, 'manifest.jsonl'),
        chunks: path.join(dirs.index, 'chunks.jsonl'),
    };
    await Promise.all([
        fs.writeFile(files.manifest, '', { flag: 'a' }),
        fs.writeFile(files.chunks, '', { flag: 'a' }),
    ]);

    return {
        enabled: true,
        root,
        source,
        date,
        runId,
        runRoot,
        dirs,
        files,
    };
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

function classifyError(err) {
    if (err instanceof SkillError) {
        return err;
    }
    const msg = compactErrorMessage(err);
    const lower = msg.toLowerCase();
    if (lower.includes('timeout')) {
        return new SkillError('NAV_TIMEOUT', msg);
    }
    if (lower.includes('playwright') || lower.includes('chromium') || lower.includes('node.js')) {
        return new SkillError('DEPENDENCY_MISSING', msg);
    }
    if (lower.includes('operation not permitted') || lower.includes('seccomp')) {
        return new SkillError('DEPENDENCY_MISSING', msg);
    }
    if (lower.includes('selector')) {
        return new SkillError('SELECTOR_MISS', msg);
    }
    if (lower.includes('captcha') || lower.includes('access denied') || lower.includes('forbidden')) {
        return new SkillError('BOT_BLOCKED', msg);
    }
    return new SkillError('EMPTY_CONTENT', msg);
}

function detectBotBlock(title, text) {
    const joined = `${title || ''}\n${text || ''}`.toLowerCase();
    const patterns = [
        'captcha',
        'are you a robot',
        'access denied',
        'forbidden',
        'verify you are human',
        'unusual traffic',
        'security check',
        'temporarily blocked',
    ];
    return patterns.some((p) => joined.includes(p));
}

function truncateText(text, maxChars) {
    if (!text || text.length <= maxChars) {
        return text || '';
    }
    return text.slice(0, maxChars);
}

function chooseWaitUntilForUrl(url, preferredWaitUntil, waitMap) {
    if (!waitMap || !waitMap.domains || typeof waitMap.domains !== 'object') {
        return preferredWaitUntil;
    }
    let host = '';
    try {
        host = new URL(url).hostname.toLowerCase();
    } catch {
        return preferredWaitUntil;
    }

    const direct = waitMap.domains[host];
    if (typeof direct === 'string' && VALID_WAIT_UNTIL.has(direct)) {
        return direct;
    }

    for (const [k, v] of Object.entries(waitMap.domains)) {
        if (!k.startsWith('*.') || typeof v !== 'string') {
            continue;
        }
        const suffix = k.slice(1).toLowerCase();
        if (host.endsWith(suffix) && VALID_WAIT_UNTIL.has(v)) {
            return v;
        }
    }

    return preferredWaitUntil;
}

async function loadWaitMap(customPath) {
    const candidates = [];
    if (customPath && typeof customPath === 'string' && customPath.trim() !== '') {
        candidates.push(path.isAbsolute(customPath) ? customPath : path.join(process.cwd(), customPath.trim()));
    }
    candidates.push(DEFAULT_WAIT_MAP_PATH);

    for (const candidate of candidates) {
        if (!fsSync.existsSync(candidate)) {
            continue;
        }
        try {
            const raw = await fs.readFile(candidate, 'utf8');
            const parsed = JSON.parse(raw);
            if (parsed && typeof parsed === 'object') {
                return { map: parsed, path: candidate };
            }
        } catch {
            continue;
        }
    }

    return { map: { domains: {} }, path: null };
}

async function navigateWithFallback(page, url, preferredWaitUntil) {
    const attempts = buildWaitOrder(preferredWaitUntil);
    const trace = [];
    let lastError = null;

    for (const waitUntil of attempts) {
        const started = Date.now();
        try {
            await page.goto(url, { waitUntil, timeout: PAGE_TIMEOUT_MS });
            trace.push({
                wait_until: waitUntil,
                status: 'ok',
                latency_ms: Date.now() - started,
            });
            return { ok: true, waitUntil, attempts: trace.length, trace };
        } catch (err) {
            lastError = err;
            trace.push({
                wait_until: waitUntil,
                status: 'error',
                latency_ms: Date.now() - started,
                error: compactErrorMessage(err),
            });
        }
    }

    return {
        ok: false,
        waitUntil: null,
        attempts: trace.length,
        trace,
        error: lastError,
    };
}

async function createBrowserContext() {
    if (!playwright) {
        try {
            playwright = require('playwright');
        } catch (err) {
            throw new SkillError(
                'DEPENDENCY_MISSING',
                `Playwright dependency missing: ${compactErrorMessage(err)}; run 'npm install' in crates/skills/browser_web`
            );
        }
    }

    const runtimeCheck = await readRuntimeRestrictionSignals();
    const executablePath = chooseChromiumExecutablePath();

    const launchEnv = { ...process.env };
    delete launchEnv.DISPLAY;
    delete launchEnv.WAYLAND_DISPLAY;
    delete launchEnv.XAUTHORITY;

    const browser = await playwright.chromium.launch({
        executablePath,
        headless: true,
        env: launchEnv,
        args: [
            '--headless=new',
            '--no-sandbox',
            '--disable-setuid-sandbox',
            '--disable-dev-shm-usage',
            '--disable-gpu',
        ],
    });

    const context = await browser.newContext({
        userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36',
        viewport: { width: 1440, height: 1024 },
        locale: 'en-US',
    });

    context.setDefaultNavigationTimeout(PAGE_TIMEOUT_MS);
    context.setDefaultTimeout(PAGE_TIMEOUT_MS);
    return { browser, context, runtimeCheck, executablePath };
}

function chooseChromiumExecutablePath() {
    for (const p of DEFAULT_SYSTEM_CHROMIUM_PATHS) {
        if (fsSync.existsSync(p)) {
            return p;
        }
    }
    return null;
}

async function readRuntimeRestrictionSignals() {
    try {
        const content = await fs.readFile('/proc/self/status', 'utf8');
        const noNewPrivs = /NoNewPrivs:\s*(\d+)/.exec(content);
        const seccomp = /Seccomp:\s*(\d+)/.exec(content);
        const noNewPrivsValue = noNewPrivs ? Number(noNewPrivs[1]) : null;
        const seccompValue = seccomp ? Number(seccomp[1]) : null;
        const restricted = noNewPrivsValue === 1 && seccompValue === 2;
        return {
            no_new_privs: noNewPrivsValue,
            seccomp: seccompValue,
            restricted,
        };
    } catch {
        return {
            no_new_privs: null,
            seccomp: null,
            restricted: false,
        };
    }
}

async function extractPage(page, url, waitUntil, options = {}) {
    const pageStart = Date.now();
    const nav = await navigateWithFallback(page, url, waitUntil);
    if (!nav.ok) {
        throw new SkillError('NAV_TIMEOUT', `navigation failed after ${nav.attempts} attempts: ${compactErrorMessage(nav.error)}`, {
            nav_trace: nav.trace,
        });
    }

    await page.waitForTimeout(EXTRACT_SETTLE_MS);
    const rawHtml = await page.content();

    const extracted = await page.evaluate(() => {
        const title = document.title || '';
        const selectors = [
            'article',
            'main',
            '[role="main"]',
            '.post-content',
            '.article-content',
            '.entry-content',
            '.content',
            '.markdown-body',
            '.prose',
        ];
        const norm = (txt) => (txt || '').replace(/\s+/g, ' ').trim();

        const removeNoise = (root) => {
            if (!root) return;
            root.querySelectorAll('script, style, noscript, svg, nav, footer, aside, form, button, iframe').forEach((el) => el.remove());
        };

        const collectText = (root) => {
            if (!root) return '';
            const clone = root.cloneNode(true);
            removeNoise(clone);
            return clone.innerText || '';
        };

        const bodyText = collectText(document.body);
        let bestSelector = null;
        let bestText = '';
        const selectorDiagnostics = [];

        for (const selector of selectors) {
            const node = document.querySelector(selector);
            const candidate = collectText(node);
            const chars = norm(candidate).length;
            selectorDiagnostics.push({ selector, chars });
            if (chars > norm(bestText).length) {
                bestSelector = selector;
                bestText = candidate;
            }
        }

        const bodyChars = norm(bodyText).length;
        const bestChars = norm(bestText).length;
        const useMain = bestChars >= Math.max(120, Math.floor(bodyChars * 0.2));

        const imageCandidates = Array.from(document.querySelectorAll('img[src]'))
            .map((img) => {
                let abs = '';
                try {
                    abs = new URL(img.getAttribute('src') || '', document.baseURI).toString();
                } catch {
                    abs = '';
                }
                return {
                    src: abs,
                    alt: (img.getAttribute('alt') || '').trim(),
                    width: img.naturalWidth || img.width || 0,
                    height: img.naturalHeight || img.height || 0,
                };
            })
            .filter((x) => /^https?:\/\//i.test(x.src));

        return {
            title,
            text: useMain ? bestText : bodyText,
            image_candidates: imageCandidates,
            extraction_trace: {
                used_main: useMain,
                best_selector: bestSelector,
                selector_diagnostics: selectorDiagnostics,
                body_chars: bodyChars,
                selected_chars: useMain ? bestChars : bodyChars,
            },
        };
    });

    const finalUrl = page.url();
    const rawText = normalizeWhitespace(extracted.text || '');
    const contentMode = options.contentMode === 'raw' ? 'raw' : 'clean';
    const cleaned = contentMode === 'raw' ? rawText : sanitizeExtractedText(rawText);
    const finalText = truncateText(cleaned, options.maxTextChars || DEFAULT_MAX_TEXT_CHARS);

    if (detectBotBlock(extracted.title, finalText)) {
        throw new SkillError('BOT_BLOCKED', 'page appears blocked by anti-bot challenge', {
            nav_trace: nav.trace,
            final_url: finalUrl,
        });
    }

    const minChars = options.minContentChars || DEFAULT_MIN_CONTENT_CHARS;
    if (!normalizeWhitespace(extracted.title) || finalText.length < minChars) {
        const code = extracted.extraction_trace && extracted.extraction_trace.used_main ? 'EMPTY_CONTENT' : 'SELECTOR_MISS';
        throw new SkillError(code, `page readability below threshold (title/text) with ${finalText.length} chars`, {
            nav_trace: nav.trace,
            extraction_trace: extracted.extraction_trace,
            final_url: finalUrl,
        });
    }

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
        text: finalText,
        content_excerpt: buildExcerpt(finalText),
        source: parseDomain(finalUrl || url),
        published_at: null,
        fetch_method: 'browser',
        extracted_at: new Date().toISOString(),
        nav_wait_until: nav.waitUntil,
        nav_attempts: nav.attempts,
        nav_attempt_trace: nav.trace,
        latency_ms: Date.now() - pageStart,
        extraction_trace: extracted.extraction_trace,
        screenshot_path: screenshotPath,
        raw_html: rawHtml,
        image_candidates: extracted.image_candidates || [],
    };
}

async function openExtract(input) {
    const {
        urls,
        waitUntil = 'domcontentloaded',
        saveScreenshot = false,
        screenshotDir = null,
        contentMode = 'clean',
        maxTextChars = DEFAULT_MAX_TEXT_CHARS,
        failFast = false,
        minContentChars = DEFAULT_MIN_CONTENT_CHARS,
        waitMapPath = null,
        enableCapture = true,
        captureRoot = null,
        captureSource = 'browser_web',
        runId = null,
        chunkChars = DEFAULT_CHUNK_CHARS,
        captureImages = true,
        maxCaptureImages = DEFAULT_MAX_CAPTURE_IMAGES,
    } = input;

    if (!urls || !Array.isArray(urls) || urls.length === 0) {
        throw new SkillError('EMPTY_CONTENT', 'urls array is required and must not be empty');
    }
    if (!['clean', 'raw'].includes(contentMode)) {
        throw new SkillError('EMPTY_CONTENT', 'contentMode must be one of clean|raw');
    }

    const { map: waitMap, path: waitMapResolved } = await loadWaitMap(waitMapPath);
    const capture = await initCaptureStorage({
        enableCapture,
        captureRoot,
        captureSource,
        runId,
        source: 'browser_web',
    });
    const effectiveScreenshotDir = screenshotDir
        ? (path.isAbsolute(screenshotDir) ? screenshotDir : path.join(process.cwd(), screenshotDir))
        : (capture.enabled ? capture.dirs.images : path.join(process.cwd(), 'image', 'browser_web'));
    const { browser, context, runtimeCheck, executablePath } = await createBrowserContext();
    const items = [];

    try {
        for (const url of urls) {
            const page = await context.newPage();
            try {
                const screenshotPath = buildScreenshotPath(effectiveScreenshotDir, url, items.length + 1);
                const effectiveWait = chooseWaitUntilForUrl(url, waitUntil, waitMap);
                const item = await extractPage(page, url, effectiveWait, {
                    saveScreenshot,
                    screenshotPath,
                    contentMode,
                    maxTextChars,
                    minContentChars,
                });
                const pageOrdinal = items.length + 1;
                if (capture.enabled) {
                    const baseName = `${String(pageOrdinal).padStart(4, '0')}_${sanitizeForFilename(parseDomain(item.final_url || url))}`;
                    const htmlPath = path.join(capture.dirs.rawHtml, `${baseName}.html`);
                    const textPath = path.join(capture.dirs.processedText, `${baseName}.txt`);

                    await fs.writeFile(htmlPath, item.raw_html || '', 'utf8');
                    await fs.writeFile(textPath, item.text || '', 'utf8');

                    const contentHash = sha256Hex(item.text || '');
                    const htmlHash = sha256Hex(item.raw_html || '');
                    let imageRel = item.screenshot_path ? toPosixRel(capture.runRoot, item.screenshot_path) : null;
                    const capturedImages = [];
                    const imageCaptureErrors = [];
                    if (captureImages && Array.isArray(item.image_candidates) && item.image_candidates.length > 0) {
                        try {
                            const seen = new Set();
                            const candidates = item.image_candidates.filter((img) => {
                                if (!img || !img.src || seen.has(img.src)) return false;
                                seen.add(img.src);
                                return true;
                            }).slice(0, Math.max(1, Number(maxCaptureImages) || DEFAULT_MAX_CAPTURE_IMAGES));

                            for (let i = 0; i < candidates.length; i += 1) {
                                const img = candidates[i];
                                const stem = path.join(capture.dirs.images, `${baseName}_img_${String(i + 1).padStart(2, '0')}`);
                                const dl = await downloadImageToFile(img.src, stem);
                                if (!dl.ok) {
                                    imageCaptureErrors.push({ url: img.src, error: dl.error });
                                    continue;
                                }
                                capturedImages.push({
                                    url: img.src,
                                    alt: img.alt || '',
                                    width: img.width || 0,
                                    height: img.height || 0,
                                    bytes: dl.bytes,
                                    content_type: dl.contentType,
                                    path: toPosixRel(capture.runRoot, dl.path),
                                });
                            }
                        } catch (imgErr) {
                            imageCaptureErrors.push({ url: null, error: compactErrorMessage(imgErr) });
                        }
                    }

                    await appendJsonl(capture.files.manifest, {
                        run_id: capture.runId,
                        source: capture.source,
                        ordinal: pageOrdinal,
                        status: 'ok',
                        url,
                        final_url: item.final_url || url,
                        title: item.title || '',
                        fetched_at: item.extracted_at,
                        fetch_method: item.fetch_method,
                        text_chars: (item.text || '').length,
                        content_hash_sha256: contentHash,
                        html_hash_sha256: htmlHash,
                        html_path: toPosixRel(capture.runRoot, htmlPath),
                        text_path: toPosixRel(capture.runRoot, textPath),
                        image_path: imageRel,
                        image_paths: capturedImages.map((x) => x.path),
                        image_capture_errors: imageCaptureErrors,
                        error_code: null,
                        error: null,
                    });

                    const chunks = chunkTextByChars(item.text || '', chunkChars);
                    for (let idx = 0; idx < chunks.length; idx += 1) {
                        const chunkText = chunks[idx];
                        await appendJsonl(capture.files.chunks, {
                            run_id: capture.runId,
                            source: capture.source,
                            chunk_id: `${capture.runId}:${pageOrdinal}:${idx + 1}`,
                            ordinal: pageOrdinal,
                            chunk_index: idx + 1,
                            url,
                            final_url: item.final_url || url,
                            title: item.title || '',
                            text: chunkText,
                            text_chars: chunkText.length,
                            content_hash_sha256: contentHash,
                            extracted_at: item.extracted_at,
                        });
                    }

                    item.capture_artifacts = {
                        run_root: capture.runRoot,
                        html_path: htmlPath,
                        text_path: textPath,
                        image_path: item.screenshot_path || null,
                        image_paths: capturedImages.map((x) => path.join(capture.runRoot, x.path)),
                        image_capture_errors: imageCaptureErrors,
                        manifest_path: capture.files.manifest,
                        chunks_path: capture.files.chunks,
                    };
                }
                delete item.raw_html;
                delete item.image_candidates;
                item.wait_strategy = {
                    requested_wait_until: waitUntil,
                    effective_wait_until: effectiveWait,
                    wait_map_path: waitMapResolved,
                };
                item.runtime = {
                    chromium_executable_path: executablePath || 'playwright-default',
                    runtime_restriction_signals: runtimeCheck,
                };
                items.push(item);
            } catch (err) {
                const classified = classifyError(err);
                const meta = err && err.meta && typeof err.meta === 'object' ? err.meta : null;
                const pageOrdinal = items.length + 1;
                items.push({
                    url,
                    final_url: url,
                    title: '',
                    text: '',
                    content_excerpt: '',
                    source: parseDomain(url),
                    published_at: null,
                    fetch_method: 'unavailable',
                    extracted_at: new Date().toISOString(),
                    error_code: classified.code,
                    error: classified.message,
                    diagnostics: meta,
                    runtime: {
                        chromium_executable_path: executablePath || 'playwright-default',
                        runtime_restriction_signals: runtimeCheck,
                    },
                });
                if (capture.enabled) {
                    await appendJsonl(capture.files.manifest, {
                        run_id: capture.runId,
                        source: capture.source,
                        ordinal: pageOrdinal,
                        status: 'error',
                        url,
                        final_url: url,
                        title: '',
                        fetched_at: new Date().toISOString(),
                        fetch_method: 'unavailable',
                        text_chars: 0,
                        content_hash_sha256: null,
                        html_hash_sha256: null,
                        html_path: null,
                        text_path: null,
                        image_path: null,
                        image_paths: [],
                        error_code: classified.code,
                        error: classified.message,
                    });
                }
                if (failFast) {
                    throw classified;
                }
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
        capture: capture.enabled ? {
            run_root: capture.runRoot,
            raw_html_dir: capture.dirs.rawHtml,
            processed_text_dir: capture.dirs.processedText,
            images_dir: capture.dirs.images,
            manifest_path: capture.files.manifest,
            chunks_path: capture.files.chunks,
        } : null,
    };
}

async function searchPage(input) {
    const { query, engine = 'google', topK = 5, region = null, lang = 'en' } = input;

    if (!query || typeof query !== 'string' || query.trim() === '') {
        throw new SkillError('EMPTY_CONTENT', 'query is required and must be a non-empty string');
    }
    if (engine !== 'google') {
        throw new SkillError('SELECTOR_MISS', `Unsupported engine: ${engine}; only 'google' is supported`);
    }

    const { browser, context, runtimeCheck, executablePath } = await createBrowserContext();
    const page = await context.newPage();
    const langParam = typeof lang === 'string' && lang.trim() ? lang.trim() : 'en';
    const regionParam = typeof region === 'string' && region.trim() ? `&gl=${encodeURIComponent(region.trim())}` : '';
    const searchUrl = `https://www.google.com/search?hl=${encodeURIComponent(langParam)}${regionParam}&q=${encodeURIComponent(query)}`;

    try {
        const nav = await navigateWithFallback(page, searchUrl, 'domcontentloaded');
        if (!nav.ok) {
            throw new SkillError('NAV_TIMEOUT', `google search page navigation failed: ${compactErrorMessage(nav.error)}`, {
                nav_trace: nav.trace,
            });
        }

        await page.waitForTimeout(SEARCH_SETTLE_MS);

        const extracted = await page.evaluate((k) => {
            const normalize = (txt) => (txt || '').replace(/\s+/g, ' ').trim();
            const seen = new Set();
            const out = [];
            const strategies = [
                { name: 'legacy_anchor_h3', rootSelector: 'a:has(h3)' },
                { name: 'result_blocks', rootSelector: 'div.g, div.MjjYud, div[data-hveid]' },
                { name: 'generic_anchor', rootSelector: 'a[href]' },
            ];

            const diagnostics = {
                strategy_hits: {},
                selected_strategy: null,
                fallback_used: false,
                total_candidates: 0,
            };

            const resolveHref = (href) => {
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
                if (!resolved || !/^https?:\/\//i.test(resolved)) {
                    return null;
                }
                return resolved;
            };

            for (const strategy of strategies) {
                if (out.length >= k) break;
                const nodes = Array.from(document.querySelectorAll(strategy.rootSelector));
                diagnostics.strategy_hits[strategy.name] = nodes.length;
                diagnostics.total_candidates += nodes.length;

                for (const node of nodes) {
                    if (out.length >= k) break;
                    const anchor = node.matches('a[href]') ? node : node.querySelector('a[href]');
                    if (!anchor) continue;

                    const href = anchor.getAttribute('href');
                    const resolved = href ? resolveHref(href) : null;
                    if (!resolved || seen.has(resolved)) continue;

                    const h3 = anchor.querySelector('h3') || node.querySelector('h3');
                    const title = normalize(h3 ? h3.innerText : anchor.innerText || '');
                    if (!title) continue;

                    const snippetNode = node.querySelector('div[data-sncf="1"], div.VwiC3b, span.aCOpRe, div.yXK7lf, div[style*="-webkit-line-clamp"], div[role="heading"] + div');
                    out.push({
                        title,
                        url: resolved,
                        snippet: normalize(snippetNode ? snippetNode.innerText : ''),
                        source: 'google',
                    });
                    seen.add(resolved);
                }

                if (out.length > 0 && diagnostics.selected_strategy === null) {
                    diagnostics.selected_strategy = strategy.name;
                }
            }

            diagnostics.fallback_used = diagnostics.selected_strategy !== 'legacy_anchor_h3';
            return { items: out, diagnostics };
        }, topK);

        const items = extracted.items || [];
        if (items.length === 0) {
            throw new SkillError('SELECTOR_MISS', `No search results extracted from Google for "${query}"`, {
                search_url: searchUrl,
                selector_diagnostics: extracted.diagnostics,
                nav_trace: nav.trace,
            });
        }

        return {
            items,
            summary: `Found ${items.length} search result(s) for "${query}"`,
            citations: items.map((item) => item.url),
            nav_wait_until: nav.waitUntil,
            nav_attempts: nav.attempts,
            nav_attempt_trace: nav.trace,
            diagnostics: extracted.diagnostics,
            runtime: {
                chromium_executable_path: executablePath || 'playwright-default',
                runtime_restriction_signals: runtimeCheck,
            },
        };
    } finally {
        await browser.close().catch(() => {});
    }
}

function summarizeText(text) {
    const clean = normalizeWhitespace(text || '');
    if (!clean) {
        return '';
    }
    const hardLimit = 280;
    if (clean.length <= hardLimit) {
        return clean;
    }
    const parts = clean.split(/(?<=[.!?])\s+/);
    let out = '';
    for (const sentence of parts) {
        const next = `${out} ${sentence}`.trim();
        if (next.length > hardLimit) {
            break;
        }
        out = next;
    }
    return out || `${clean.slice(0, hardLimit)}...`;
}

async function searchExtract(input) {
    const {
        query,
        engine = 'google',
        topK = 5,
        extractTopN = 3,
        waitUntil = 'domcontentloaded',
        summarize = true,
        contentMode = 'clean',
        maxTextChars = DEFAULT_MAX_TEXT_CHARS,
        failFast = false,
        region = null,
        lang = 'en',
    } = input;

    const searchResult = await searchPage({ query, engine, topK, region, lang });
    const urlsToExtract = searchResult.items.slice(0, extractTopN).map((item) => item.url);
    if (urlsToExtract.length === 0) {
        return {
            items: [],
            summary: `No search results found for "${query}"`,
            citations: [],
            search_diagnostics: searchResult.diagnostics || null,
        };
    }

    const extractResult = await openExtract({
        urls: urlsToExtract,
        waitUntil,
        contentMode,
        maxTextChars,
        failFast,
    });

    if (summarize) {
        extractResult.items = extractResult.items.map((item) => ({
            ...item,
            summary: summarizeText(item.text || item.content_excerpt || ''),
        }));
    }

    return {
        items: extractResult.items,
        summary: `Searched Google for "${query}" and extracted ${extractResult.items.length} page(s)`,
        citations: extractResult.citations,
        search_diagnostics: searchResult.diagnostics || null,
    };
}

async function main() {
    let inputData = '';
    process.stdin.setEncoding('utf8');

    for await (const chunk of process.stdin) {
        inputData += chunk;
    }

    if (!inputData.trim()) {
        process.stderr.write('[EMPTY_CONTENT] No input data received\n');
        process.exit(1);
    }

    let input;
    try {
        input = JSON.parse(inputData.trim());
    } catch (e) {
        process.stderr.write(`[EMPTY_CONTENT] Failed to parse input JSON: ${e.message}\n`);
        process.exit(1);
    }

    const { action } = input;

    try {
        let result;
        switch (action) {
            case 'openExtract':
            case 'open_extract':
                result = await openExtract(input);
                break;
            case 'searchPage':
            case 'search_page':
                result = await searchPage(input);
                break;
            case 'searchExtract':
            case 'search_extract':
                result = await searchExtract(input);
                break;
            default:
                throw new SkillError('EMPTY_CONTENT', `Unknown action: ${action}`);
        }

        process.stdout.write(`${JSON.stringify(result)}\n`);
    } catch (error) {
        const classified = classifyError(error);
        const runtimeCheck = await readRuntimeRestrictionSignals();
        let extraMsg = classified.message;
        let outCode = classified.code;
        if (
            runtimeCheck.restricted
            || extraMsg.includes('Operation not permitted')
        ) {
            extraMsg = `${extraMsg}; runtime restriction detected (NoNewPrivs=${runtimeCheck.no_new_privs}, Seccomp=${runtimeCheck.seccomp})`;
            outCode = 'DEPENDENCY_MISSING';
        }
        process.stderr.write(`[${outCode}] ${extraMsg}\n`);
        if (classified.meta) {
            process.stderr.write(`${JSON.stringify(classified.meta)}\n`);
        }
        process.exit(1);
    }
}

main().catch((error) => {
    const classified = classifyError(error);
    process.stderr.write(`[${classified.code}] ${classified.message}\n`);
    process.exit(1);
});
