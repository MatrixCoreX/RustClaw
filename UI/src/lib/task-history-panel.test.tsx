import assert from "node:assert/strict";
import test from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import { TaskHistoryPanel } from "../components/TaskHistoryPanel.tsx";

test("task history renders status, source, user, duration, and pagination", () => {
  const markup = renderToStaticMarkup(
    <TaskHistoryPanel
      lang="zh"
      t={(zh) => zh}
      taskHistory={[
        {
          task_id: "history-task-1",
          kind: "ask",
          status: "succeeded",
          channel: "wechat",
          source_user_id: "42",
          external_user_id: "wechat-user-17",
          summary: "整理下载内容",
          created_at_ts: 1_700_000_000,
          updated_at_ts: 1_700_000_125,
          duration_seconds: 125,
        },
      ]}
      taskHistoryLoading={false}
      taskHistoryError={null}
      taskHistoryTotal={21}
      taskHistoryOffset={20}
      taskHistoryLimit={20}
      onFetchTaskHistory={() => {}}
      onViewTask={() => {}}
    />,
  );

  assert.match(markup, /已完成/);
  assert.match(markup, /整理下载内容/);
  assert.match(markup, /来源: 微信/);
  assert.match(markup, /wechat-user-17/);
  assert.match(markup, /2m 5s/);
  assert.match(markup, /第 2 \/ 2 页/);
  assert.match(markup, /打开报告/);
});
