import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";
import { guardFixtureRequests } from "./helpers/fixture-network.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/aipp-artifact-download";
await mkdir(output, { recursive:true });
const server = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
let browser;
try {
  browser = await chromium.launch({ executablePath:process.env.BROWSER_EXECUTABLE || undefined, headless:true });
  for (const view of ["chat", "aipp"]) for (const width of [1440,390]) for (const theme of ["dark","light"]) {
    const context = await browser.newContext({ viewport:{width,height:900}, acceptDownloads:true });
    let held;
    try {
      const page = await context.newPage(); await guardFixtureRequests(page, server.base);
      const errors = [], downloads = [];
      page.on("pageerror", error => errors.push(error.message)); page.on("download", download => downloads.push(download));
      await page.addInitScript(() => {
        window.fixtureDownloads = [];
        const fetcher = window.fetch;
        window.fetch = function(input, init) {
          const url = typeof input === "string" ? input : input instanceof Request ? input.url : String(input);
          if (url.includes("/artifacts/file/content")) {
            const signal = init?.signal;
            const record = {hasSignal:!!signal, aborted:!!signal?.aborted}; window.fixtureDownloads.push(record);
            signal?.addEventListener("abort", () => { record.aborted = true; });
          }
          return fetcher.call(this,input,init);
        };
      });
      let requests = 0, fail = true;
      const bytes = Buffer.from("artifact bytes\n");
      await page.route("**/v1/tasks/download-test/artifacts/file/content", async route => {
        requests++;
        if (requests === 1) { held = route; return; }
        await route.fulfill({status:fail ? 503 : 200, contentType:"text/plain", body:fail ? "unavailable" : bytes});
      });
      await page.goto(`${server.base}/test/fixtures/aipp-artifact-download.html?view=${view}&theme=${theme}`);
      const download = () => view === "chat" ? page.getByRole("button", {name:"Download report.txt",exact:true}) : page.getByTitle("Download", {exact:true});
      await download().evaluate(button => { button.click(); button.click(); });
      await page.waitForFunction(() => window.fixtureDownloads.length > 0);
      assert.equal(requests,1,"overlapping callbacks must not launch duplicate transfers");
      await page.getByRole("button", {name:"Remove card",exact:true}).click();
      await page.waitForFunction(() => window.fixtureDownloads.every(read => read.hasSignal && read.aborted));
      await held.fulfill({contentType:"text/plain",body:bytes}).catch(() => {}); held = undefined;
      await page.getByRole("button", {name:"Show card",exact:true}).click();
      await download().click();
      await page.getByText(view === "chat" ? "Download failed. Try again." : "Could not read the file. Try again.", {exact:true}).waitFor();
      assert.equal(downloads.length,0,"cancelled and failed transfers cannot save obsolete files");
      fail = false; const next = page.waitForEvent("download"); await download().click();
      const saved = await next;
      assert.equal(saved.suggestedFilename(),"report.txt"); assert.deepEqual(await readFile(await saved.path()),bytes);
      assert.equal(downloads.length,1);
      assert.deepEqual(errors,[]);
      await page.screenshot({path:`${output}/${view}-${width}-${theme}.png`});
      console.log(`PASS ${view} ${width} ${theme}: one active transfer, removal abort, no late save, failure retry, exact downloaded bytes`);
    } finally { await held?.abort().catch(() => {}); await context.close(); }
  }
} finally { await browser?.close(); await server.close(); }
