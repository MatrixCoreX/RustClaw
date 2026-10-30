import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import ts from "typescript";

const uiRoot = fileURLToPath(new URL("../../", import.meta.url));
const config = JSON.parse(readFileSync(join(uiRoot, "tsconfig.json"), "utf8"));
function inputFiles(root: string) {
  const parsed = ts.parseJsonConfigFileContent(config, ts.sys, root);
  assert.deepEqual(parsed.errors, []);
  return parsed.fileNames.map(file => relative(root, file).split(sep).join("/"));
}

test("TypeScript excludes generated output without excluding source, tests or build helpers", () => {
  const root = mkdtempSync(join(tmpdir(), "agent-typescript-inputs-"));
  try {
    const files = [
      "src/application.tsx", "test/runtime.test.ts", "test/fixtures/runtime.tsx",
      "vite.config.ts", "build-inputs.mjs", "dist/assets/generated.js",
      "node_modules/fixture/index.js", "bower_components/fixture/index.js",
      "jspm_packages/fixture/index.js",
    ];
    for (const file of files) {
      const target = join(root, file);
      mkdirSync(dirname(target), { recursive: true });
      writeFileSync(target, "export const fixture = 1;\n");
    }
    assert.deepEqual(inputFiles(root).sort(), files.slice(0, 5).sort());
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("actual UI type-check inputs retain application and regression code, not deployed assets", () => {
  const files = inputFiles(uiRoot);
  for (const file of ["src/App.tsx", "src/hooks/useSystemRuntime.ts", "src/hooks/useSystemRuntime.test.ts",
    "test/fixtures/system-host-ownership.tsx", "vite.config.ts", "build-inputs.mjs"]) {
    assert.ok(files.includes(file), file);
  }
  assert.equal(files.some(file => file.startsWith("dist/")), false);
});
