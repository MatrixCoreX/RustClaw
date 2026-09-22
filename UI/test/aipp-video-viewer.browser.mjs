import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdir, readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../", import.meta.url));
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/aipp-video-viewer-tests";
await mkdir(output, { recursive: true });
// A real, tiny WebM exercises browser decoding without using private downloaded media.
const clip = execFileSync("ffmpeg", ["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=640x360:rate=12", "-t", "2", "-c:v", "libvpx", "-pix_fmt", "yuv420p", "-threads", "2", "-an", "-f", "webm", "pipe:1"]);
const server = await createServer({ root, server: { host: "127.0.0.1", port: 0, strictPort: true } });
let browser;
try {
  await server.listen();
  const base = `http://127.0.0.1:${server.httpServer.address().port}`;
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  const page = await browser.newPage({ acceptDownloads: true });
  const errors = [], requests = [];
  page.on("pageerror", error => errors.push(error.message));
  const poster = Buffer.from(await page.evaluate(() => {
    const canvas = document.createElement("canvas"); canvas.width = 640; canvas.height = 360;
    const context = canvas.getContext("2d"); context.fillStyle = "#397262"; context.fillRect(0, 0, 640, 360);
    return canvas.toDataURL().split(",")[1];
  }), "base64");
  let failVideo = false, failPoster = false, corruptOriginal = false, delayVideo = false;
  await page.route("**/v1/tasks/video-viewer-test/artifacts/**", async route => {
    const url = new URL(route.request().url()); requests.push(url);
    if (url.searchParams.get("preview") === "poster") {
      await route.fulfill({ status: failPoster ? 503 : 200, contentType: "image/png", body: failPoster ? "unavailable" : poster }); return;
    }
    if (delayVideo) await new Promise(resolve => setTimeout(resolve, 300));
    const corrupt = corruptOriginal && !url.searchParams.has("preview");
    await route.fulfill({ status: failVideo ? 503 : 200, contentType: "video/webm", body: failVideo ? "unavailable" : corrupt ? "invalid video" : clip });
  });
  const videoRequests = () => requests.filter(url => url.searchParams.get("preview") !== "poster");
  const dialog = page.getByRole("dialog");
  const ready = () => page.waitForFunction(() => {
    const video = document.querySelector("dialog video");
    return video instanceof HTMLVideoElement && video.readyState >= 2 && video.videoWidth > 0;
  });
  const layout = async () => {
    const viewport = page.viewportSize();
    for (const locator of [dialog, dialog.locator("video"), dialog.locator("header button")]) {
      const box = await locator.boundingBox();
      assert.ok(box && box.x >= 0 && box.y >= 0 && box.x + box.width <= viewport.width + 1 && box.y + box.height <= viewport.height + 1);
    }
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false);
  };
  for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
    const lang = theme === "light" ? "zh" : "en";
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    requests.length = 0;
    await page.goto(`${base}/test/fixtures/aipp-video-viewer.html?lang=${lang}&theme=${theme}`);
    await page.locator("article img").waitFor();
    assert.equal(await page.locator("video").count(), 0, "the card must not contain a player");
    assert.equal(videoRequests().length, 0, "the list must fetch posters only");
    const open = page.getByRole("button", { name: lang === "zh" ? "展开视频" : "Expand video", exact: true });
    await open.focus(); await page.keyboard.press("Enter"); await ready(); await layout();
    assert.equal(await dialog.locator("video").evaluate(video => video.controls && video.playsInline), true);
    await dialog.locator("video").evaluate(async video => {
      await video.play();
      window.testVideo = video;
    });
    await page.waitForFunction(() => window.testVideo.currentTime > 0);
    await page.screenshot({ path: `${output}/${width}-${theme}.png` });
    const downloadEvent = page.waitForEvent("download");
    await dialog.getByRole("button", { name: lang === "zh" ? "下载视频" : "Download video", exact: true }).click();
    const download = await downloadEvent;
    assert.equal(download.suggestedFilename(), "clip.webm"); assert.deepEqual(await readFile(await download.path()), clip);
    await page.keyboard.press("Escape"); await dialog.waitFor({ state: "detached" });
    assert.equal(await page.evaluate(() => window.testVideo.paused && !window.testVideo.hasAttribute("src")), true, "closing stops playback and releases the source");
    assert.equal(await open.evaluate(node => node === document.activeElement), true);
    assert.notEqual(await page.evaluate(() => document.body.style.overflow), "hidden");
    await page.getByTitle(lang === "zh" ? "预览" : "Preview", { exact: true }).nth(1).click(); await ready(); await layout();
    assert.ok(videoRequests().some(url => url.pathname.includes("video-2") && url.searchParams.get("preview") === "browser"), "large/unsupported-format video opens using the browser preview");
    await dialog.locator("header button").click(); await dialog.waitFor({ state: "detached" });
    console.log(`PASS ${width} ${theme}: poster-only list, keyboard expand, playback, download, stop, focus, secondary preview and responsive layout`);
  }
  await page.goto(`${base}/test/fixtures/aipp-video-viewer.html?lang=en`);
  const open = page.getByRole("button", { name: "Expand video", exact: true });
  corruptOriginal = true;
  await open.click(); await ready();
  assert.ok(videoRequests().some(url => url.pathname.includes("video-1") && url.searchParams.get("preview") === "browser"));
  await page.keyboard.press("Escape"); corruptOriginal = false;
  failVideo = true;
  await open.click(); await dialog.getByRole("alert").waitFor();
  failVideo = false;
  await dialog.getByRole("button", { name: "Retry", exact: true }).click(); await ready();
  await page.mouse.click(1, 1); await dialog.waitFor({ state: "detached" });
  failPoster = true;
  await page.reload(); await open.click(); await ready(); await page.keyboard.press("Escape");
  delayVideo = true;
  await open.click(); await dialog.getByRole("status").waitFor(); await page.keyboard.press("Escape");
  await dialog.waitFor({ state: "detached" }); await page.waitForTimeout(400);
  assert.equal(await page.locator("video").count(), 0);
  delayVideo = false;
  await open.click(); await ready();
  await page.keyboard.press("Tab");
  assert.equal(await dialog.evaluate(node => node.contains(document.activeElement)), true);
  await page.keyboard.press("Escape");
  assert.deepEqual(errors, []);
  console.log("PASS corrupt original fallback, failed preview retry, missing poster, backdrop close, pending-load cancellation and focus containment");
} finally {
  await browser?.close(); await server.close();
}
