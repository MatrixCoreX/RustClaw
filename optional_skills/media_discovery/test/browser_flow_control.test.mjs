import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import test from "node:test";
import {
  assertBrowserFlow, navigationAccessError, observePlatformBackpressure,
  retryAfterTimestamp, stopsCollection,
} from "../src/browser_flow_control.mjs";
import { assertNavigationResponse } from "../src/browser_search.mjs";
import { backgroundRetryDelayMs } from "../src/main.mjs";

const now = Date.parse("2026-09-11T04:00:00Z");
const response = ({ status = 429, url = "https://www.douyin.com/api/feed", type = "fetch", retry = "120" } = {}) => ({
  status: () => status, url: () => url, request: () => ({ resourceType: () => type }),
  headers: () => ({ "retry-after": retry, "content-type": "text/html" }),
});

test("Retry-After accepts seconds and HTTP dates, not invalid or overflowing values", () => {
  assert.equal(retryAfterTimestamp("120", now), "2026-09-11T04:02:00.000Z");
  assert.equal(retryAfterTimestamp(" 0 ", now), "2026-09-11T04:00:00.000Z");
  assert.equal(retryAfterTimestamp("Fri, 11 Sep 2026 06:00:00 GMT", now), "2026-09-11T06:00:00.000Z");
  assert.equal(retryAfterTimestamp("Fri, 11 Sep 2026 03:00:00 GMT", now), "2026-09-11T04:00:00.000Z");
  for (const value of [null, undefined, "", "-1", "0.5", "nonsense", "9".repeat(100)]) {
    assert.equal(retryAfterTimestamp(value, now), null);
  }
});

test("navigation errors keep a typed limit and the server's retry time", () => {
  const error = navigationAccessError(response(), "source_navigation", now);
  assert.equal(error.message, "rate_limited");
  assert.equal(error.status_code, 429);
  assert.equal(error.retry_after_at, "2026-09-11T04:02:00.000Z");
  assert.equal(error.discovery_stage, "source_navigation");
  assert.throws(() => assertNavigationResponse(response({ retry: "86400" }), "search_results_ready"), error =>
    error.message === "rate_limited" && Date.parse(error.retry_after_at) > Date.now() + 86_000_000);
  for (const status of [401, 403]) {
    const blocked = navigationAccessError(response({ status }), "detail_navigation", now);
    assert.equal(blocked.message, "challenge_required");
    assert.equal(blocked.retry_after_at, undefined);
  }
  assert.equal(navigationAccessError(response({ status: 200 }), "navigation", now), null);
});

test("the platform's document and API limits stop only that context and dispose cleanly", () => {
  for (const type of ["document", "xhr", "fetch"]) {
    const context = new EventEmitter();
    const other = new EventEmitter();
    const dispose = observePlatformBackpressure(context, "douyin", () => now);
    context.emit("response", response({ type }));
    assert.throws(() => assertBrowserFlow({ context: () => context }, "feed_scroll"), error =>
      error.message === "rate_limited" && error.retry_after_at === "2026-09-11T04:02:00.000Z"
      && error.discovery_stage === "feed_scroll");
    assert.doesNotThrow(() => assertBrowserFlow({ context: () => other }));
    dispose();
    assert.equal(context.listenerCount("response"), 0);
    assert.doesNotThrow(() => assertBrowserFlow({ context: () => context }));
  }
});

test("unrelated domains, subresources and non-limit responses do not block a platform", () => {
  const context = new EventEmitter();
  const dispose = observePlatformBackpressure(context, "douyin", () => now);
  for (const options of [
    { url: "https://telemetry.example.test/api" },
    { url: "https://douyin.com.example.test/api" },
    { url: "https://www.kuaishou.com/api" },
    { url: "https://www.douyin.com@untrusted.example.test/api" },
    { type: "image" }, { type: "stylesheet" }, { status: 200 }, { status: 404 },
  ]) context.emit("response", response(options));
  assert.doesNotThrow(() => assertBrowserFlow({ context: () => context }));
  context.emit("response", response({ url: "https://api.douyin.com/feed" }));
  assert.throws(() => assertBrowserFlow({ context: () => context }), { message: "rate_limited" });
  dispose();
});

test("subsequent responses cannot shorten the server's cooldown", () => {
  const context = new EventEmitter();
  const dispose = observePlatformBackpressure(context, "douyin", () => now);
  for (const retry of ["120", "7200", "60", undefined]) context.emit("response", response({ retry }));
  assert.throws(() => assertBrowserFlow({ context: () => context }), error =>
    error.retry_after_at === "2026-09-11T06:00:00.000Z");
  dispose();
});

test("server cooldown is a floor even beyond local backoff caps; invalid values keep normal backoff", () => {
  const configs = { douyin: { rest_min_seconds: 5, rest_max_seconds: 5 } };
  const delay = (value, failures = 1) => backgroundRetryDelayMs(configs, "rate_limited", failures, () => 0.5, value, now);
  assert.equal(delay("2026-09-12T04:00:00Z"), 86_400_000);
  assert.equal(delay("2026-09-11T04:02:00Z"), 600_000);
  assert.equal(delay("2026-09-11T03:00:00Z"), 600_000);
  assert.equal(delay("invalid"), 600_000);
  assert.equal(delay(null), 600_000);
  assert.equal(backgroundRetryDelayMs(configs, null, 1, () => 0.5, "2026-09-12T04:00:00Z", now), 5000);
});

test("access barriers stop the batch but a single broken post does not", () => {
  for (const code of ["login_required", "challenge_required", "network_access_restricted", "rate_limited",
    "collection_stopped", "interactive_verification_cancelled", "interactive_verification_timeout"]) {
    assert.equal(stopsCollection(new Error(code)), true);
  }
  assert.equal(stopsCollection(new Error("source_unavailable")), false);
  assert.equal(stopsCollection(new Error("media_element_not_found")), false);
});
