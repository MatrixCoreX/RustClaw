import assert from "node:assert/strict";
import test from "node:test";
import { PageRequestLifecycle } from "./page-request-lifecycle";

test("slow foreground load cannot be superseded by polling", () => {
  const requests = new PageRequestLifecycle();
  const first = requests.begin(false)!;
  assert.equal(requests.begin(true), null);
  assert.equal(requests.isCurrent(first), true);
  assert.equal(requests.finish(first), true);
  const refresh = requests.begin(true)!;
  assert.equal(requests.isCurrent(refresh), true);
  assert.equal(requests.finish(refresh), false);
});

test("new foreground owns loading until it completes, even on failure", () => {
  const requests = new PageRequestLifecycle();
  const old = requests.begin(false)!;
  const current = requests.begin(false)!;
  assert.equal(requests.isCurrent(old), false);
  assert.equal(requests.finish(old), false);
  assert.equal(requests.begin(true), null);
  assert.equal(requests.finish(current), true);
  assert.notEqual(requests.begin(true), null);
});

test("page change invalidates pending results and loading ownership", () => {
  const requests = new PageRequestLifecycle();
  const old = requests.begin(false)!;
  requests.invalidate();
  assert.equal(requests.isCurrent(old), false);
  assert.equal(requests.finish(old), false);
  assert.notEqual(requests.begin(true), null);
});

test("foreground inspection is read-only and follows finish and invalidation", () => {
  const requests = new PageRequestLifecycle();
  assert.equal(requests.currentRequestId(), 0);
  assert.equal(requests.hasForegroundRequest(), false);
  const old = requests.begin(false)!;
  const current = requests.begin(false)!;
  assert.equal(requests.currentRequestId(), current);
  assert.equal(requests.hasForegroundRequest(), true);
  assert.equal(requests.finish(old), false);
  assert.equal(requests.hasForegroundRequest(), true);
  assert.equal(requests.finish(current), true);
  assert.equal(requests.hasForegroundRequest(), false);
  requests.begin(true);
  assert.equal(requests.hasForegroundRequest(), false);
  requests.begin(false);
  requests.invalidate();
  assert.equal(requests.hasForegroundRequest(), false);
  assert.equal(requests.isCurrent(current), false);
  assert.notEqual(requests.currentRequestId(), current);
});
