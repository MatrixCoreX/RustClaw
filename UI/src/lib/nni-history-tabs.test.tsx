import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { NniHistoryTabs } from "../components/NniHistoryTabs";

const renderTabs = (activeView: "overview" | "rewards" | "records" | "errors") =>
  renderToStaticMarkup(
    <NniHistoryTabs
      activeView={activeView}
      recordsTotal={12}
      errorsTotal={3}
      rewardsTotal={7}
      t={(zh) => zh}
      onChange={() => undefined}
    />,
  );

test("shows the device overview and three record pages as accessible tabs", () => {
  const markup = renderTabs("records");

  assert.match(markup, /role="tablist"/);
  assert.match(markup, /aria-label="NNI 页面"/);
  assert.match(markup, /id="nni-overview-tab"[^>]*aria-selected="false"/);
  assert.match(markup, /id="nni-history-rewards-tab"[^>]*aria-selected="false"/);
  assert.match(markup, /id="nni-history-records-tab"[^>]*aria-selected="true"/);
  assert.match(markup, /id="nni-history-errors-tab"[^>]*aria-selected="false"/);
  assert.match(markup, />请求记录<\/span><span[^>]*>12<\/span>/);
  assert.match(markup, />心跳错误<\/span><span[^>]*>3<\/span>/);
  assert.match(markup, />心跳奖励<\/span><span[^>]*>7<\/span>/);
  assert.match(markup, />设备与运行<\/span>/);
});

test("marks the heartbeat error tab active after switching views", () => {
  const markup = renderTabs("errors");

  assert.match(markup, /id="nni-history-records-tab"[^>]*aria-selected="false"/);
  assert.match(markup, /id="nni-history-errors-tab"[^>]*aria-selected="true"/);
  assert.match(markup, /aria-controls="nni-history-records-panel"/);
  assert.match(markup, /aria-controls="nni-history-errors-panel"/);
  assert.match(markup, /aria-controls="nni-history-rewards-panel"/);
  assert.match(markup, /aria-controls="nni-overview-primary-panel nni-overview-actions-panel"/);
});

test("shows the four device containers on the overview page", () => {
  const markup = renderTabs("overview");

  assert.match(markup, /id="nni-overview-tab"[^>]*aria-selected="true"/);
  assert.match(markup, /id="nni-history-rewards-tab"[^>]*aria-selected="false"/);
  assert.match(markup, /id="nni-history-records-tab"[^>]*aria-selected="false"/);
  assert.match(markup, /id="nni-history-errors-tab"[^>]*aria-selected="false"/);
});

test("marks heartbeat rewards active", () => {
  const markup = renderTabs("rewards");

  assert.match(markup, /id="nni-history-rewards-tab"[^>]*aria-selected="true"/);
  assert.match(markup, /id="nni-history-records-tab"[^>]*aria-selected="false"/);
  assert.match(markup, /id="nni-history-errors-tab"[^>]*aria-selected="false"/);
});
