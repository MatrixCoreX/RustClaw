import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("repository package confines the Node build source to this skill", async () => {
  const manifest = await readFile(
    new URL("../skill.toml", import.meta.url),
    "utf8",
  );

  assert.match(
    manifest,
    /^source_root = "optional_skills\/media_discovery"$/m,
  );
  assert.doesNotMatch(manifest, /^source_root = "\."$/m);
  assert.match(manifest, /^progress_frames = true$/m);
  assert.doesNotMatch(manifest, /tesseract/u);
  assert.match(manifest, /^llm_gateway = false$/m);
});

test("collector source has no OCR or model-review execution path", async () => {
  const [browserSource, mainSource] = await Promise.all([
    readFile(new URL("../src/browser.mjs", import.meta.url), "utf8"),
    readFile(new URL("../src/main.mjs", import.meta.url), "utf8"),
  ]);
  assert.doesNotMatch(browserSource, /recognizeScreenshot|recognized_text|raw_recognized_text/u);
  assert.doesNotMatch(mainSource, /recognition_mode|AGENT_INTERNAL_LLM/u);
});
