const test = require('node:test');
const assert = require('node:assert/strict');

const { SkillError, partialExtractionItem } = require('./browser_web.js');

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
        new Error('playwright executable missing'),
        null,
        { restricted: false },
    );

    assert.equal(item.fetch_method, 'unavailable');
    assert.equal(item.title, '');
    assert.equal(item.text, '');
    assert.equal(item.error_code, 'DEPENDENCY_MISSING');
});
