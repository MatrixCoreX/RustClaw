import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { FinancialServiceNodeSelector } from "../components/FinancialServiceNodeSelector";

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
    />,
  );

  assert.match(markup, /节点切换/);
  assert.doesNotMatch(markup, /资产节点|不改变 NNI 和 BANCOR 节点/);
  assert.match(markup, /api-2\.example\.test/);
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
    />,
  );

  assert.match(markup, /节点切换/);
  assert.doesNotMatch(markup, /BANCOR 节点|不改变 NNI 和资产节点/);
  assert.match(markup, /data-financial-service-node-selector="bancor"/);
});
