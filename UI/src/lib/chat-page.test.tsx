import assert from "node:assert/strict";
import test from "node:test";
import { createRef, type ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { ChatPage } from "../components/ChatPage";

const t = (zh: string, _en: string) => zh;

function props(): ComponentProps<typeof ChatPage> {
  return {
    t,
    tSlash: (text) => text,
    chatMessages: [],
    chatThreads: [
      {
        id: "chat-thread-one",
        title: "自定义任务",
        preview: "检查任务名称操作",
        updatedAt: 1,
        messageCount: 1,
        teachingMode: false,
        taskId: null,
        taskStatus: null,
        llmCallCount: null,
      },
    ],
    activeChatThreadId: "chat-thread-one",
    chatInput: "",
    chatAttachments: [],
    chatTeachingMode: false,
    chatTeachingTaskResult: null,
    chatTeachingLlmDebug: null,
    chatTeachingLlmDebugLoading: false,
    chatTeachingLlmDebugError: null,
    chatTeachingRuns: [],
    activeChatTeachingRunId: null,
    chatSending: false,
    chatWorking: false,
    chatRecording: false,
    chatVoiceRecordingSupported: false,
    chatAudioInputDevices: [],
    chatAudioInputDeviceId: "",
    chatError: null,
    chatAttachmentInputRef: createRef<HTMLInputElement>(),
    toLocalTime: () => "刚刚",
    onChatTeachingModeChange: () => {},
    onSelectChatTeachingRun: () => {},
    onCreateNewChatThread: () => {},
    onSelectChatThread: () => {},
    onRenameChatThread: async () => true,
    onDeleteChatThread: async () => true,
    onClearMessages: async () => true,
    onChatInputChange: () => {},
    onChatInputKeyDown: () => {},
    onAttachmentSelection: () => {},
    onRemoveAttachment: () => {},
    onStartVoiceRecording: () => {},
    onStopVoiceRecording: () => {},
    onAudioInputDeviceChange: () => {},
    onSendMessage: () => {},
    onQueryChatTeachingLlmDebug: () => {},
  };
}

test("renders task rename and delete as directly operable controls", () => {
  const markup = renderToStaticMarkup(<ChatPage {...props()} />);

  assert.match(markup, /aria-label="重命名任务：自定义任务"/);
  assert.match(markup, /aria-label="删除任务：自定义任务"/);
  assert.doesNotMatch(markup, /aria-expanded=/);
});
