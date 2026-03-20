const fs = require('fs/promises');
const path = require('path');
const { spawn } = require('child_process');

const ROOT = __dirname;
const HELPER = path.join(ROOT, 'browser_web.js');
const CASES = path.join(ROOT, 'regression_sites.json');
const REPO_ROOT = path.resolve(ROOT, '..', '..', '..');

function runHelper(payload) {
    return new Promise((resolve) => {
        const childEnv = { ...process.env };
        delete childEnv.DISPLAY;
        delete childEnv.WAYLAND_DISPLAY;
        delete childEnv.XAUTHORITY;

        const child = spawn('node', [HELPER], {
            cwd: REPO_ROOT,
            env: childEnv,
            stdio: ['pipe', 'pipe', 'pipe'],
        });
        let stdout = '';
        let stderr = '';
        child.stdout.on('data', (d) => {
            stdout += d.toString('utf8');
        });
        child.stderr.on('data', (d) => {
            stderr += d.toString('utf8');
        });
        child.on('close', (code) => {
            if (code !== 0) {
                resolve({
                    ok: false,
                    error: stderr.trim() || `helper exited with code ${code}`,
                });
                return;
            }
            try {
                resolve({
                    ok: true,
                    result: JSON.parse((stdout || '').trim()),
                });
            } catch (err) {
                resolve({
                    ok: false,
                    error: `invalid helper stdout: ${err.message}`,
                });
            }
        });
        child.stdin.write(`${JSON.stringify(payload)}\n`);
        child.stdin.end();
    });
}

function summarizeOpenExtract(outcome) {
    if (!outcome.ok) {
        return { ok: false, success: 0, total: 0, navTimeout: 0, blocked: 0 };
    }
    const items = outcome.result.items || [];
    const success = items.filter((x) => x.fetch_method === 'browser').length;
    const navTimeout = items.filter((x) => x.error_code === 'NAV_TIMEOUT').length;
    const blocked = items.filter((x) => x.error_code === 'BOT_BLOCKED').length;
    return {
        ok: true,
        success,
        total: items.length,
        navTimeout,
        blocked,
    };
}

function summarizeSearch(outcome) {
    if (!outcome.ok) {
        return { ok: false, total: 0 };
    }
    const items = outcome.result.items || [];
    return {
        ok: true,
        total: items.length,
    };
}

async function main() {
    const content = await fs.readFile(CASES, 'utf8');
    const cases = JSON.parse(content);

    const openPayload = {
        action: 'openExtract',
        urls: cases.open_extract_urls || [],
        waitUntil: 'domcontentloaded',
        contentMode: 'clean',
        maxTextChars: 12000,
        failFast: false,
        saveScreenshot: false,
    };

    const openOutcome = await runHelper(openPayload);
    const openStats = summarizeOpenExtract(openOutcome);

    const searchStats = [];
    for (const q of cases.search_queries || []) {
        const payload = {
            action: 'searchPage',
            query: q.query,
            engine: 'google',
            topK: 5,
            region: q.region || null,
            lang: q.lang || 'en',
        };
        const outcome = await runHelper(payload);
        searchStats.push({
            query: q.query,
            ...summarizeSearch(outcome),
            error: outcome.ok ? null : outcome.error,
        });
    }

    const successRate = openStats.total > 0 ? openStats.success / openStats.total : 0;
    const allSearchNonEmpty = searchStats.every((s) => s.ok && s.total > 0);
    const pass = openStats.ok && successRate >= 0.6 && allSearchNonEmpty;

    const report = {
        pass,
        generated_at: new Date().toISOString(),
        open_extract: {
            ...openStats,
            success_rate: successRate,
        },
        search_page: searchStats,
    };

    console.log(JSON.stringify(report, null, 2));
    if (!pass) {
        process.exit(2);
    }
}

main().catch((err) => {
    console.error(err.message || String(err));
    process.exit(2);
});
