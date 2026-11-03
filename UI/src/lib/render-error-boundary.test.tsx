import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { RenderErrorBoundary } from "../components/RenderErrorBoundary";

test("a failed page keeps navigation available, hides raw errors and supports retry", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let fail = true;
  function Page() {
    if (fail) throw new Error("fixture-private-provider-payload");
    return <p>recovered-page</p>;
  }
  let renderer: ReactTestRenderer;
  const oldError = console.error;
  console.error = () => {};
  try {
    await act(async () => { renderer = create(<div><nav>navigation</nav>
      <RenderErrorBoundary t={(_zh, en) => en}><Page /></RenderErrorBoundary></div>); });
    const failed = JSON.stringify(renderer!.toJSON());
    assert.match(failed, /navigation/);
    assert.match(failed, /Try again/);
    assert.doesNotMatch(failed, /fixture-private-provider-payload/);
    assert.match(renderer!.root.findByProps({ role: "alert" }).props.className, /theme-panel-soft/);
    assert.equal(renderer!.root.findByType("h2").props.style.color, "var(--theme-text-strong)");
    assert.equal(renderer!.root.findAllByType("p")[0].props.style.color, "var(--theme-text-muted)");
    fail = false;
    await act(async () => { renderer!.root.findByType("button").props.onClick(); });
    assert.match(JSON.stringify(renderer!.toJSON()), /recovered-page/);
  } finally {
    if (renderer!) await act(async () => { renderer.unmount(); });
    console.error = oldError;
  }
});
