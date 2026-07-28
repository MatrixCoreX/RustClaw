import fs from "node:fs";
import readline from "node:readline";

const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
lines.once("line", (line) => {
  const request = JSON.parse(line);
  const requestId = request.request_id;
  const args = request.args || {};
  const action = args.action || "calculate";
  const ok = (extra) => ({ request_id: requestId, status: "ok", text: "", error_text: null, extra });
  let output;
  if (action === "calculate") output = ok({ result: { value: (args.a || 0) + (args.b || 0) } });
  else if (action === "validation_error") output = { request_id: requestId, status: "error", text: "", error_text: "invalid fixture input", extra: { error_code: "fixture_invalid", message_key: "fixture.invalid" } };
  else if (action === "artifact") { fs.writeFileSync(args.artifact_path, "reference-artifact\n"); output = ok({ artifact: { created: true } }); }
  else if (action === "waiting") output = ok({ continuation: { state: "waiting", poll_after_ms: 10 } });
  else if (action === "needs_user") output = ok({ continuation: { state: "needs_user", required_fields: ["confirmation"] } });
  else if (action === "timeout") { setTimeout(() => process.stdout.write(`${JSON.stringify(ok({}))}\n`), 5000); return; }
  else if (action === "malformed") { process.stdout.write("{not-json\n"); return; }
  else if (action === "multiple") { process.stdout.write(`${JSON.stringify(ok({}))}\n${JSON.stringify(ok({}))}\n`); return; }
  else if (action === "oversized") { process.stdout.write(`${"x".repeat(1024 * 1024 + 1)}\n`); return; }
  else if (action === "stderr") { process.stderr.write("reference diagnostic\n"); output = ok({ diagnostic_preserved: true }); }
  else output = ok({});
  process.stdout.write(`${JSON.stringify(output)}\n`);
  lines.close();
});
