import assert from "node:assert/strict";
import test from "node:test";

import { emptyChatActivity, reduceChatActivity } from "./chat-activity";

test("projects task submission as queue status instead of assistant text", () => {
  const activity = reduceChatActivity(emptyChatActivity(), {
    schema_version: 1,
    seq: 1,
    task_id: "task-queued",
    event_kind: "task_submitted",
    payload: { task_status: "queued", execution_mode: "foreground" },
  });

  assert.equal(activity.stage, "queued");
  assert.equal(activity.activeName, null);
  assert.equal(activity.commandPreview, null);
});

test("counts LLM starts without counting streamed fragments", () => {
  const started = reduceChatActivity(emptyChatActivity(), {
    schema_version: 1,
    seq: 1,
    task_id: "task-1",
    event_kind: "model_turn",
    payload: { type: "started", provider: "provider:model" },
  });
  const fragment = reduceChatActivity(started, {
    schema_version: 1,
    seq: 2,
    task_id: "task-1",
    event_kind: "model_turn",
    payload: { type: "text_delta", text_delta_bytes: 20 },
  });

  assert.equal(started.llmCallCount, 1);
  assert.equal(fragment.llmCallCount, 1);
  assert.equal(fragment.stage, "llm_response");
});

test("shows only the structured tool name and round", () => {
  const activity = reduceChatActivity(emptyChatActivity(), {
    schema_version: 1,
    seq: 4,
    task_id: "task-1",
    event_kind: "tool_active",
    payload: {
      action_ref: "media.download",
      round_no: 2,
      command_preview: "ffmpeg",
      arguments: { secret: "must-not-be-projected" },
    },
  });

  assert.equal(activity.stage, "running_tool");
  assert.equal(activity.activeName, "media.download");
  assert.equal(activity.roundNo, 2);
  assert.equal(activity.commandPreview, "ffmpeg");
  assert.equal(JSON.stringify(activity).includes("must-not-be-projected"), false);
});

test("ignores a replayed event sequence", () => {
  const first = reduceChatActivity(emptyChatActivity(), {
    schema_version: 1,
    seq: 5,
    task_id: "task-1",
    event_kind: "model_turn",
    payload: { type: "started" },
  });
  const replayed = reduceChatActivity(first, {
    schema_version: 1,
    seq: 5,
    task_id: "task-1",
    event_kind: "model_turn",
    payload: { type: "started" },
  });

  assert.equal(replayed.llmCallCount, 1);
});

test("uses the shared skill progress event without exposing frame params", () => {
  const activity = reduceChatActivity(emptyChatActivity(), {
    schema_version: 1,
    seq: 6,
    task_id: "task-1",
    event_kind: "skill_progress",
    event_type: "skill_progress",
    payload: {
      skill_name: "media_download",
      frame: {
        detail_key: "media_download.transcribe.recognizing_speech",
        params: { unsafe_display_text: "do not render me" },
        current: 2,
        total: 3,
      },
    },
  });

  assert.equal(activity.stage, "running_tool");
  assert.equal(activity.activeName, "media_download");
  assert.equal(activity.progressDetailKey, "media_download.transcribe.recognizing_speech");
  assert.equal(activity.progressCurrent, 2);
  assert.equal(activity.progressTotal, 3);
  assert.equal(JSON.stringify(activity).includes("do not render me"), false);
});

test("projects terminal respond actions as response verification instead of returned tools", () => {
  const activity = reduceChatActivity(emptyChatActivity(), {
    schema_version: 1,
    seq: 7,
    task_id: "task-respond",
    event_kind: "tool_finished",
    payload: {
      action_kind: "respond",
      skill: "respond",
      status: "ok",
    },
  });

  assert.equal(activity.stage, "verifying_response");
  assert.equal(activity.activeName, null);
});

test("keeps ordinary completed capabilities in the tool-returned stage", () => {
  const activity = reduceChatActivity(emptyChatActivity(), {
    schema_version: 1,
    seq: 8,
    task_id: "task-skill",
    event_kind: "tool_finished",
    payload: {
      action_kind: "call_skill",
      resolved_tool_or_skill: "rss_fetch",
      status: "ok",
    },
  });

  assert.equal(activity.stage, "tool_returned");
  assert.equal(activity.activeName, "rss_fetch");
});
