const test = require('node:test');
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const path = require('node:path');

const {
    SkillError,
    classifyError,
    isPrivateIp,
    partialExtractionItem,
    validateNetworkUrl,
} = require('./browser_web.js');

test('preserves extracted title when readability threshold rejects short body text', () => {
    const error = new SkillError('SELECTOR_MISS', 'page readability below threshold', {
        final_url: 'https://example.com/',
        min_content_chars: 200,
        partial_title: 'Example Domain',
        partial_text: 'Short readable body.',
        partial_content_excerpt: 'Short readable body.',
    });

    const { classified, item } = partialExtractionItem(
        'https://example.com',
        error,
        '/usr/bin/chromium',
        { restricted: false },
    );

    assert.equal(classified.code, 'SELECTOR_MISS');
    assert.equal(item.fetch_method, 'browser_partial');
    assert.equal(item.final_url, 'https://example.com/');
    assert.equal(item.title, 'Example Domain');
    assert.equal(item.text, 'Short readable body.');
    assert.equal(item.error_code, 'SELECTOR_MISS');
});

test('keeps dependency failures unavailable without invented partial content', () => {
    const { item } = partialExtractionItem(
        'https://example.com',
        new SkillError('DEPENDENCY_MISSING', 'dependency_unavailable'),
        null,
        { restricted: false },
    );

    assert.equal(item.fetch_method, 'unavailable');
    assert.equal(item.title, '');
    assert.equal(item.text, '');
    assert.equal(item.error_code, 'DEPENDENCY_MISSING');
});

test('does not infer machine error codes from natural-language exception text', () => {
    const classified = classifyError(new Error('playwright chromium timeout selector captcha'));

    assert.equal(classified.code, 'BROWSER_OPERATION_FAILED');
});

test('blocks literal private targets and credential-bearing URLs', async () => {
    for (const url of [
        'http://127.0.0.1/',
        'http://169.254.169.254/latest/meta-data/',
        'https://user:secret@example.com/',
        'http://service.local/',
    ]) {
        await assert.rejects(
            validateNetworkUrl(url),
            (error) => error instanceof SkillError
                && ['PRIVATE_NETWORK_BLOCKED', 'URL_CREDENTIALS_BLOCKED'].includes(error.code)
        );
    }
});

test('network address classifier covers private and public literals', () => {
    assert.equal(isPrivateIp('10.0.0.1'), true);
    assert.equal(isPrivateIp('198.18.0.1'), true);
    assert.equal(isPrivateIp('::1'), true);
    assert.equal(isPrivateIp('fc00::1'), true);
    assert.equal(isPrivateIp('1.1.1.1'), false);
    assert.equal(isPrivateIp('2606:4700:4700::1111'), false);
});

test('helper process returns a single structured failure envelope', () => {
    const result = spawnSync(
        process.execPath,
        [path.join(__dirname, 'browser_web.js')],
        {
            input: `${JSON.stringify({ action: 'removed_search_action' })}\n`,
            encoding: 'utf8',
        },
    );

    assert.equal(result.status, 1);
    assert.equal(result.stdout, '');
    const lines = result.stderr.trim().split('\n');
    assert.equal(lines.length, 1);
    const failure = JSON.parse(lines[0]);
    assert.equal(failure.error_code, 'INVALID_ACTION');
    assert.equal(failure.error_text, 'unsupported_action');
    assert.equal(failure.retryable, false);
});
