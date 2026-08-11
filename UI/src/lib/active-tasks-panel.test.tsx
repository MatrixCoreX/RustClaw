import assert from "node:assert/strict";
import test from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import {
  ActiveTasksPanel,
  type ActiveTasksPanelProps,
} from "../components/ActiveTasksPanel.tsx";

function props(): ActiveTasksPanelProps {
  return {
    lang: "zh",
    t: (zh) => zh,
    activeTasks: [
      {
        index: 3,
        task_id: "task-cancel-by-id",
        kind: "ask",
        status: "running",
        channel: "wechat",
        source_user_id: "42",
        external_user_id: "wechat-user-17",
        summary: "正在处理",
        age_seconds: 10,
      },
    ],
    activeTasksLoading: false,
    activeTasksError: null,
    activeTasksLastUpdated: null,
    resumeTaskError: null,
    resumeTaskMessage: null,
    cancelTaskError: null,
    cancelTaskMessage: null,
    cancelingTaskId: null,
    taskControlSubmittingId: null,
    taskControlMessage: null,
    taskControlError: null,
    canUseInteractionContext: false,
    resumeDrafts: {},
    resumeSubmittingTaskId: null,
    toLocalTime: () => "",
    onFetchActiveTasks: () => {},
    onViewTask: () => {},
    onCancelTask: () => {},
    onControlTask: () => {},
    onResumeDraftChange: () => {},
    onSubmitResume: () => {},
  };
}

test("task-id cancellation stays available before interaction identity loads", () => {
  const markup = renderToStaticMarkup(<ActiveTasksPanel {...props()} />);
  const cancelButton = markup.match(
    /<button[^>]*class="[^"]*border-rose-300[^"]*"[^>]*>[\s\S]*?<\/button>/,
  )?.[0];

  assert.ok(cancelButton);
  assert.doesNotMatch(cancelButton, /\sdisabled(?:=|\s|>)/);
  assert.match(cancelButton, /取消/);
});

test("active task cards show their channel and sending user", () => {
  const markup = renderToStaticMarkup(<ActiveTasksPanel {...props()} />);

  assert.match(markup, /来源: 微信/);
  assert.match(markup, /用户:/);
  assert.match(markup, /wechat-user-17/);
});
