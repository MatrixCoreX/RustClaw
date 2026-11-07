import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { UiDialogProvider, useUiDialog } from "../components/UiDialogProvider";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
async function dialogs() {
  let service!: ReturnType<typeof useUiDialog>, renderer!: ReactTestRenderer;
  function Probe() { service = useUiDialog(); return null; }
  await act(async () => { renderer = create(<UiDialogProvider><Probe /></UiDialogProvider>); });
  return { service, close: async () => { await act(async () => renderer.unmount()); } };
}

test("UI dialog rejects an already aborted approval without exposing a queued modal", async () => {
  const host = await dialogs(), controller = new AbortController(); controller.abort();
  try {
    let pending!: Promise<boolean>;
    await act(async () => { pending = host.service.confirm({ signal: controller.signal, message: "private fixture" }); });
    assert.equal(await pending, false);
  } finally { await host.close(); }
});

test("UI dialog abort settles an active approval before a stale modal can be rendered", async () => {
  const host = await dialogs(), controller = new AbortController();
  try {
    let pending!: Promise<boolean>;
    await act(async () => {
      pending = host.service.confirm({ signal: controller.signal, message: "private fixture" });
      controller.abort();
    });
    assert.equal(await pending, false);
  } finally { await host.close(); }
});

test("UI dialog removes cancelled queued approvals and settles active and queued requests independently", async () => {
  const host = await dialogs(), active = new AbortController(), queued = new AbortController();
  try {
    let first!: Promise<boolean>, second!: Promise<boolean>, third!: Promise<string | null>;
    await act(async () => {
      first = host.service.confirm({ signal: active.signal, message: "first private fixture" });
      second = host.service.confirm({ signal: queued.signal, message: "second private fixture" });
      third = host.service.prompt({ signal: queued.signal, message: "third private fixture" });
      queued.abort();
      active.abort();
    });
    assert.deepEqual(await Promise.all([first, second, third]), [false, false, null]);
  } finally { await host.close(); }
});
