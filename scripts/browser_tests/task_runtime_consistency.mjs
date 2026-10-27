import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { chromium } from 'playwright';

const base = process.env.UI_TEST_BASE_URL || 'http://127.0.0.1:3000';
const output = 'tmp/task-runtime-consistency-browser';
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
    await page.addInitScript(() => Object.defineProperty(globalThis.crypto, 'randomUUID', { value: undefined, configurable: true }));
    await page.goto(`${base}/test/task-runtime-consistency.html`);
    await page.evaluate(value => { document.documentElement.dataset.theme = value; document.body.classList.add('theme-shell'); }, theme);
    await page.getByRole('button', { name: '慢任务', exact: true }).click();
    await page.getByRole('button', { name: '快任务', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[data-testid="task"]').textContent === 'fast');
    await page.waitForTimeout(450);
    assert.equal(await page.getByTestId('task').textContent(), 'fast');
    await page.getByText('状态暂不可用', { exact: true }).waitFor();
    await page.getByRole('button', { name: '暂停任务', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[data-testid="control"]').textContent.includes('任务已暂停'));
    assert.equal(await page.getByTestId('error').textContent(), '');
    await page.screenshot({ path: `${output}/${name}.png`, fullPage: true });
    await page.getByRole('button', { name: '慢任务', exact: true }).click();
    await page.getByRole('button', { name: '切换身份', exact: true }).click();
    await page.waitForTimeout(450);
    assert.equal(await page.getByTestId('task').textContent(), 'none');
    assert.equal(await page.getByTestId('owner').textContent(), 'two');
    assert.equal(await page.getByTestId('loading').textContent(), 'false');
    assert.deepEqual(errors, []);
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await page.close();
    console.log(`TASK_RUNTIME_CONSISTENCY_BROWSER ${name} ok`);
  }
} finally { await browser.close(); }
