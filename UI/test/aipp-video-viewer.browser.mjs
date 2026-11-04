import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdir, readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";
import { guardFixtureRequests } from "./helpers/fixture-network.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../", import.meta.url));
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/aipp-video-viewer-tests";
await mkdir(output, { recursive: true });
// A real, tiny WebM exercises browser decoding without using private downloaded media.
const clip = execFileSync("ffmpeg", ["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=640x360:rate=12", "-t", "2", "-c:v", "libvpx", "-pix_fmt", "yuv420p", "-threads", "2", "-an", "-f", "webm", "pipe:1"]);
const server = await isolatedUiServer(root);
let browser;
try {
  const base = server.base;
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  const page = await browser.newPage({ acceptDownloads: true });
  await guardFixtureRequests(page, server.base);
  const errors = [], requests = [], nativeRequests = [];
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
    if (route.request().method() === "HEAD") {
      await route.fulfill({ status: failVideo ? 401 : 200, headers: { "content-type": "video/webm", "accept-ranges": "bytes", "content-length": String(clip.length) } }); return;
    }
    if (route.request().resourceType() === "media") {
      const headers = await route.request().allHeaders();
      nativeRequests.push({ url: url.href, headers });
      const range = /^bytes=(\d+)-(\d*)$/.exec(headers.range || "");
      const start = range ? Number(range[1]) : 0;
      const end = range?.[2] ? Math.min(Number(range[2]), clip.length - 1) : clip.length - 1;
      await route.fulfill({ status: range ? 206 : 200, headers: { "content-type": "video/webm", "accept-ranges": "bytes",
        ...(range ? { "content-range": `bytes ${start}-${end}/${clip.length}` } : {}),
      }, body: clip.subarray(start, end + 1) }); return;
    }
    if (delayVideo) await new Promise(resolve => setTimeout(resolve, 300));
    const corrupt = corruptOriginal && !url.searchParams.has("preview");
    await route.fulfill({ status: failVideo ? 503 : 200, contentType: "video/webm", body: failVideo ? "unavailable" : corrupt ? "invalid video" : clip });
  });
  const videoRequests = () => requests.filter(url => url.searchParams.get("preview") !== "poster");
  const posterRequests = () => requests.filter(url => url.searchParams.get("preview") === "poster" && url.pathname.includes("video-1"));
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
    await page.locator("article img").evaluate(image => image.decode());
    const loadedPosters = posterRequests().length;
    assert.equal(await page.locator("video").count(), 0, "the card must not contain a player");
    assert.equal(videoRequests().length, 0, "the list must fetch posters only");
    const open = page.getByRole("button", { name: lang === "zh" ? "展开视频" : "Expand video", exact: true });
    await open.focus(); await page.keyboard.press("Enter"); await ready(); await layout();
    assert.equal(posterRequests().length, loadedPosters, "opening a loaded thumbnail must reuse its poster bytes");
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
    await page.getByTitle(lang === "zh" ? "预览" : "Preview", { exact: true }).nth(0).click(); await ready();
    assert.equal(posterRequests().length, loadedPosters, "the file preview action also reuses the loaded poster");
    await page.keyboard.press("Escape"); await dialog.waitFor({ state: "detached" });
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
  await page.context().addCookies([{ name: "fixture-session", value: "browser-only", url: base, httpOnly: true, sameSite: "Strict" }]);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 }); requests.length = 0; nativeRequests.length = 0;
    await page.goto(`${base}/test/fixtures/aipp-video-viewer.html?lang=en&auth=webd`);
    await open.click(); await ready();
    assert.equal(await dialog.locator("video").evaluate(video => video.src.startsWith("blob:")), false);
    await dialog.locator("video").evaluate(async video => { await video.play(); window.nativeVideo = video; });
    await page.waitForFunction(() => window.nativeVideo.currentTime > 0);
    assert.ok(nativeRequests.some(request => request.headers.range?.startsWith("bytes=") && request.headers.cookie?.includes("fixture-session=browser-only")));
    assert.ok(videoRequests().every(url => !url.searchParams.has("preview")), "normal streaming must not trigger conversion");
    await page.keyboard.press("Escape"); await dialog.waitFor({ state: "detached" });
    assert.equal(await page.evaluate(() => window.nativeVideo.paused && !window.nativeVideo.hasAttribute("src")), true);
  }
  assert.deepEqual(errors, []);
  console.log("PASS desktop/mobile same-origin session: cookie-authenticated native Range playback and close cleanup without buffering");
  for (const auth of ["webd", "key"]) {
    requests.length = 0; nativeRequests.length = 0;
    await page.goto(`${base}/test/fixtures/aipp-video-viewer.html?lang=en&auth=${auth}&view=chat`);
    assert.equal(videoRequests().length, 0, "large chat videos remain manual");
    await page.getByRole("button", { name:"Load preview", exact:true }).click();
    await page.waitForFunction(() => document.querySelector("video")?.readyState >= 2);
    assert.equal(await page.locator("video").evaluate(video => video.src.startsWith("blob:")), auth !== "webd");
    assert.equal(videoRequests().some(url => url.searchParams.get("preview") === "browser"), auth !== "webd");
    await page.locator("video").evaluate(async video => { await video.play(); window.chatVideo = video; });
    await page.waitForFunction(() => window.chatVideo.currentTime > 0);
    await page.goto(`${base}/test/fixtures/aipp-video-viewer.html`);
  }
  assert.deepEqual(errors, []);
  console.log("PASS chat manual video: native session playback and unchanged authenticated key-mode compatible copy");
} finally {
  await browser?.close(); await server.close();
}
