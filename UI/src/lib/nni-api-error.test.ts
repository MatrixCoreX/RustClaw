import assert from "node:assert/strict";
import test from "node:test";

import { formatNniApiError, formatNniErrorCause } from "./nni-api-error";

const zh = (value: string) => value;
const en = (_zh: string, value: string) => value;

const RATE_LIMIT_CODES = [
  "nni_rate_limit_explorer_read",
  "nni_rate_limit_network_read",
  "nni_rate_limit_bancor_public_read",
  "nni_rate_limit_bancor_private",
  "nni_rate_limit_asset_transfer",
  "nni_rate_limit_heartbeat",
  "nni_rate_limit_asset_authorization",
  "nni_rate_limit_reward_private",
  "nni_rate_limit_admin_read",
  "nni_rate_limit_admin_write",
  "nni_rate_limit_general",
] as const;

test("NNI rate-limit machine codes have Chinese and English presentation copy", () => {
  for (const code of RATE_LIMIT_CODES) {
    const chinese = formatNniApiError(code, zh);
    const english = formatNniApiError(code, en);
    assert.ok(chinese.length > 6, code);
    assert.ok(english.length > 12, code);
    assert.doesNotMatch(chinese, /nni_rate_limit/);
    assert.doesNotMatch(english, /nni_rate_limit/);
    assert.notEqual(chinese, english);
  }
});

test("NNI Bancor private rate limits explain the affected operation", () => {
  assert.equal(
    formatNniApiError("nni_rate_limit_bancor_private", zh),
    "账户与交易请求过于频繁，请稍后再试。",
  );
  assert.equal(
    formatNniApiError("nni_rate_limit_bancor_private", en),
    "Account and trading requests are too frequent. Try again shortly.",
  );
});

test("unknown NNI machine codes never leak into visible fallback text", () => {
  for (const code of ["nni_future_machine_failure", "upstream_unavailable", "HTTP:UPSTREAM_FAILED"]) {
    assert.doesNotMatch(formatNniApiError(code, zh), new RegExp(code));
    assert.doesNotMatch(formatNniApiError(code, zh, code), new RegExp(code));
    assert.match(formatNniApiError(code, en), /did not complete/i);
  }
});

test("human diagnostics remain visible while Error causes use the same formatter", () => {
  assert.equal(
    formatNniApiError("connection reset by peer", en),
    "connection reset by peer",
  );
  assert.equal(
    formatNniErrorCause(new Error("nni_rate_limit_network_read"), zh),
    "网络状态刷新过于频繁，请稍后再试。",
  );
});
