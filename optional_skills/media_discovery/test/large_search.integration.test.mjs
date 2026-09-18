import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright";
import { collectPlatform } from "../src/browser.mjs";
import { handleRequest } from "../src/main.mjs";
import { sourceTargets } from "../src/platforms.mjs";
import { readRecords } from "../src/storage.mjs";

const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";

test("real browser collects 300 virtualized search posts and resumes beyond them after reopening", { skip: !enabled }, async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-large-search-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const context = { skill_storage: { storage_kind: "directory", directory_path: root } };
  const config = { platform: "douyin", source_mode: "topics", topics: ["财经"], browser_mode: "silent" };
  const source = sourceTargets("douyin", config)[0].url;
  const clicked = [];
  const launch = chromium.launchPersistentContext.bind(chromium);
  t.mock.method(chromium, "launchPersistentContext", async (...args) => {
    const browser = await launch(...args);
    await browser.exposeFunction("recordPost", id => clicked.push(id));
    await browser.route("**/*", route => {
      if (!route.request().isNavigationRequest()) return route.abort();
      return route.fulfill({ contentType: "text/html", body: `<body>
        <input data-e2e="searchbar-input"><button data-e2e="searchbar-button">search</button>
        <main></main><script>
        let position=1;
        const query=${JSON.stringify(source)};
        function render(){
          const id=String(73000000+position);
          document.querySelector('main').innerHTML='<a href="https://www.douyin.com/video/'+id+'">post '+position+'</a>';
          document.querySelector('main a').onclick=async event=>{
            event.preventDefault();const href=event.currentTarget.href;await window.recordPost(id);
            history.pushState({},'',href);
            const svg='<svg xmlns="http://www.w3.org/2000/svg" width="420" height="420"><rect width="420" height="420" fill="#278b9a"/><text x="20" y="200" font-size="40">post '+id+'</text></svg>';
            document.querySelector('main').innerHTML='<h1>post '+id+'</h1><time datetime="2026-09-14">2026-09-14</time><img width="420" height="420" src="data:image/svg+xml;base64,'+btoa(svg)+'">';
          };
        }
        document.querySelector('button').onclick=()=>{history.pushState({},'',query);render()};
        window.scrollBy=()=>{if(position<305)position++;render()};
        onpopstate=()=>render();
        </script></body>` });
    });
    return browser;
  });
  // Test fixtures accelerate waits only; production pacing bounds remain intact.
  const runtime = { collectPlatform: request => collectPlatform({ ...request,
    config: { ...request.config, pacing_min_delay_ms: 1, pacing_max_delay_ms: 1 } }) };
  const result = await handleRequest({ args: { ...config, action: "run_once", max_items_per_run: 300 }, context }, runtime);
  assert.equal(result.status, "ok", JSON.stringify(result));
  assert.equal(result.extra.run.counts.items, 300);
  assert.equal(result.extra.run.counts.images, 300);
  assert.equal(result.extra.run.collection_outcome.stop_reason, "target_reached");
  assert.equal(result.extra.run.collection_outcome.sources[0].scrolls, 299);
  const expected = Array.from({ length: 300 }, (_, i) => String(73000001 + i));
  assert.deepEqual(clicked, expected);
  let records = await readRecords(root);
  assert.equal(records.length, 300);
  assert.ok((await fs.stat(path.join(root, "exports", records.at(-1).image_screenshot_path))).size > 512);
  await handleRequest({ args: { ...config, action: "enable", max_items_per_run: 2, confirm: true }, context });
  const resumed = await handleRequest({ args: { action: "run_enabled_once" }, context }, { ...runtime, maxContinuousCycles: 1 });
  assert.equal(resumed.status, "ok", JSON.stringify(resumed));
  assert.equal(resumed.extra.background_worker.counts.items, 2);
  assert.deepEqual(clicked.slice(300), ["73000301", "73000302"]);
  records = await readRecords(root);
  assert.equal(records.length, 302);
  assert.equal(records.at(-1).global_sequence, 302);
});
