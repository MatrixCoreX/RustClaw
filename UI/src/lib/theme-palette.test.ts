import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";

const css = readFileSync(new URL("../index.css", import.meta.url), "utf8");
const lightTheme = css.match(/:root\[data-theme="light"\]\s*\{([\s\S]*?)\n\}/)?.[1] ?? "";

function themeValue(name: string): string {
  const value = lightTheme.match(new RegExp(`--theme-${name}:\\s*([^;]+);`, "i"))?.[1]?.trim();
  assert.ok(value, `missing light-theme color --theme-${name}`);
  return value;
}

function themeHex(name: string): string {
  const value = themeValue(name);
  assert.match(value, /^#[0-9a-f]{6}$/i, `--theme-${name} must be a six-digit hex color`);
  return value;
}

function colorChannels(value: string): number[] {
  if (value.startsWith("#")) {
    return [1, 3, 5].map((offset) => Number.parseInt(value.slice(offset, offset + 2), 16));
  }
  const channels = value.match(/^rgba?\(\s*(\d+),\s*(\d+),\s*(\d+)/i);
  assert.ok(channels, `unsupported color format: ${value}`);
  return channels.slice(1, 4).map(Number);
}

function rgbaAlpha(name: string): number {
  const match = themeValue(name).match(/^rgba\([^,]+,[^,]+,[^,]+,\s*([0-9.]+)\)$/i);
  assert.ok(match, `--theme-${name} must use rgba`);
  return Number(match[1]);
}

function relativeLuminance(hex: string): number {
  const channels = [1, 3, 5].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16) / 255);
  const linear = channels.map((channel) =>
    channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4,
  );
  return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
}

function contrastRatio(first: string, second: string): number {
  const lighter = Math.max(relativeLuminance(first), relativeLuminance(second));
  const darker = Math.min(relativeLuminance(first), relativeLuminance(second));
  return (lighter + 0.05) / (darker + 0.05);
}

function tsxSourceFiles(directory: URL): URL[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    if (entry.isDirectory()) {
      return tsxSourceFiles(new URL(`${entry.name}/`, directory));
    }
    return entry.isFile() && entry.name.endsWith(".tsx")
      ? [new URL(entry.name, directory)]
      : [];
  });
}

test("light theme stays low-glare while preserving readable contrast", () => {
  const background = themeHex("body-bg");
  const text = themeHex("body-text");

  assert.ok(relativeLuminance(background) <= 0.8, "light background became too bright");
  assert.ok(contrastRatio(text, background) >= 7, "body text must keep enhanced contrast");
});

test("every light-theme surface uses neutral gray instead of pure white", () => {
  const surfaceNames = [
    "body-bg",
    "shell-start",
    "shell-end",
    "header-bg",
    "card",
    "card-strong",
    "input-bg",
    "code-bg",
    "table-head",
    "sidebar-bg",
    "sidebar-item-bg",
    "sidebar-item-hover",
  ];

  for (const name of surfaceNames) {
    const value = themeValue(name);
    assert.doesNotMatch(value, /#fff(?:fff)?\b|rgba?\(\s*255\s*,\s*255\s*,\s*255/i);
    const channels = colorChannels(value);
    assert.ok(Math.max(...channels) - Math.min(...channels) <= 8, `--theme-${name} is not neutral gray`);
  }
});

test("light containers and controls keep clearly visible borders", () => {
  assert.ok(rgbaAlpha("border") >= 0.3);
  assert.ok(rgbaAlpha("border-strong") >= 0.4);
  assert.ok(rgbaAlpha("secondary-btn-border") >= 0.4);
  for (const opacity of ["5", "8", "10", "12", "15", "20", "25"]) {
    assert.ok(css.includes(`.border-white\\/${opacity}`), `missing light override for border-white/${opacity}`);
  }
});

test("light theme primary buttons keep accessible white-label contrast", () => {
  assert.ok(contrastRatio("#ffffff", themeHex("primary-btn-start")) >= 4.5);
  assert.ok(contrastRatio("#ffffff", themeHex("primary-btn-end")) >= 4.5);
  assert.match(
    css,
    /:root\[data-theme="light"\]\s+\.theme-accent-btn\s*\{[^}]*color:\s*#ffffff\s*!important;/s,
  );
});

test("every text-white opacity used by the UI has a readable light-theme override", () => {
  const opacities = new Set<string>();
  for (const file of tsxSourceFiles(new URL("../", import.meta.url))) {
    const source = readFileSync(file, "utf8");
    for (const match of source.matchAll(/(?<![:\w-])text-white\/(\d+)/g)) {
      opacities.add(match[1]);
    }
  }

  for (const opacity of opacities) {
    assert.ok(
      css.includes(`.text-white\\/${opacity}`),
      `missing readable light-theme override for text-white/${opacity}`,
    );
  }
  assert.ok(css.includes(".placeholder\\:text-white\\/35::placeholder"));
});
