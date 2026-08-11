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
});
