import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

import {
  FinancialServiceNodeSelector,
  normalizeCustomFinancialNodeUrl,
} from "../components/FinancialServiceNodeSelector";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

test("asset service selector uses the compact node-switch label", () => {
  const markup = renderToStaticMarkup(
    <FinancialServiceNodeSelector
      t={(zh) => zh}
      service="assets"
      nodes={["https://api-1.example.test", "https://api-2.example.test"]}
      selectedNodeUrl="https://api-2.example.test"
      saving={false}
      error={null}
      onChange={async () => true}
      onAddNode={async () => true}
    />,
  );

  assert.match(markup, /节点切换/);
  assert.doesNotMatch(markup, /资产节点|不改变 NNI 和 BANCOR 节点/);
  assert.match(markup, /api-2\.example\.test/);
  assert.match(markup, /添加自定义节点/);
  assert.match(markup, /data-financial-service-node-selector="assets"/);
});

test("BANCOR selector uses the same compact node-switch label", () => {
  const markup = renderToStaticMarkup(
    <FinancialServiceNodeSelector
      t={(zh) => zh}
      service="bancor"
      nodes={["https://api-1.example.test", "https://api-2.example.test"]}
      selectedNodeUrl="https://api-1.example.test"
      saving={false}
      error={null}
      onChange={async () => true}
      onAddNode={async () => true}
    />,
  );

  assert.match(markup, /节点切换/);
  assert.doesNotMatch(markup, /BANCOR 节点|不改变 NNI 和资产节点/);
  assert.match(markup, /data-financial-service-node-selector="bancor"/);
});

test("custom node URLs are normalized and unsafe forms are rejected", () => {
  assert.equal(
    normalizeCustomFinancialNodeUrl(" https://API.example.test:8443/v1/ "),
    "https://api.example.test:8443",
  );
  assert.equal(
    normalizeCustomFinancialNodeUrl("https://api.example.test/network/"),
    "https://api.example.test/network",
  );
  assert.equal(normalizeCustomFinancialNodeUrl("ftp://api.example.test"), null);
  assert.equal(normalizeCustomFinancialNodeUrl("https://user:secret@api.example.test"), null);
  assert.equal(normalizeCustomFinancialNodeUrl("https://api.example.test?token=secret"), null);
  assert.equal(normalizeCustomFinancialNodeUrl("not a URL"), null);
});

test("custom node control adds and selects a validated node", async () => {
  let addedNode = "";
  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(
      <FinancialServiceNodeSelector
        t={(zh) => zh}
        service="assets"
        nodes={["https://api-1.example.test", "https://api-2.example.test"]}
        selectedNodeUrl="https://api-1.example.test"
        saving={false}
        error={null}
        onChange={async () => true}
        onAddNode={async (nodeUrl) => {
          addedNode = nodeUrl;
          return true;
        }}
      />,
    );
  });

  try {
    const addButton = renderer!.root.findAllByType("button").find(
      (button) => button.props["aria-label"] === "添加自定义节点",
    );
    assert.ok(addButton);
    await act(async () => addButton.props.onClick());
    const input = renderer!.root.findByType("input");
    await act(async () => input.props.onChange({ target: { value: "https://custom.example.test/v1/" } }));
    const form = renderer!.root.findByType("form");
    await act(async () => form.props.onSubmit({ preventDefault() {} }));

    assert.equal(addedNode, "https://custom.example.test");
    assert.equal(renderer!.root.findAllByType("input").length, 0);
  } finally {
    await act(async () => renderer?.unmount());
  }
});
