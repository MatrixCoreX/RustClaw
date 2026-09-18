const test = require('node:test');
const assert = require('node:assert/strict');
const { createBrowserContext, extractPage } = require('./browser_web.js');

test('challenge detection distinguishes document identifiers from visible controls', async (t) => {
    const { browser, context } = await createBrowserContext();
    t.after(() => browser.close());
    const page = await context.newPage();
    let body = '';
    await page.route('**/*', (route) => route.fulfill({
        status: 200,
        contentType: 'text/html',
        body: new URL(route.request().url()).pathname === '/document' ? body : '',
    }));
    const documentText = '<main><h1>Verification accessibility</h1><p>'
        + 'Documentation describes accessible alternatives for challenge controls. '.repeat(8)
        + '</p></main>';
    const examples = [
        ['definition and reference IDs', '<dfn id="dfn-captcha">CAPTCHA</dfn>'
            + '<a id="ref-for-dfn-captcha-1">Reference</a><p class="captcha-notes">Notes</p>', false],
        ['hidden verification iframe', '<iframe hidden src="https://fixture.invalid/captcha"></iframe>', false],
        ['hidden verification input', '<input name="captcha" type="hidden" value="token">', false],
        ['invisible verification parent', '<div style="display:none"><input name="captcha"></div>', false],
        ['visible verification input', '<input name="captcha" autocomplete="off">', true],
        ['visible verification iframe', '<iframe src="https://fixture.invalid/captcha"></iframe>', true],
        ['visible challenge provider iframe', '<iframe src="https://challenges.cloudflare.com/widget"></iframe>', true],
    ];
    for (const [label, markup, blocked] of examples) {
        await t.test(label, async () => {
            body = '<!doctype html><title>Fixture</title>' + documentText + markup;
            const result = extractPage(page, 'https://fixture.invalid/document', 'domcontentloaded', {
                deadlineAt: Date.now() + 15_000,
                maxTextChars: 5000,
                minContentChars: 20,
            });
            if (blocked) {
                await assert.rejects(result, (error) => error.code === 'BOT_BLOCKED'
                    && error.meta.challenge_signals.some((signal) => signal.count > 0));
            } else {
                assert.equal((await result).title, 'Fixture');
            }
        });
    }
});
