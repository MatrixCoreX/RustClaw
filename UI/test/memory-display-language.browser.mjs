// Actual compiled UI; fixture reads only, no native/provider or production claims.
import assert from 'node:assert/strict';
import {mkdir, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {builtUiServer} from './helpers/built-ui-server.mjs';
import {contents, memoryDisplayFixture} from './helpers/memory-display-fixture.mjs';

const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = fileURLToPath(new URL('../', import.meta.url));
const output = process.env.UI_BROWSER_ARTIFACTS || join(tmpdir(), `memory-display-${Date.now()}`);
await mkdir(output, {recursive: true});
const server = await builtUiServer(process.env.UI_DIST_DIR || join(root, 'dist'));
let browser;
const results = [];
try {
  browser = await chromium.launch({executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true});
  for (const width of [1440, 390]) {
    const page = await browser.newPage({viewport: {width, height: 1000}, locale: 'zh-CN', serviceWorkers: 'block'});
    const errors = [], denied = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => {
      localStorage.setItem('agent-runtime.monitor.currentPage', 'memory');
      localStorage.setItem('agent-runtime.monitor.pollingSeconds', '0');
      localStorage.setItem('agent-runtime.monitor.lang', 'zh');
    });
    await page.route('**/*', async route => {
      const request = route.request(), url = new URL(request.url());
      if (url.origin !== server.base) { denied.push(url.origin); await route.abort(); return; }
      if (!url.pathname.startsWith('/v1/') && !url.pathname.startsWith('/webd/')) {
        await route.continue(); return;
      }
      if (request.method() !== 'GET' && !(request.method() === 'POST' && url.pathname === '/webd/login')) {
        denied.push(request.method() + ' ' + url.pathname); await route.abort(); return;
      }
      const data = memoryDisplayFixture(url.pathname);
      await route.fulfill({status: data === undefined ? 404 : 200, contentType: 'application/json',
        body: JSON.stringify(data === undefined ? {ok: false, error: 'fixture_unavailable'} : {ok: true, data})});
    });
    try {
      await page.goto(server.base);
      await page.getByLabel('用户名', {exact: true}).waitFor();
      assert.equal(await page.locator('html').getAttribute('lang'), 'zh-CN');
      await page.getByLabel('Webd 地址（可选）', {exact: true}).fill(server.base);
      await page.getByLabel('用户名', {exact: true}).fill('fixture-user');
      await page.getByLabel('密码', {exact: true}).fill('fixture-password');
      await page.getByRole('button', {name: '进入控制台', exact: true}).click();
      await page.getByRole('heading', {name: '记忆条目', exact: true}).waitFor();
      const show = page.getByRole('button', {name: '展开全文', exact: true});
      await show.first().waitFor();
      const measurements = await show.evaluateAll(buttons => buttons.map(button => {
        const paragraph = document.getElementById(button.getAttribute('aria-controls'));
        return {text: paragraph.textContent, height: paragraph.clientHeight,
          lineHeight: Number.parseFloat(getComputedStyle(paragraph).lineHeight)};
      }));
      assert.equal(measurements.length, 68, 'all20 paged items plus all48 fact cards must retain disclosure');
      for (const row of measurements) {
        assert.ok(contents.includes(row.text), 'complete long string must remain accessible without slicing');
        assert.ok(row.height <= row.lineHeight * 3 + 2, 'collapsed display should remain three visual lines');
      }
      const id = await show.first().getAttribute('aria-controls');
      const before = await page.evaluate(id => document.getElementById(id).textContent, id);
      await show.first().click();
      const collapse = page.getByRole('button', {name: '收起全文', exact: true});
      assert.equal(await collapse.getAttribute('aria-expanded'), 'true');
      const expandedHeight = await page.evaluate(id => document.getElementById(id).clientHeight, id);
      assert.ok(expandedHeight > measurements[0].height);
      assert.equal(await page.evaluate(id => document.getElementById(id).textContent, id), before);
      await collapse.click();
      assert.equal(await page.evaluate(id => document.getElementById(id).textContent, id), before);
      const dimensions = await page.evaluate(() => ({height: document.documentElement.scrollHeight,
        width: document.documentElement.scrollWidth, viewport: innerWidth}));
      assert.ok(dimensions.width <= dimensions.viewport);
      for (const language of ['zh', 'en', 'zh']) {
        if ((await page.locator('html').getAttribute('lang')) !== (language === 'zh' ? 'zh-CN' : 'en')) {
          await page.getByRole('button', {name: language === 'zh' ? 'English' : '中文', exact: true}).click();
        }
        assert.equal(await page.locator('html').getAttribute('lang'), language === 'zh' ? 'zh-CN' : 'en');
        await page.getByRole('button', {name: language === 'zh' ? '导出 JSON' : 'Export JSON', exact: true}).click();
        const dialog = page.getByRole('alertdialog'); await dialog.waitFor();
        const cancel = dialog.getByRole('button', {name: language === 'zh' ? '取消' : 'Cancel', exact: true}).last();
        await cancel.click(); await dialog.waitFor({state: 'hidden'});
      }
      await page.screenshot({path: join(output, `memory-${width}.png`)});
      assert.deepEqual(errors, []); assert.deepEqual(denied, []);
      results.push({width, passed: true, fullStrings: measurements.length, dimensions,
        expandedHeight, languages: ['zh', 'en', 'zh'], real_backend_writes: 0, provider_calls: 0});
    } finally { await page.close(); }
  }
  await writeFile(join(output, 'report.json'), JSON.stringify({results}, null, 2));
  console.log(JSON.stringify({results}));
} finally { await browser?.close(); await server.close(); }
