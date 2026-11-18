import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { MemoryImportControls } from "../components/MemoryImportControls";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
test("memory import controls retain explicit resume and cancellation while the same file is pending", async () => {
  let resumed = 0, cancelled = 0;
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(<MemoryImportControls t={zh => zh}
    progress={{ phase: "paused", received: 5, total: 10 }} onImport={async () => {}}
    onResume={() => { resumed++; }} onCancel={() => { cancelled++; }} />); });
  try {
    assert.equal(renderer.root.findByType("input").props.disabled, true);
    const buttons = renderer.root.findAllByType("button");
    assert.equal(buttons.length, 2);
    await act(async () => { buttons[0].props.onClick(); buttons[1].props.onClick(); });
    assert.equal(resumed, 1); assert.equal(cancelled, 1);
    assert.equal(renderer.root.findAllByProps({ role: "status" }).length, 1);
  } finally { await act(async () => renderer.unmount()); }
});

test("memory import controls do not claim cancellation is available once confirmation is in flight", async () => {
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(<MemoryImportControls t={(_zh, en) => en}
    progress={{ phase: "confirming", received: 10, total: 10 }} onImport={async () => {}}
    onResume={async () => {}} onCancel={async () => {}} />); });
  try {
    assert.equal(renderer.root.findByType("input").props.disabled, true);
    const buttons = renderer.root.findAllByType("button");
    assert.equal(buttons.length, 1); assert.equal(buttons[0].props.disabled, true);
    assert.ok(JSON.stringify(renderer.toJSON()).includes("Confirming the whole import"));
  } finally { await act(async () => renderer.unmount()); }
});
