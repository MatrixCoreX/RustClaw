import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { HostSystemSummaryPanel } from "../components/HostSystemSummaryPanel";
import { hostSystemSummaryFixture as fixture } from "../../test/fixtures/host-system";
import type { HostCapacitySummary, HostSystemSummary } from "../types/api";

const full: HostCapacitySummary = { total_bytes: 10_000, available_bytes: 100, available_ratio: 0.01 };
const unknown: HostCapacitySummary = { total_bytes: null, available_bytes: null, available_ratio: null };
function render(summary: HostSystemSummary) {
  let refreshes = 0;
  const html = renderToStaticMarkup(React.createElement(HostSystemSummaryPanel, {
    summary, loading: false, errorCode: null,
    t: (_zh, en) => en, onRefresh: () => { refreshes += 1; },
  }));
  assert.equal(refreshes, 0, "rendering a storage hint must not start work");
  assert.match(html, /lg:grid-cols-4/);
  return html;
}

test("a full system volume is visible even when the data volume has space", () => {
  const html = render({ ...fixture, system_storage: full });
  assert.match(html, /Data storage/);
  assert.match(html, /System volume:/);
  assert.match(html, /The system volume is nearly full/);
  assert.doesNotMatch(html, /The data volume is nearly full/);
  assert.match(html, /Memory available/);
  assert.doesNotMatch(html, /Resources available/);
});

test("data and system warnings remain independent, including unknown measurements", () => {
  assert.match(render({ ...fixture, storage: full, system_storage: fixture.storage }), /The data volume is nearly full/);
  assert.match(render({ ...fixture, storage: full, system_storage: full }), /Data and system volumes are nearly full/);
  const html = render({ ...fixture, system_storage: unknown });
  assert.doesNotMatch(html, /nearly full/);
  assert.match(html, /System volume: -- available/);
});

test("older servers do not acquire invented system-disk measurements or warnings", () => {
  const html = render(fixture);
  assert.match(html, /Data storage/);
  assert.doesNotMatch(html, /System volume:|nearly full/);
});
