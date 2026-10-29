import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

test("React web types are explicit locked developer dependencies", () => {
  const manifest = JSON.parse(readFileSync(new URL("../../package.json", import.meta.url), "utf8"));
  const lock = JSON.parse(readFileSync(new URL("../../package-lock.json", import.meta.url), "utf8"));
  for (const name of ["@types/react", "@types/react-dom"]) {
    assert.equal(typeof manifest.devDependencies[name], "string");
    assert.equal(lock.packages[""].devDependencies[name], manifest.devDependencies[name]);
    assert.ok(lock.packages[`node_modules/${name}`].version);
  }
});
