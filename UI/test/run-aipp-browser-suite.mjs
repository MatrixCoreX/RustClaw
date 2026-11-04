// Discover isolated AiAPP fixtures, retain every result, and fail on any failure.
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, open, readFile, readdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export async function runAippBrowserSuite({
  testDirectory = fileURLToPath(new URL("./", import.meta.url)),
  outputDirectory = process.env.UI_BROWSER_ARTIFACTS || join(tmpdir(), `aipp-browser-suite-${Date.now()}`),
} = {}) {
  await mkdir(outputDirectory, { recursive: true, mode: 0o700 });
  const tests = (await readdir(testDirectory, { withFileTypes: true }))
    .filter(item => item.isFile() && /^aipp-[a-z0-9-]+\.browser\.mjs$/.test(item.name))
    .map(item => item.name).sort();
  const results = [];
  for (const name of tests) {
    const output = await mkdtemp(join(outputDirectory, basename(name, ".browser.mjs")+"-"));
    const logPath = join(output, "execution.log");
    const log = await open(logPath, "wx", 0o600);
    let child;
    try {
      console.log(`Running isolated fixture: ${name}`);
      child = spawnSync(process.execPath, [join(testDirectory, name)], {
        env: { ...process.env, UI_BROWSER_ARTIFACTS: output }, stdio: ["ignore", log.fd, log.fd],
      });
    } finally { await log.close(); }
    process.stdout.write(await readFile(logPath, "utf8"));
    results.push({ test: name, passed: child.status === 0 && !child.error,
      exit_code: child.status, signal: child.signal, launch_failed: Boolean(child.error), output_directory: output });
  }
  const report = { schema_version: 1, passed: tests.length > 0 && results.every(row => row.passed),
    discovered: tests.length, results };
  await writeFile(join(outputDirectory, "report.json"), JSON.stringify(report, null, 2), { mode: 0o600 });
  return report;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const report = await runAippBrowserSuite();
  console.log(JSON.stringify(report));
  process.exitCode = report.passed ? 0 : 1;
}
