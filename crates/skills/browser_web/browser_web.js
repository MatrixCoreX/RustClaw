const fs = require('fs/promises');
const fsSync = require('fs');
const path = require('path');
const crypto = require('crypto');
const dns = require('dns').promises;
const net = require('net');
const { execFileSync } = require('child_process');

const PAGE_TIMEOUT_MS = 45000;
const EXTRACT_SETTLE_MS = 1200;
const EXCERPT_LIMIT = 500;
const VALID_WAIT_UNTIL = new Set(['domcontentloaded', 'load', 'networkidle']);
const DEFAULT_MIN_CONTENT_CHARS = 200;
const DEFAULT_MAX_TEXT_CHARS = 12000;
const DEFAULT_WAIT_MAP_PATH = path.join(process.cwd(), 'configs', 'browser_web_wait_map.json');
const DEFAULT_CAPTURE_ROOT = path.join(process.cwd(), 'skills_output');
const DEFAULT_SCREENSHOT_ROOT = path.join(process.cwd(), 'skills_output', 'browser_web', 'screenshots');
const DEFAULT_CHUNK_CHARS = 3000;
const DEFAULT_MAX_CAPTURE_IMAGES = 12;
const DEFAULT_IMAGE_FETCH_TIMEOUT_MS = 15000;
const DEFAULT_IMAGE_FETCH_MAX_BYTES = 6 * 1024 * 1024;
const MAX_CAPTURE_HTML_CHARS = 4 * 1024 * 1024;
const MAX_NETWORK_POLICY_EVENTS = 200;
const MAX_NETWORK_REDIRECTS = 5;
const DNS_POLICY_CACHE = new Map();

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

function domainMatches(host, domain) {
    return host === domain || host.endsWith(`.${domain}`);
}

function isPrivateIpv4(ip) {
    const parts = ip.split('.').map(Number);
    if (parts.length !== 4 || parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)) {
        return true;
    }
    const [a, b, c] = parts;
    return a === 0
        || a === 10
        || a === 127
        || (a === 100 && b >= 64 && b <= 127)
        || (a === 169 && b === 254)
        || (a === 172 && b >= 16 && b <= 31)
        || (a === 192 && b === 168)
        || (a === 192 && b === 0 && (c === 0 || c === 2))
        || (a === 198 && (b === 18 || b === 19))
        || (a === 198 && b === 51 && c === 100)
        || (a === 203 && b === 0 && c === 113)
        || a >= 224;
}

function isPrivateIpv6(ip) {
    const value = ip.toLowerCase();
    if (value === '::' || value === '::1') return true;
    if (value.startsWith('fc') || value.startsWith('fd')) return true;
    if (/^fe[89ab]/.test(value)) return true;
    if (value.startsWith('ff')) return true;
    if (value.startsWith('2001:db8:')) return true;
    const mapped = /^::ffff:(\d+\.\d+\.\d+\.\d+)$/.exec(value);
    return mapped ? isPrivateIpv4(mapped[1]) : false;
}

function isPrivateIp(ip) {
    const family = net.isIP(ip);
    if (family === 4) return isPrivateIpv4(ip);
    if (family === 6) return isPrivateIpv6(ip);
    return true;
}

function isProxySyntheticIp(ip) {
    if (net.isIP(ip) !== 4) return false;
    const [a, b] = ip.split('.').map(Number);
    return a === 198 && (b === 18 || b === 19);
}

function firstProxyValue(scheme) {
    const names = scheme === 'https:'
        ? ['HTTPS_PROXY', 'https_proxy', 'ALL_PROXY', 'all_proxy']
        : ['HTTP_PROXY', 'http_proxy', 'ALL_PROXY', 'all_proxy'];
    for (const name of names) {
        const value = (process.env[name] || '').trim();
        if (value) return value;
    }
    return null;
}

function hostMatchesNoProxy(host) {
    const value = (process.env.NO_PROXY || process.env.no_proxy || '').trim();
    if (!value) return false;
    return value.split(',').map((entry) => entry.trim()).some((entry) => {
        if (entry === '*') return true;
        const withoutPort = entry.includes(':') && !entry.startsWith('[')
            ? entry.split(':')[0]
            : entry;
        const domain = withoutPort.replace(/^\./, '').replace(/\.$/, '').toLowerCase();
        return domain !== '' && domainMatches(host, domain);
    });
}

