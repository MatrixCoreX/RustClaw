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
    chatVoiceRecordingAvailability: "media_devices_unavailable",
    chatAudioInputDevices: [],
    chatAudioInputDeviceId: "",
    chatError: null,
    chatHistoryHasMore: false,
    chatHistoryLoading: false,
    chatBodyLoadingMessageId: null,
    chatAttachmentInputRef: createRef<HTMLInputElement>(),
    toLocalTime: () => "刚刚",
    onChatTeachingModeChange: () => {},
    onSelectChatTeachingRun: () => {},
    onCreateNewChatThread: () => {},
    onSelectChatThread: () => {},
    onRenameChatThread: async () => true,
    onDeleteChatThread: async () => true,
    onLoadEarlierConversationHistory: () => {},
    onLoadNextChatMessageBody: () => {},
    onClearMessages: async () => true,
    onChatInputChange: () => {},
    onChatInputKeyDown: () => {},
    onAttachmentSelection: () => {},
    onRemoveAttachment: () => {},
    onStartVoiceRecording: () => {},
    onStopVoiceRecording: () => {},
    onCancelVoiceRecording: () => {},
    onAudioInputDeviceChange: () => {},
    onSendMessage: () => {},
    onQueryChatTeachingLlmDebug: () => {},
  };
}

test("renders task rename and delete as directly operable controls", () => {
  const markup = renderToStaticMarkup(<ChatPage {...props()} />);

  assert.match(markup, /aria-label="重命名任务：自定义任务"/);
  assert.match(markup, /aria-label="删除任务：自定义任务"/);
  assert.match(markup, /aria-label="收起任务历史"/);
  assert.match(markup, /aria-expanded="true"/);
  assert.match(markup, /aria-controls="chat-task-history-content"/);
});

test("renders progressive controls for older history and long messages", () => {
  const pageProps = props();
  pageProps.chatHistoryHasMore = true;
  pageProps.chatMessages = [
    {
      id: "a-large",
      role: "assistant",
      text: "部分回答",
      ts: 1,
      bodyResult: {
        schema_version: 1,
        complete: false,
        original_size_bytes: 100_000,
        returned_size_bytes: 10_000,
        content_sha256: "a".repeat(64),
        continuation: {
          kind: "conversation_body_range",
          url: `/v1/tasks/task-1/conversation-body/assistant?start_byte=10000&sha256=${"a".repeat(64)}`,
          next_start_byte: 10_000,
        },
      },
    },
  ];

  const markup = renderToStaticMarkup(<ChatPage {...pageProps} />);

  assert.match(markup, /加载更早的任务/);
  assert.match(markup, /继续查看完整内容/);
  assert.match(markup, /9.8 KB/);
});

test("renders a newly prepended task above older task history", () => {
  const pageProps = props();
  pageProps.chatThreads = [
    {
      ...pageProps.chatThreads[0],
      id: "chat-thread-new",
      title: "最新创建任务",
      updatedAt: 2,
    },
    {
      ...pageProps.chatThreads[0],
      id: "chat-thread-old",
      title: "旧任务",
      updatedAt: 1,
    },
  ];
  pageProps.activeChatThreadId = "chat-thread-new";

  const markup = renderToStaticMarkup(<ChatPage {...pageProps} />);

  assert.ok(markup.indexOf("最新创建任务") < markup.indexOf("旧任务"));
});

test("describes hold-to-talk voice as release-to-send without a preview step", () => {
  const markup = renderToStaticMarkup(
    <ChatPage
      {...props()}
      chatVoiceRecordingSupported
      chatVoiceRecordingAvailability="available"
    />,
  );

  assert.match(markup, /按住发言/);
  assert.match(markup, /松开后自动发送/);
  assert.doesNotMatch(markup, /松开后试听/);
});

test("keeps an actionable HTTPS explanation when HTTP IP recording is blocked", () => {
  const markup = renderToStaticMarkup(
    <ChatPage
      {...props()}
      chatVoiceRecordingAvailability="insecure_context"
    />,
  );

  assert.match(markup, /语音需要 HTTPS/);
  assert.match(markup, /HTTP IP 页面使用麦克风/);
});
