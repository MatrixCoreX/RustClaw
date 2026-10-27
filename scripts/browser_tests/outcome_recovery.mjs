import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { chromium } from 'playwright';

const base = process.env.UI_TEST_BASE_URL || 'http://127.0.0.1:3000';
const out = 'tmp/outcome-recovery-browser';
await mkdir(out, { recursive: true });
const browser = await chromium.launch({ headless: true, channel: process.env.UI_TEST_BROWSER_CHANNEL || 'chrome' });
try {
  for (const [name, width, height, theme] of [['desktop', 1440, 900, 'dark'], ['mobile', 390, 844, 'dark'], ['desktop-light', 1440, 900, 'light'], ['mobile-light', 390, 844, 'light']]) {
    const page = await browser.newPage({ viewport: { width, height } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`${base}/test/outcome-recovery.html`);
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme; document.body.classList.add('theme-shell'); }, theme);
    await page.getByRole('button', { name: '核查并补发' }).click();
    await page.getByRole('alertdialog').waitFor();
    assert.match(await page.getByRole('alertdialog').evaluate(el => getComputedStyle(el).backgroundColor), /^rgb\(/);
    await page.getByText(/补发可能造成重复/).waitFor();
    assert.equal(await page.getByTestId('decision').textContent(), '');
    await page.screenshot({ path: `${out}/${name}-confirmation.png`, fullPage: true });
    await page.getByRole('alertdialog').getByRole('button', { name: '取消', exact: true }).last().click();
    assert.equal(await page.getByTestId('decision').textContent(), '');
    await page.getByRole('button', { name: '核查并补发' }).click();
    await page.getByRole('button', { name: '确认未收到，补发', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[data-testid="decision"]').textContent.startsWith('retry_unknown:'));
    await page.getByRole('button', { name: '我已收到', exact: true }).click();
    await page.getByText(/不会把你的确认当作渠道送达证明/).waitFor();
    await page.getByRole('alertdialog').getByRole('button', { name: '我已收到', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[data-testid="decision"]').textContent.startsWith('confirm_received:'));
    const layout = await page.evaluate(() => ({ width: innerWidth, scroll: document.documentElement.scrollWidth }));
    assert.ok(layout.scroll <= layout.width, JSON.stringify(layout));
    assert.deepEqual(errors, []);
    await page.screenshot({ path: `${out}/${name}-status.png`, fullPage: true });
    await page.close();
    console.log(`OUTCOME_RECOVERY_BROWSER ${name} ok`);
  }
} finally { await browser.close(); }
