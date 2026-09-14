import assert from "node:assert/strict";
import test from "node:test";
import { unzipSync } from "fflate";
import { downloadAippGallery, fetchAippImage, imageDownloadName } from "./aipp-image-download";

const image = (n: number) => ({ title: `Image ${n}`, filename: "../../same.png", previewUrl: `/images/${n}`, downloadUrl: `/images/${n}` });
const response = (body = "picture", status = 200, mime = "image/jpeg") => new Response(body, { status, headers: { "content-type": mime } });
test("image filenames are flat, safe and match actual MIME", () => {
  assert.equal(imageDownloadName("../../bad:*.png", "image/jpeg"), "bad__.jpg");
  assert.equal(imageDownloadName("..\\.png", "image/webp"), "image.webp");
});
test("authenticated local preview is fetched, with one transient retry", async () => {
  let requests = 0;
  const blob = await fetchAippImage(async () => response("picture", ++requests === 1 ? 503 : 200), "/preview", new AbortController().signal);
  assert.equal(requests, 2); assert.equal(blob.type, "image/jpeg"); assert.equal(await blob.text(), "picture");
});
test("authorization failures do not retry; external URLs never reach authenticated fetch", async () => {
  let requests = 0;
  const fetcher = async () => { requests++; return response("denied", 403); };
  await assert.rejects(fetchAippImage(fetcher, "/preview", new AbortController().signal));
  assert.equal(requests, 1);
  for (const url of ["https://elsewhere.test/a", "//elsewhere.test/a", "/\\elsewhere.test/a"]) await assert.rejects(fetchAippImage(fetcher, url, new AbortController().signal));
  assert.equal(requests, 1);
});
test("rejects HTML, empty data and oversized chunked images", async () => {
  for (const [body, mime] of [["html", "text/html"], ["", "image/png"], ["x".repeat(20), "image/png"]]) {
    await assert.rejects(fetchAippImage(async () => response(body, 200, mime), "/preview", new AbortController().signal, 10));
  }
});
test("download all produces an ordered ZIP with unique filenames and original bytes", async () => {
  const requests: string[] = [], progress: number[] = [];
  const zip = await downloadAippGallery([image(1), image(2), image(3)], async (path) => { requests.push(path); return response(path); }, new AbortController().signal, (n) => progress.push(n));
  assert.deepEqual(requests, ["/images/1", "/images/2", "/images/3"]);
  assert.deepEqual(progress, [1, 2, 3]);
  const files = unzipSync(new Uint8Array(await zip.arrayBuffer()));
  assert.deepEqual(Object.keys(files), ["001-same.jpg", "002-same.jpg", "003-same.jpg"]);
  assert.equal(new TextDecoder().decode(files["002-same.jpg"]), "/images/2");
});
test("one missing image rejects the entire ZIP rather than silently skipping it", async () => {
  const progress: number[] = [];
  await assert.rejects(downloadAippGallery([image(1), image(2), image(3)], async (path) => response(path, path.endsWith("2") ? 404 : 200), new AbortController().signal, (n) => progress.push(n)));
  assert.deepEqual(progress, [1]);
});
test("cancel stops gallery requests before the next image", async () => {
  const controller = new AbortController(); let requests = 0;
  await assert.rejects(downloadAippGallery([image(1), image(2)], async () => { requests++; return response(); }, controller.signal, () => controller.abort()));
  assert.equal(requests, 1);
});
test("empty and unbounded galleries are rejected before network requests", async () => {
  const unexpected = async () => { throw new Error("must not fetch"); };
  for (const entries of [[], Array.from({ length: 1001 }, (_, i) => image(i))]) await assert.rejects(downloadAippGallery(entries, unexpected, new AbortController().signal, () => {}), /count_invalid/);
});