async function validateNetworkUrl(rawUrl, policy = {}, applyDomainPolicy = false) {
    let parsed;
    try {
        parsed = new URL(rawUrl);
    } catch {
        throw new SkillError('URL_INVALID', 'url_invalid');
    }
    if (!['http:', 'https:'].includes(parsed.protocol)) {
        throw new SkillError('URL_SCHEME_BLOCKED', 'url_scheme_blocked');
    }
    if (parsed.username || parsed.password) {
        throw new SkillError('URL_CREDENTIALS_BLOCKED', 'url_credentials_blocked');
    }
    const host = parsed.hostname
        .toLowerCase()
        .replace(/^\[/, '')
        .replace(/\]$/, '')
        .replace(/\.$/, '');
    const domainsAllow = Array.isArray(policy.domainsAllow) ? policy.domainsAllow : [];
    const domainsDeny = Array.isArray(policy.domainsDeny) ? policy.domainsDeny : [];
    if (applyDomainPolicy && domainsDeny.some((domain) => domainMatches(host, domain))) {
        throw new SkillError('DOMAIN_BLOCKED', 'domain_blocked');
    }
    if (
        applyDomainPolicy
        && domainsAllow.length > 0
        && !domainsAllow.some((domain) => domainMatches(host, domain))
    ) {
        throw new SkillError('DOMAIN_NOT_ALLOWED', 'domain_not_allowed');
    }
    if (
        host === 'localhost'
        || host.endsWith('.localhost')
        || host.endsWith('.local')
        || host.endsWith('.internal')
    ) {
        throw new SkillError('PRIVATE_NETWORK_BLOCKED', 'private_network_blocked');
    }

    const literalFamily = net.isIP(host);
    let addresses;
    if (literalFamily) {
        addresses = [host];
    } else if (DNS_POLICY_CACHE.has(host)) {
        addresses = DNS_POLICY_CACHE.get(host);
    } else {
        let resolved;
        try {
            resolved = await dns.lookup(host, { all: true, verbatim: true });
        } catch {
            throw new SkillError('DNS_RESOLUTION_FAILED', 'dns_resolution_failed');
        }
        addresses = Array.from(new Set(resolved.map((entry) => entry.address)));
        DNS_POLICY_CACHE.set(host, addresses);
    }
    const proxyMediated = !literalFamily && Boolean(firstProxyValue(parsed.protocol)) && !hostMatchesNoProxy(host);
    if (
        addresses.length === 0
        || addresses.some((address) => isPrivateIp(address) && !(proxyMediated && isProxySyntheticIp(address)))
    ) {
        throw new SkillError('PRIVATE_NETWORK_BLOCKED', 'private_network_blocked');
    }
    parsed.hash = '';
    return {
        url: parsed.toString(),
        host,
        proxy_mediated: proxyMediated,
        resolved_addresses: addresses,
    };
}

async function installNetworkGuard(context, policy) {
    const events = [];
    await context.route('**/*', async (route) => {
        const request = route.request();
        const url = request.url();
        if (!/^https?:/i.test(url)) {
            await route.continue();
            return;
        }
        try {
            const observation = await validateNetworkUrl(
                url,
                policy,
                request.isNavigationRequest() && request.resourceType() === 'document'
            );
            if (
                events.length < MAX_NETWORK_POLICY_EVENTS
                && (request.isNavigationRequest() || request.resourceType() === 'document')
            ) {
                events.push({
                    decision: 'allow',
                    url: observation.url,
                    host: observation.host,
                    proxy_mediated: observation.proxy_mediated,
                    resource_type: request.resourceType(),
                });
            }
            await route.continue();
        } catch (error) {
            const classified = classifyError(error);
            if (events.length < MAX_NETWORK_POLICY_EVENTS) {
                events.push({
                    decision: 'deny',
                    url,
                    error_code: classified.code,
                    resource_type: request.resourceType(),
                });
            }
            await route.abort('blockedbyclient');
        }
    });
    return events;
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
        throw new SkillError('WORKSPACE_PATH_OUTSIDE', 'workspace_path_outside');
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

async function readResponseBytes(response, maxBytes) {
    const declared = Number(response.headers.get('content-length') || 0);
    if (declared > maxBytes) {
        throw new SkillError('RESPONSE_TOO_LARGE', 'response_too_large');
    }
    if (!response.body || typeof response.body.getReader !== 'function') {
        throw new SkillError('RESPONSE_READ_FAILED', 'response_body_unavailable');
    }
    const reader = response.body.getReader();
    const chunks = [];
    let total = 0;
    while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        total += value.byteLength;
        if (total > maxBytes) {
            await reader.cancel().catch(() => {});
            throw new SkillError('RESPONSE_TOO_LARGE', 'response_too_large');
        }
        chunks.push(Buffer.from(value));
    }
    return Buffer.concat(chunks, total);
}

