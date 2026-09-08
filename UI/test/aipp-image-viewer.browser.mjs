import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

// Use an installed Playwright module without adding browser dependencies to the UI bundle.
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../", import.meta.url));
const output = process.env.UI_BROWSER_ARTIFACTS || path.join(os.tmpdir(), "aipp-image-viewer-tests");
await mkdir(output, { recursive: true });
const server = await createServer({ root, server: { host: "127.0.0.1", port: 0, strictPort: true } });
let browser;
try {
  await server.listen();
  const base = `http://127.0.0.1:${server.httpServer.address().port}`;
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  const context = await browser.newContext({ acceptDownloads: true });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const downloads = [];
  page.on("download", (download) => downloads.push(download));
  // Raster fixtures exercise real image decoding, aspect ratio, and pixel visibility.
  await page.goto("about:blank");
  const makePng = (width, height) => page.evaluate(({ width, height }) => {
    const canvas = document.createElement("canvas");
    canvas.width = width; canvas.height = height;
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "#d9eadb"; ctx.fillRect(0, 0, width, height);
    ctx.fillStyle = "#397262"; ctx.fillRect(width * .1, height * .1, width * .8, height * .6);
    ctx.fillStyle = "#e9ad5a"; ctx.fillRect(width * .2, height * .25, width * .6, height * .3);
    ctx.fillStyle = "#252a30"; ctx.font = `${Math.floor(width / 12)}px sans-serif`;
    ctx.fillText("IMAGE PREVIEW", width * .1, height * .85);
    return canvas.toDataURL("image/png").split(",")[1];
  }, { width, height });
  const portrait = Buffer.from(await makePng(600, 900), "base64");
  const landscape = Buffer.from(await makePng(1600, 700), "base64");
  let failPreview = false, failDownload = false, corruptPreview = false, delayPreview = false;
  const requests = [];
  await page.route("**/v1/aipps/**/preview", async (route) => {
    requests.push(new URL(route.request().url()).pathname);
    await route.fulfill({ contentType: "image/png", body: portrait });
  });
  await page.route("**/fixture/**", async (route) => {
    const url = new URL(route.request().url()).pathname;
    requests.push(url);
    if ((url.endsWith("download") && failDownload) || (url.endsWith("preview") && failPreview)) {
      await route.fulfill({ status: 403, body: "denied" }); return;
    }
    if (url.endsWith("preview") && delayPreview) await new Promise((resolve) => setTimeout(resolve, 300));
    await route.fulfill({ contentType: url.includes("text") ? "text/plain" : "image/png", body: url.includes("text") ? "contents" : corruptPreview && url.endsWith("preview") ? "invalid image" : url.includes("landscape") ? landscape : portrait });
  });
  const dialog = page.getByRole("dialog");
  const readyImage = () => page.waitForFunction(() => {
    const image = document.querySelector("dialog img");
    return image instanceof HTMLImageElement && image.complete && image.naturalWidth > 0;
  });
  const checkLayout = async () => {
    const box = await dialog.boundingBox();
    const button = await dialog.locator("footer button").boundingBox();
    const viewport = page.viewportSize();
    assert.ok(box.x >= 0 && box.y >= 0 && box.x + box.width <= viewport.width + 1 && box.y + box.height <= viewport.height + 1);
    assert.ok(button.x > box.x + box.width / 2 && button.y > box.y + box.height / 2);
    assert.ok(button.x + button.width <= box.x + box.width && button.y + button.height <= box.y + box.height);
    assert.equal(await dialog.evaluate((node) => node.scrollHeight > node.clientHeight), false);
  };
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    await page.setViewportSize(viewport);
    for (const theme of ["light", "dark"]) {
      const lang = theme === "light" ? "zh" : "en";
      await page.goto(`${base}/test/fixtures/aipp-image-viewer.html?theme=${theme}&lang=${lang}`);
      for (const kind of ["collection", "cover", "activity"]) {
        const thumbnail = page.getByTestId(kind).locator("button[aria-label]").first();
        await page.getByTestId(kind).locator("img").first().waitFor();
        const downloadsBefore = downloads.length;
        const requestCount = requests.length;
        await thumbnail.click();
        await dialog.waitFor(); await readyImage(); await checkLayout();
        assert.equal(downloads.length, downloadsBefore, "opening must not download");
        assert.equal(requests.length, requestCount, "reuse the authenticated thumbnail blob");
        if (kind === "activity") await page.screenshot({ path: path.join(output, `${viewport.width}-${theme}.png`) });
        const downloadEvent = page.waitForEvent("download");
        await dialog.locator("footer button").click();
        const download = await downloadEvent;
        assert.equal(download.suggestedFilename(), kind === "activity" ? "portrait.png" : `media-${kind === "cover" ? "000000000002" : "000000000001"}.png`);
        assert.deepEqual(await readFile(await download.path()), portrait);
        assert.ok(requests.at(-1).endsWith(kind === "activity" ? "/download" : "/preview"));
        await page.keyboard.press("Escape");
        await dialog.waitFor({ state: "detached" });
        assert.equal(await thumbnail.evaluate((node) => node === document.activeElement), true);
        assert.notEqual(await page.evaluate(() => document.body.style.overflow), "hidden");
        console.log(`PASS ${kind} ${viewport.width} ${theme}: enlarge, authenticated download, layout, Escape`);
      }
    }
  }
  await page.goto(`${base}/test/fixtures/aipp-image-viewer.html?lang=en`);
  const openLandscape = () => page.getByTestId("activity").getByTitle("Preview", { exact: true }).nth(1).click();
  failPreview = true;
  await openLandscape();
  await dialog.getByRole("alert").waitFor();
  failPreview = false;
  await dialog.getByRole("button", { name: "Retry", exact: true }).click();
  await readyImage(); await checkLayout();
  assert.equal(await dialog.locator("img").evaluate((img) => img.naturalWidth), 1600);
  failDownload = true;
  await dialog.locator("footer button").click();
  await dialog.getByText("Image download failed. Try again.", { exact: true }).waitFor();
  await checkLayout();
  failDownload = false;
  const retryDownload = page.waitForEvent("download");
  await dialog.locator("footer button").click();
  assert.deepEqual(await readFile(await (await retryDownload).path()), landscape);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  console.log("PASS image artifact preview, long filename, preview/download failure and retry");
  corruptPreview = true;
  await openLandscape();
  await dialog.getByRole("alert").waitFor();
  corruptPreview = false;
  await dialog.getByRole("button", { name: "Retry", exact: true }).click(); await readyImage();
  await page.mouse.click(1, 1); await dialog.waitFor({ state: "detached" });
  console.log("PASS corrupt image retry and backdrop close");
  delayPreview = true;
  await openLandscape();
  await page.keyboard.press("Escape"); await dialog.waitFor({ state: "detached" });
  await page.waitForTimeout(400); delayPreview = false;
  await openLandscape(); await readyImage();
  await page.keyboard.press("Tab");
  assert.equal(await dialog.evaluate((node) => node.contains(document.activeElement)), true);
  await page.keyboard.press("Escape");
  const popupEvent = page.waitForEvent("popup");
  await page.getByTestId("activity").getByTitle("Preview", { exact: true }).nth(2).click();
  const popup = await popupEvent; await popup.close();
  assert.equal(await dialog.count(), 0);
  console.log("PASS pending request cancellation, keyboard focus, unchanged text preview");
  assert.deepEqual(errors, []);
  console.log(`PASS browser suite; screenshots: ${output}`);
  await context.close();
} finally {
  await browser?.close();
  await server.close();
}
