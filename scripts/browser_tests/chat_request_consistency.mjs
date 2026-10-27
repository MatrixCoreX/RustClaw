import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { chromium } from 'playwright';

const base = process.env.UI_TEST_BASE_URL || 'http://127.0.0.1:3019';
const output = 'tmp/chat-request-consistency-browser';
await mkdir(output, { recursive: true });
const browser = await chromium.launch({ headless: true, channel: 'chrome' });
try {
  for (const [name, width, height, theme] of [
    ['desktop-dark', 1440, 900, 'dark'], ['desktop-light', 1440, 900, 'light'],
    ['mobile-dark', 390, 844, 'dark'], ['mobile-light', 390, 844, 'light'],
  ]) {
    const page = await browser.newPage({ viewport: { width, height } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`${base}/test/chat-request-consistency.html`);
    await page.evaluate(value => { document.documentElement.dataset.theme = value; document.body.classList.add('theme-shell'); }, theme);
    await page.getByRole('button', { name: 'Slow trace', exact: true }).click();
    await page.getByRole('button', { name: 'Fast trace', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[data-testid="trace"]').textContent === '2');
    await page.waitForTimeout(650);
    assert.equal(await page.getByTestId('trace').textContent(), '2');
    await page.getByRole('button', { name: 'Slow trace', exact: true }).click();
    await page.getByRole('button', { name: 'Switch owner', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[data-testid="owner"]').textContent === 'two');
    await page.getByRole('button', { name: 'Switch owner', exact: true }).click();
    await page.waitForTimeout(650);
    assert.equal(await page.getByTestId('trace').textContent(), 'none');
    assert.equal(await page.getByTestId('loading').textContent(), 'false');
    assert.equal(await page.getByTestId('error').textContent(), '');
    assert.deepEqual(errors, []);
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await page.screenshot({ path: `${output}/${name}.png`, fullPage: true });
    await page.close();
    console.log(`CHAT_REQUEST_CONSISTENCY_BROWSER ${name} ok`);
  }
} finally { await browser.close(); }
