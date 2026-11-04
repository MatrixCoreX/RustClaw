import assert from "node:assert/strict";
import { mkdtemp, readFile, mkdir, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { runAippBrowserSuite } from "./run-aipp-browser-suite.mjs";
import { guardFixtureRequests } from "./helpers/fixture-network.mjs";

test("suite retains every failure, continues later tests, and excludes live/non-file entries", async () => {
  const root = await mkdtemp(join(tmpdir(), "aipp-suite-unit-"));
  try {
    const testDirectory = join(root, "tests"), outputDirectory = join(root, "evidence");
    await mkdir(testDirectory);
    await writeFile(join(testDirectory, "aipp-a-fail.browser.mjs"), 'console.error("expected fixture failure");process.exitCode=7;');
    await writeFile(join(testDirectory, "aipp-b-pass.browser.mjs"), 'console.log("later fixture ran");');
    await writeFile(join(testDirectory, "aipp-provider.live.browser.mjs"), 'throw new Error("never run a live test");');
    await mkdir(join(testDirectory, "aipp-not-file.browser.mjs"));
    await symlink(join(testDirectory, "aipp-b-pass.browser.mjs"), join(testDirectory, "aipp-link.browser.mjs"));
    const report = await runAippBrowserSuite({ testDirectory, outputDirectory });
    assert.equal(report.passed, false);
    assert.equal(report.discovered, 2);
    assert.equal(report.results[0].exit_code, 7);
    assert.equal(report.results[1].passed, true);
    assert.match(await readFile(join(report.results[1].output_directory, "execution.log"), "utf8"), /later fixture ran/);
    assert.deepEqual(JSON.parse(await readFile(join(outputDirectory, "report.json"))), report);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("an empty fixture suite cannot pass", async () => {
  const root = await mkdtemp(join(tmpdir(), "aipp-suite-empty-unit-"));
  try {
    const report = await runAippBrowserSuite({ testDirectory: root, outputDirectory: join(root, "evidence") });
    assert.equal(report.passed, false);
    assert.equal(report.discovered, 0);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("unmocked API, provider and other loopback traffic is blocked; fixture assets remain usable", async () => {
  let handler;
  await guardFixtureRequests({ context: () => ({ route: async (pattern, callback) => {
    assert.equal(pattern, "**/*"); handler = callback;
  } }) }, "http://127.0.0.1:43123");
  for (const [url, expected] of [
    ["http://127.0.0.1:43123/test/fixtures/aipp.html", "continue"],
    ["http://127.0.0.1:43123/src/component.tsx?t=123", "continue"],
    ["http://127.0.0.1:43123/v1/aipps", "abort"],
    ["http://127.0.0.1:43123/webd/session", "abort"],
    ["http://127.0.0.1:8787/v1/tasks", "abort"],
    ["https://provider.example/v1/messages", "abort"],
  ]) {
    let decision;
    await handler({ request: () => ({ url: () => url }), continue: () => { decision = "continue"; }, abort: () => { decision = "abort"; } });
    assert.equal(decision, expected, url);
  }
  await assert.rejects(guardFixtureRequests({}, "https://provider.example"), /ephemeral_loopback/);
});
