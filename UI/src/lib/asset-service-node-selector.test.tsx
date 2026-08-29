import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { FinancialServiceNodeSelector } from "../components/FinancialServiceNodeSelector";

test("asset service selector explains its independent scope", () => {
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

  assert.match(markup, /资产节点/);
  assert.match(markup, /不改变 NNI 和 BANCOR 节点/);
  assert.match(markup, /api-2\.example\.test/);
  assert.match(markup, /data-financial-service-node-selector="assets"/);
});

test("BANCOR selector explains that it does not change the other nodes", () => {
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

  assert.match(markup, /BANCOR 节点/);
  assert.match(markup, /不改变 NNI 和资产节点/);
  assert.match(markup, /data-financial-service-node-selector="bancor"/);
});
