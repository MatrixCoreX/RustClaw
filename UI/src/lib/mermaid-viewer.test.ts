import assert from "node:assert/strict";
import test from "node:test";

import { fitDiagramScale, scaledDiagramSize } from "./mermaid-viewer";

test("fit scale keeps small diagrams at natural size", () => {
  assert.equal(fitDiagramScale(900, 600), 1);
});

test("fit scale bounds oversized diagrams", () => {
  assert.equal(fitDiagramScale(600, 1200), 0.5);
  assert.equal(fitDiagramScale(100, 1000), 0.1);
});

test("scaled canvas reserves the transformed diagram dimensions", () => {
  assert.deepEqual(scaledDiagramSize({ width: 640, height: 480 }, 1.25), {
    width: 800,
    height: 600,
  });
});

test("invalid scale falls back to natural dimensions", () => {
  assert.deepEqual(scaledDiagramSize({ width: 640, height: 480 }, Number.NaN), {
    width: 640,
    height: 480,
  });
});
