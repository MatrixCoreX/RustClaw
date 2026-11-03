// Test the actual compiled stylesheet. No dev server, provider calls or mutations.
import assert from "node:assert/strict";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../", import.meta.url));
const dist = process.env.UI_DIST_DIR || join(root, "dist");
const output = process.env.UI_BROWSER_ARTIFACTS || join(tmpdir(), `status-contrast-${Date.now()}`);
await mkdir(output, { recursive: true });
const html = await readFile(join(dist, "index.html"), "utf8");
const assets = [...html.matchAll(/href="(\/assets\/[A-Za-z0-9_.-]+\.css)"/g)].map(match => match[1]);
assert.ok(assets.length, "A compiled UI stylesheet is required");
const stylesheet = (await Promise.all(assets.map(asset => readFile(join(dist, asset), "utf8")))).join("\n");
const tokens = new Set();
for (const file of await readdir(join(root, "src"), { recursive: true })) {
  if (!file.endsWith(".tsx")) continue;
  const source = await readFile(join(root, "src", file), "utf8");
  for (const token of source.matchAll(/\btext-(?:emerald|amber|red|sky)-(?:100|200|300)(?:\/\d+)?\b/g)) tokens.add(token[0]);
}
assert.ok(tokens.size > 20, "Cover the real status classes, including opacity variants");
const fixtures = [...tokens].sort().map(token => ({ token, tone: token.split("-")[1] }));
fixtures.push({ token: "setup-status setup-status-done", tone: "emerald" },
  { token: "setup-status setup-status-attention", tone: "amber" });
const browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
const results = [];
try {
  for (const width of [1440, 390]) for (const theme of ["dark", "light"]) {
    const page = await browser.newPage({ viewport: { width, height: 900 }, serviceWorkers: "block" });
    try {
      await page.route("**/*", route => route.abort());
      await page.setContent(`<!doctype html><html data-theme="${theme}"><head><style>${stylesheet}</style></head>
        <body style="background:var(--theme-body-bg);padding:16px"><main class="theme-shell" style="background:var(--theme-card-strong);background-image:none;padding:16px">
        ${fixtures.map(({ token, tone }, index) => `<section class="bg-${tone}-500/10" style="margin:8px;padding:8px;background-image:none">
          <span data-status="${index}" class="${token}" style="font-size:12px">${token}</span></section>`).join("")}
        </main></body></html>`);
      const measurements = await page.locator("[data-status]").evaluateAll(elements => {
        const canvas = document.createElement("canvas");
        canvas.width = canvas.height = 1;
        const context = canvas.getContext("2d", { willReadFrequently: true });
        const rgba = color => {
          context.clearRect(0, 0, 1, 1);
          context.fillStyle = color;
          context.fillRect(0, 0, 1, 1);
          const [r, g, b, a] = context.getImageData(0, 0, 1, 1).data;
          return { r, g, b, a: a / 255 };
        };
        const over = (color, background) => ({ r: color.r * color.a + background.r * (1 - color.a),
          g: color.g * color.a + background.g * (1 - color.a),
          b: color.b * color.a + background.b * (1 - color.a), a: 1 });
        const luminance = color => [color.r, color.g, color.b].map(value => {
          const channel = value / 255;
          return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
        }).reduce((sum, value, index) => sum + value * [0.2126, 0.7152, 0.0722][index], 0);
        return elements.map(element => {
          const ancestors = [];
          for (let node = element; node; node = node.parentElement) ancestors.unshift(node);
          const background = ancestors.reduce((paint, node) => over(rgba(getComputedStyle(node).backgroundColor), paint),
            { r: 255, g: 255, b: 255, a: 1 });
          const style = getComputedStyle(element);
          const foreground = over(rgba(style.color), background);
          const a = luminance(foreground), b = luminance(background);
          return { token: element.className, color: style.color, contrast: (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05) };
        });
      });
      const failures = measurements.filter(row => row.contrast < 4.5);
      const setupDimensions = await page.locator(".setup-status").evaluateAll(elements => elements.map(element => {
        const { width, height } = element.getBoundingClientRect();
        return { width, height };
      }));
      results.push({ width, theme, measured: measurements.length, passed: failures.length === 0,
        failures, setup_dimensions: setupDimensions, minimum_contrast: Math.min(...measurements.map(row => row.contrast)) });
      await page.screenshot({ path: join(output, `${theme}-${width}.png`), fullPage: true });
    } finally { await page.close(); }
  }
  await writeFile(join(output, "report.json"), JSON.stringify({ results, real_backend_writes: 0, provider_calls: 0 }, null, 2));
  for (const width of [1440, 390]) assert.deepEqual(
    results.find(row => row.width === width && row.theme === "light").setup_dimensions,
    results.find(row => row.width === width && row.theme === "dark").setup_dimensions,
    "A theme-specific color correction must not change badge dimensions");
  console.log(JSON.stringify({ cases: results.length, measured_status_variants: fixtures.length,
    passed: results.filter(row => row.passed).length, failures: results.filter(row => !row.passed) }));
  assert.ok(results.every(row => row.passed), "Small status text must reach 4.5:1 contrast in both themes");
} finally { await browser.close(); }
