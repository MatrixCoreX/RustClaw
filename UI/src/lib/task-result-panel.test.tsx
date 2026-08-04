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
    onControlSubagent: () => {},
    onViewTask: () => {},
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

test("renders a beginner-facing subagent panel with active and done controls", () => {
  const base = props();
  base.taskResult = {
    task_id: "parent-1",
    status: "running",
    result_json: {
      child_task_graph: {
        schema_version: 2,
        parent_task_id: "parent-1",
        status: "active",
        session_open_capacity: 4,
        session_open_count: 2,
        main_agent_counted: false,
        nodes: [
          {
            child_task_id: "child-active",
            role: "explorer",
            required: true,
            readiness: "running",
            thread_state: "open",
            execution_state: "running",
          },
          {
            child_task_id: "child-done",
            role: "verifier",
            required: false,
            readiness: "succeeded",
            thread_state: "done",
            execution_state: "succeeded",
          },
        ],
      },
    },
  };
  const markup = renderToStaticMarkup(<TaskResultPanel {...base} />);
  assert.match(markup, /并行任务/);
  assert.match(markup, /正在处理 1 项，已完成 1 项/);
  assert.match(markup, /停止全部/);
  assert.match(markup, /补充要求/);
  assert.match(markup, /关闭/);
  assert.match(markup, /主任务不计入并行容量/);
});