async function downloadImageToFile(url, outputPath, networkPolicy = {}) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), DEFAULT_IMAGE_FETCH_TIMEOUT_MS);
    try {
        let current = (await validateNetworkUrl(url, networkPolicy, false)).url;
        let response = null;
        for (let redirect = 0; redirect <= MAX_NETWORK_REDIRECTS; redirect += 1) {
            response = await fetch(current, {
                method: 'GET',
                signal: controller.signal,
                redirect: 'manual',
                headers: {
                    'user-agent': 'RustClaw/1.0',
                    'accept': 'image/*,*/*;q=0.8',
                },
            });
            if (response.status < 300 || response.status >= 400) break;
            if (redirect === MAX_NETWORK_REDIRECTS) {
                throw new SkillError('REDIRECT_LIMIT_EXCEEDED', 'redirect_limit_exceeded');
            }
            const location = response.headers.get('location');
            if (!location) {
                throw new SkillError('REDIRECT_URL_INVALID', 'redirect_location_missing');
            }
            const next = new URL(location, current);
            if (new URL(current).protocol === 'https:' && next.protocol !== 'https:') {
                throw new SkillError('REDIRECT_SCHEME_DOWNGRADE', 'redirect_scheme_downgrade');
            }
            current = (await validateNetworkUrl(next.toString(), networkPolicy, false)).url;
        }
        if (!response) {
            throw new SkillError('RESPONSE_READ_FAILED', 'response_unavailable');
        }
        if (!response.ok) {
            throw new SkillError('HTTP_STATUS_ERROR', `http_status_${response.status}`);
        }
        const contentType = response.headers.get('content-type') || '';
        if (!contentType.toLowerCase().includes('image/')) {
            throw new SkillError('CONTENT_TYPE_BLOCKED', 'content_type_blocked');
        }
        const body = await readResponseBytes(response, DEFAULT_IMAGE_FETCH_MAX_BYTES);
        const ext = guessExtFromContentType(contentType);
        const finalPath = outputPath.endsWith(ext) ? outputPath : `${outputPath}${ext}`;
        await fs.writeFile(finalPath, body);
        return {
            ok: true,
            path: finalPath,
            bytes: body.byteLength,
            contentType,
            sha256: crypto.createHash('sha256').update(body).digest('hex'),
        };
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
    const workspaceRoot = path.resolve(options.workspaceRoot || process.cwd());
    const root = ensureWithinRoot(
        workspaceRoot,
        path.resolve(options.captureRoot || DEFAULT_CAPTURE_ROOT)
    );
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
    if (err && err.name === 'TimeoutError') {
        return new SkillError('NAV_TIMEOUT', msg);
    }
    return new SkillError('BROWSER_OPERATION_FAILED', msg);
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

async function loadWaitMap(customPath, workspaceRoot = process.cwd()) {
    const candidates = [];
    if (customPath && typeof customPath === 'string' && customPath.trim() !== '') {
        const candidate = path.isAbsolute(customPath)
            ? customPath
            : path.join(workspaceRoot, customPath.trim());
        candidates.push(ensureWithinRoot(workspaceRoot, candidate));
    }
    candidates.push(ensureWithinRoot(workspaceRoot, DEFAULT_WAIT_MAP_PATH));

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
            const response = await page.goto(url, { waitUntil, timeout: PAGE_TIMEOUT_MS });
            const responseStatus = response ? response.status() : null;
            trace.push({
                wait_until: waitUntil,
                status: 'ok',
                status_code: responseStatus,
                final_url: page.url(),
                latency_ms: Date.now() - started,
            });
            return {
                ok: true,
                waitUntil,
                attempts: trace.length,
                trace,
                responseStatus,
                finalUrl: page.url(),
            };
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

async function createBrowserContext(networkPolicy = {}) {
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

    let browser;
    try {
        browser = await playwright.chromium.launch({
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
    } catch (error) {
        throw new SkillError('DEPENDENCY_MISSING', compactErrorMessage(error));
    }

    const context = await browser.newContext({
        userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36',
        viewport: { width: 1440, height: 1024 },
        locale: 'en-US',
    });

    context.setDefaultNavigationTimeout(PAGE_TIMEOUT_MS);
    context.setDefaultTimeout(PAGE_TIMEOUT_MS);
    const networkPolicyEvents = await installNetworkGuard(context, networkPolicy);
    return { browser, context, runtimeCheck, executablePath, networkPolicyEvents };
}

function chooseChromiumExecutablePath() {
    const envOverride = process.env.BROWSER_WEB_CHROMIUM_PATH || process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH;
    if (envOverride && fsSync.existsSync(envOverride)) {
        return envOverride;
    }
    for (const p of chromiumExecutableCandidates()) {
        if (fsSync.existsSync(p)) {
            return p;
        }
    }
    return null;
}

function chromiumExecutableCandidates() {
    const homeDir = process.env.HOME || '';
    const candidates = [
        '/usr/bin/chromium',
        '/usr/bin/chromium-browser',
        '/usr/bin/google-chrome',
        '/usr/bin/google-chrome-stable',
        '/opt/homebrew/bin/chromium',
        '/opt/homebrew/bin/chromium-browser',
        '/opt/homebrew/bin/google-chrome',
        '/usr/local/bin/chromium',
        '/usr/local/bin/chromium-browser',
        '/usr/local/bin/google-chrome',
        '/snap/bin/chromium',
        '/Applications/Chromium.app/Contents/MacOS/Chromium',
        '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
        '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge',
        '/Applications/Brave Browser.app/Contents/MacOS/Brave Browser',
        path.join(homeDir, 'Applications', 'Chromium.app', 'Contents', 'MacOS', 'Chromium'),
        path.join(homeDir, 'Applications', 'Google Chrome.app', 'Contents', 'MacOS', 'Google Chrome'),
        path.join(homeDir, 'Applications', 'Microsoft Edge.app', 'Contents', 'MacOS', 'Microsoft Edge'),
        path.join(homeDir, 'Applications', 'Brave Browser.app', 'Contents', 'MacOS', 'Brave Browser'),
    ];
    const commandCandidates = [
        'chromium',
        'chromium-browser',
        'google-chrome',
        'google-chrome-stable',
        'chrome',
        'msedge',
        'microsoft-edge',
        'brave-browser',
    ];
    for (const command of commandCandidates) {
        try {
            const resolved = execFileSync('which', [command], {
                encoding: 'utf8',
                stdio: ['ignore', 'pipe', 'ignore'],
            }).trim();
            if (resolved) {
                candidates.push(resolved);
            }
        } catch {
            // Ignore missing commands and continue through the candidate chain.
        }
    }
    return [...new Set(candidates.filter(Boolean))];
}

async function readRuntimeRestrictionSignals() {
    if (process.platform !== 'linux') {
        return {
            platform: process.platform,
            no_new_privs: null,
            seccomp: null,
            restricted: false,
        };
    }
    try {
        const content = await fs.readFile('/proc/self/status', 'utf8');
        const noNewPrivs = /NoNewPrivs:\s*(\d+)/.exec(content);
        const seccomp = /Seccomp:\s*(\d+)/.exec(content);
        const noNewPrivsValue = noNewPrivs ? Number(noNewPrivs[1]) : null;
        const seccompValue = seccomp ? Number(seccomp[1]) : null;
        const restricted = noNewPrivsValue === 1 && seccompValue === 2;
        return {
            platform: process.platform,
            no_new_privs: noNewPrivsValue,
            seccomp: seccompValue,
            restricted,
        };
    } catch {
        return {
            platform: process.platform,
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
    const fullRawHtml = await page.content();
    const rawHtmlTruncated = fullRawHtml.length > MAX_CAPTURE_HTML_CHARS;
    const rawHtml = rawHtmlTruncated
        ? fullRawHtml.slice(0, MAX_CAPTURE_HTML_CHARS)
        : fullRawHtml;

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

        const fallbackTitle = norm(
            title
                || (document.querySelector('meta[property="og:title"]')?.getAttribute('content') || '')
                || (document.querySelector('meta[name="twitter:title"]')?.getAttribute('content') || '')
                || (document.querySelector('h1')?.innerText || '')
        );

        const fallbackDescription = norm(
            (document.querySelector('meta[name="description"]')?.getAttribute('content') || '')
            || (document.querySelector('meta[property="og:description"]')?.getAttribute('content') || '')
            || (document.querySelector('meta[name="twitter:description"]')?.getAttribute('content') || '')
        );

        const fallbackNodes = Array.from(
            document.querySelectorAll('h1, h2, h3, p, li, blockquote, pre, code, [role="heading"]')
        ).slice(0, 160);
        const fallbackBody = norm(
            fallbackNodes
                .map((node) => (node && typeof node.innerText === 'string' ? node.innerText : ''))
                .join('\n')
        );
        const fallbackText = norm([fallbackTitle, fallbackDescription, fallbackBody].filter(Boolean).join('\n'));
        const challengeSelectors = [
            'iframe[src*="captcha"]',
            'iframe[src*="challenges.cloudflare.com"]',
            '[id*="captcha"]',
            '[class*="captcha"]',
            'input[name*="captcha"]',
        ];
        const challengeSignals = challengeSelectors
            .map((selector) => ({ selector, count: document.querySelectorAll(selector).length }))
            .filter((entry) => entry.count > 0);

        return {
            title,
            text: useMain ? bestText : bodyText,
            fallback_title: fallbackTitle,
            fallback_text: fallbackText,
            image_candidates: imageCandidates,
            challenge_signals: challengeSignals,
            extraction_trace: {
                used_main: useMain,
                best_selector: bestSelector,
                selector_diagnostics: selectorDiagnostics,
                body_chars: bodyChars,
                selected_chars: useMain ? bestChars : bodyChars,
                fallback_chars: fallbackText.length,
            },
        };
    });

    const finalUrl = page.url();
    const rawText = normalizeWhitespace(extracted.text || '');
    const contentMode = options.contentMode === 'raw' ? 'raw' : 'clean';
    const cleaned = contentMode === 'raw' ? rawText : sanitizeExtractedText(rawText);
    const textTruncated = cleaned.length > (options.maxTextChars || DEFAULT_MAX_TEXT_CHARS);
    let finalText = truncateText(cleaned, options.maxTextChars || DEFAULT_MAX_TEXT_CHARS);
    let finalTitle = normalizeWhitespace(extracted.title || extracted.fallback_title || '');

    const fallbackRaw = normalizeWhitespace(extracted.fallback_text || '');
    const fallbackClean = contentMode === 'raw' ? fallbackRaw : sanitizeExtractedText(fallbackRaw);
    const fallbackText = truncateText(fallbackClean, options.maxTextChars || DEFAULT_MAX_TEXT_CHARS);

    if ((!finalTitle || finalText.length < 80) && fallbackText.length > finalText.length) {
        finalText = fallbackText;
    }
    if (!finalTitle) {
        finalTitle = parseDomain(finalUrl || url);
    }

    if (
        [401, 403, 429].includes(nav.responseStatus)
        || (extracted.challenge_signals || []).length > 0
    ) {
        throw new SkillError('BOT_BLOCKED', 'structured_challenge_signal', {
            nav_trace: nav.trace,
            final_url: finalUrl,
            response_status: nav.responseStatus,
            challenge_signals: extracted.challenge_signals || [],
        });
    }

    const minChars = options.minContentChars || DEFAULT_MIN_CONTENT_CHARS;
    if (finalText.length < minChars) {
        const code = extracted.extraction_trace && extracted.extraction_trace.used_main ? 'EMPTY_CONTENT' : 'SELECTOR_MISS';
        throw new SkillError(code, `page readability below threshold (title/text) with ${finalText.length} chars`, {
            nav_trace: nav.trace,
            extraction_trace: extracted.extraction_trace,
            final_url: finalUrl,
            min_content_chars: minChars,
            partial_title: finalTitle,
            partial_text: finalText,
            partial_content_excerpt: buildExcerpt(finalText),
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
        title: finalTitle,
        text: finalText,
        content_excerpt: buildExcerpt(finalText),
        source: parseDomain(finalUrl || url),
        published_at: null,
        fetch_method: 'browser',
        extracted_at: new Date().toISOString(),
        nav_wait_until: nav.waitUntil,
        nav_attempts: nav.attempts,
        nav_attempt_trace: nav.trace,
        response_status: nav.responseStatus,
        latency_ms: Date.now() - pageStart,
        extraction_trace: extracted.extraction_trace,
        text_truncated: textTruncated,
        text_chars_before_limit: cleaned.length,
        content_sha256: sha256Hex(finalText),
        screenshot_path: screenshotPath,
        raw_html: rawHtml,
        raw_html_truncated: rawHtmlTruncated,
        raw_html_chars_before_limit: fullRawHtml.length,
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
        captureImages = false,
        maxCaptureImages = DEFAULT_MAX_CAPTURE_IMAGES,
        domainsAllow = [],
        domainsDeny = [],
        workspaceRoot = process.cwd(),
    } = input;

    if (!urls || !Array.isArray(urls) || urls.length === 0) {
        throw new SkillError('EMPTY_CONTENT', 'urls array is required and must not be empty');
    }
    if (!['clean', 'raw'].includes(contentMode)) {
        throw new SkillError('EMPTY_CONTENT', 'contentMode must be one of clean|raw');
    }

    const networkPolicy = { domainsAllow, domainsDeny };
    const validatedUrls = [];
    for (const url of urls) {
        validatedUrls.push((await validateNetworkUrl(url, networkPolicy, true)).url);
    }
    const resolvedWorkspaceRoot = path.resolve(workspaceRoot);
    const { map: waitMap, path: waitMapResolved } = await loadWaitMap(
        waitMapPath,
        resolvedWorkspaceRoot
    );
    const capture = await initCaptureStorage({
        enableCapture,
        captureRoot,
        captureSource,
        runId,
        source: 'browser_web',
        workspaceRoot: resolvedWorkspaceRoot,
    });
    const effectiveScreenshotDir = ensureWithinRoot(
        resolvedWorkspaceRoot,
        screenshotDir
        ? (path.isAbsolute(screenshotDir) ? screenshotDir : path.join(resolvedWorkspaceRoot, screenshotDir))
        : (capture.enabled ? capture.dirs.images : DEFAULT_SCREENSHOT_ROOT)
    );
    const {
        browser,
        context,
        runtimeCheck,
        executablePath,
        networkPolicyEvents,
    } = await createBrowserContext(networkPolicy);
    const items = [];

    try {
        for (const url of validatedUrls) {
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
                                const dl = await downloadImageToFile(img.src, stem, networkPolicy);
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
                                    sha256: dl.sha256,
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
                item.trust = {
                    classification: 'untrusted_web_content',
                    instructions_executable: false,
                };
                item.provenance = {
                    source: 'browser',
                    requested_url: url,
                    final_url: item.final_url || url,
                    observed_at: item.extracted_at,
                };
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
                const { classified, item } = partialExtractionItem(url, err, executablePath, runtimeCheck);
                const pageOrdinal = items.length + 1;
                items.push(item);
                if (capture.enabled) {
                    await appendJsonl(capture.files.manifest, {
                        run_id: capture.runId,
                        source: capture.source,
                        ordinal: pageOrdinal,
                        status: 'error',
                        url,
                        final_url: item.final_url,
                        title: item.title,
                        fetched_at: item.extracted_at,
                        fetch_method: item.fetch_method,
                        text_chars: item.text.length,
                        content_hash_sha256: item.text ? sha256Hex(item.text) : null,
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
    const sourceRefs = items
        .filter((item) => item.final_url || item.url)
        .map((item, index) => ({
            url: item.final_url || item.url,
            title: item.title || null,
            rank: index + 1,
            kind: 'browser_page',
            content_sha256: item.content_sha256 || null,
        }));

    return {
        items,
        summary: 'browser_extract_result_set',
        success_count: successCount,
        failure_count: failureCount,
        citations,
        source_refs: sourceRefs,
        trust: {
            classification: 'untrusted_web_content',
            instructions_executable: false,
        },
        network_policy: {
            decisions: networkPolicyEvents,
            decisions_truncated: networkPolicyEvents.length >= MAX_NETWORK_POLICY_EVENTS,
        },
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

function partialExtractionItem(url, err, executablePath, runtimeCheck) {
    const classified = classifyError(err);
    const meta = err && err.meta && typeof err.meta === 'object' ? err.meta : null;
    const title = normalizeWhitespace(meta && meta.partial_title ? meta.partial_title : '');
    const text = normalizeWhitespace(meta && meta.partial_text ? meta.partial_text : '');
    const finalUrl = normalizeWhitespace(meta && meta.final_url ? meta.final_url : '') || url;
    return {
        classified,
        item: {
            url,
            final_url: finalUrl,
            title,
            text,
            content_excerpt: normalizeWhitespace(
                meta && meta.partial_content_excerpt ? meta.partial_content_excerpt : buildExcerpt(text)
            ),
            source: parseDomain(finalUrl),
            published_at: null,
            fetch_method: title || text ? 'browser_partial' : 'unavailable',
            extracted_at: new Date().toISOString(),
            error_code: classified.code,
            error: classified.message,
            diagnostics: meta,
            content_sha256: text ? sha256Hex(text) : null,
            trust: {
                classification: 'untrusted_web_content',
                instructions_executable: false,
            },
            runtime: {
                chromium_executable_path: executablePath || 'playwright-default',
                runtime_restriction_signals: runtimeCheck,
            },
        },
    };
}

function isRetryableErrorCode(code) {
    return new Set([
        'BROWSER_OPERATION_FAILED',
        'DNS_RESOLUTION_FAILED',
        'NAV_TIMEOUT',
    ]).has(code);
}

function writeFailure(error, details = null) {
    const classified = classifyError(error);
    process.stderr.write(`${JSON.stringify({
        error_code: classified.code,
        error_text: classified.message,
        retryable: isRetryableErrorCode(classified.code),
        details: details || classified.meta || null,
    })}\n`);
}

async function main() {
    let inputData = '';
    process.stdin.setEncoding('utf8');

    for await (const chunk of process.stdin) {
        inputData += chunk;
    }

    if (!inputData.trim()) {
        writeFailure(new SkillError('INVALID_INPUT', 'input_empty'));
        process.exit(1);
    }

    let input;
    try {
        input = JSON.parse(inputData.trim());
    } catch (e) {
        writeFailure(new SkillError('INVALID_INPUT', 'input_json_invalid', {
            parser_error: compactErrorMessage(e),
        }));
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
            default:
                throw new SkillError('INVALID_ACTION', 'unsupported_action');
        }

        process.stdout.write(`${JSON.stringify(result)}\n`);
    } catch (error) {
        const classified = classifyError(error);
        const runtimeCheck = await readRuntimeRestrictionSignals();
        if (runtimeCheck.restricted) {
            writeFailure(
                new SkillError('DEPENDENCY_MISSING', classified.message),
                {
                    cause_error_code: classified.code,
                    runtime_restriction: runtimeCheck,
                    cause_details: classified.meta || null,
                }
            );
        } else {
            writeFailure(classified, {
                runtime_restriction: runtimeCheck,
                cause_details: classified.meta || null,
            });
        }
        process.exit(1);
    }
}

if (require.main === module) {
    main().catch((error) => {
        writeFailure(error);
        process.exit(1);
    });
}

module.exports = {
    SkillError,
    classifyError,
    isPrivateIp,
    partialExtractionItem,
    validateNetworkUrl,
};
