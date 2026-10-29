import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { chromium } from 'playwright';

const base = process.env.UI_TEST_BASE_URL || 'http://127.0.0.1:3001';
const output = 'tmp/task-evidence-browser';
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
    await page.goto(`${base}/test/fixtures/task-evidence.html`);
    await page.evaluate(value => { document.documentElement.dataset.theme = value; document.body.classList.add('theme-shell'); }, theme);
    await page.getByText('1–12 / 25', { exact: true }).waitFor();
    await page.getByRole('button', { name: 'Next evidence page' }).click();
    await page.getByText('13–24 / 25', { exact: true }).waitFor();
    await page.locator('summary').first().click();
    assert.ok((await page.locator('dd').first().textContent()).startsWith('kb:fixture/12:'));
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await page.screenshot({ path: `${output}/${name}.png`, fullPage: true });
    await page.getByRole('button', { name: 'Next evidence page' }).click();
    await page.getByText('25–25 / 25', { exact: true }).waitFor();
    assert.equal(await page.getByRole('button', { name: 'Next evidence page' }).isDisabled(), true);
    await page.getByRole('button', { name: 'Switch task' }).click();
    await page.getByText('1–12 / 25', { exact: true }).waitFor();
    assert.equal(await page.getByRole('button', { name: 'Previous evidence page' }).isDisabled(), true);
    assert.equal(await page.locator('details[open]').count(), 0);
    assert.deepEqual(errors, []);
    await page.close();
    console.log(`TASK_EVIDENCE_BROWSER ${name} ok`);
  }
} finally { await browser.close(); }
