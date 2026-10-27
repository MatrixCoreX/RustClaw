import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { chromium } from 'playwright';

const base = process.env.UI_TEST_BASE_URL || 'http://127.0.0.1:3022';
const output = 'tmp/financial-request-ownership-browser';
await mkdir(output, { recursive: true });
const browser = await chromium.launch({ headless: true, channel: process.env.UI_TEST_BROWSER_CHANNEL || 'chrome' });
try {
  for (const [name, width, height, theme] of [
    ['desktop-dark', 1440, 900, 'dark'], ['desktop-light', 1440, 900, 'light'],
    ['mobile-dark', 390, 844, 'dark'], ['mobile-light', 390, 844, 'light'],
  ]) {
    const page = await browser.newPage({ viewport: { width, height } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`${base}/test/financial-request-ownership.html`);
    await page.evaluate(value => { document.documentElement.dataset.theme = value; document.body.classList.add('theme-shell'); }, theme);
    await page.getByRole('button', { name: 'Slow requests', exact: true }).click();
    await page.getByRole('button', { name: 'Switch owner', exact: true }).click();
    await page.getByRole('button', { name: 'Switch owner', exact: true }).click();
    await page.waitForTimeout(600);
    for (const field of ['market', 'history', 'binding', 'transfer']) {
      assert.equal(await page.getByTestId(field).textContent(), 'none', `${name}:${field}`);
    }
    assert.equal(await page.getByTestId('loading').textContent(), 'false');
    await page.getByRole('button', { name: 'Fresh requests', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[data-testid="transfer"]').textContent === 'asset_transfer_completed');
    assert.equal(await page.getByTestId('market').textContent(), 'A');
    assert.equal(await page.getByTestId('history').textContent(), 'owner');
    assert.equal(await page.getByTestId('binding').textContent(), 'owner');
    await page.getByRole('button', { name: 'Sign out', exact: true }).click();
    const requests = await page.getByTestId('requests').textContent();
    await page.getByRole('button', { name: 'Fresh requests', exact: true }).click();
    assert.equal(await page.getByTestId('requests').textContent(), requests);
    for (const field of ['market', 'history', 'binding', 'transfer']) {
      assert.equal(await page.getByTestId(field).textContent(), 'none');
    }
    assert.equal(await page.getByTestId('error').textContent(), '');
    assert.deepEqual(errors, []);
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await page.screenshot({ path: `${output}/${name}.png`, fullPage: true });
    await page.close();
    console.log(`FINANCIAL_REQUEST_OWNERSHIP_BROWSER ${name} ok`);
  }
  for (const width of [1440, 390]) {
    const page = await browser.newPage({ viewport: { width, height: 1000 } });
    const errors = [];
    const mutations = [];
    let configReads = 0;
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => {
      localStorage.setItem('agent-runtime.monitor.authMode', 'webd');
      localStorage.setItem('agent-runtime.monitor.baseUrl', location.origin);
      localStorage.setItem('agent-runtime.monitor.lang', 'en');
      localStorage.setItem('agent-runtime.monitor.currentPage', 'assets');
      localStorage.setItem('agent-runtime.monitor.nniNavigationVisible', 'true');
    });
    await page.route('**/*', async route => {
      const request = route.request();
      const url = new URL(request.url());
      if (url.origin !== new URL(base).origin) return route.abort();
      if (!url.pathname.startsWith('/v1/') && !url.pathname.startsWith('/webd/')) return route.continue();
      if (request.method() !== 'GET') mutations.push(url.pathname);
      if (url.pathname === '/webd/session') {
        return route.fulfill({ json: { ok: true, data: { logged_in: true, csrf_token: 'ab'.repeat(16) } } });
      }
      if (url.pathname === '/v1/auth/me') {
        return route.fulfill({ json: { ok: true, data: { user_id: 1, chat_id: 1, role: 'admin', key_id: 1 } } });
      }
      if (url.pathname === '/v1/nni/config') {
        configReads++;
        return route.fulfill({ json: { ok: true, data: { joined: false, remote_nodes: ['https://node.example.test'], asset_owner_pubkey: null } } });
      }
      return route.fulfill({ status: 404, json: { ok: false, error: 'fixture_unavailable' } });
    });
    await page.goto(base);
    try {
      await page.getByText('Asset overview', { exact: true }).waitFor({ timeout: 15_000 });
    } catch (error) {
      await page.screenshot({ path: `${output}/app-${width}-failure.png`, fullPage: true });
      console.log(JSON.stringify({ width, errors, configReads, body: (await page.locator('body').innerText()).slice(0, 1800) }));
      throw error;
    }
    await page.waitForTimeout(500);
    assert.ok(configReads >= 1 && configReads < 10, `bounded config reads: ${configReads}`);
    assert.deepEqual(errors, []);
    assert.deepEqual(mutations, []);
    await page.screenshot({ path: `${output}/app-${width}.png`, fullPage: true });
    await page.close();
    console.log(`FINANCIAL_REQUEST_OWNERSHIP_BROWSER app-${width} ok`);
  }
} finally { await browser.close(); }
