import assert from "node:assert/strict";
import test from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import {
  TaskResultPanel,
  type TaskResultPanelProps,
} from "../components/TaskResultPanel.tsx";

function props(): TaskResultPanelProps {
  return {
    lang: "zh",
    t: (zh) => zh,
    tSlash: (value) => value.split(" / ")[0] ?? value,
    taskId: "task-plan-card",
    taskLoading: false,
    taskError: null,
    taskResult: {
      task_id: "task-plan-card",
      status: "running",
      result_json: null,
      error_text: null,
      task_plan: {
        schema_version: 1,
        source: "task_plan",
        status: "ok",
        data_only: true,
        render_owner: "ui_cli_channel_projection",
        task_id: "task-plan-card",
        plan_revision: 2,
        updated_at_ms: 1234,
        steps: [
          { step_id: "inspect", title: "检查当前状态", status: "completed" },
          { step_id: "implement", title: "实施修复", status: "in_progress" },
          { step_id: "verify", title: "验证结果", status: "pending" },
        ],
        checkpoint: {
          kind: "task_plan",
          ref: "task_plan:task-plan-card:2",
          plan_revision: 2,
        },
      },
    },
    taskLlmDebug: null,
    taskLlmDebugLoading: false,
    taskLlmDebugError: null,
    resumeDrafts: {},
    resumeSubmittingTaskId: null,
    taskControlSubmittingId: null,
    onTaskIdChange: () => {},
    onQueryTask: () => {},
    onQueryTaskLlmDebug: () => {},
    onResumeDraftChange: () => {},
    onSubmitResume: () => {},
    onDecideTaskApproval: () => {},
    onControlTask: () => {},
    onControlTaskGoal: () => {},
  };
}

test("renders the task plan as a clear step card with raw JSON collapsed", () => {
  const markup = renderToStaticMarkup(<TaskResultPanel {...props()} />);

  assert.match(markup, /当前执行计划/);
  assert.match(markup, /已完成 1 \/ 3 步/);
  assert.match(markup, /检查当前状态/);
  assert.match(markup, /实施修复/);
  assert.match(markup, /验证结果/);
  assert.match(markup, /已完成/);
  assert.match(markup, /进行中/);
  assert.match(markup, /待处理/);
  assert.match(markup, /技术详情（原始 JSON）/);
  assert.match(markup, /<details/);
  assert.match(markup, />v2</);
});
