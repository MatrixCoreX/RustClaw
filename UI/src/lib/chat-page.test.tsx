import assert from "node:assert/strict";
import test from "node:test";
import { createRef, type ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

import { ChatPage } from "../components/ChatPage";
import { emptyChatActivity } from "./chat-activity";

const t = (zh: string, _en: string) => zh;

function props(): ComponentProps<typeof ChatPage> {
  return {
    t,
    tSlash: (text) => text,
    artifactFetch: async () => new Response(),
    chatMessages: [],
    chatThreads: [
      {
        id: "chat-thread-one",
        agentId: "main",
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
    availableAgents: [{ id: "main", name: "Main" }],
    activeChatAgentId: "main",
    activeChatCanChangeAgent: true,
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
    chatCompacting: false,
    chatWorking: false,
    chatActivity: emptyChatActivity(),
    chatRecording: false,
    chatVoiceInputEnabled: true,
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
    onActiveChatAgentChange: () => {},
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
    onCompactContext: async () => true,
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
  assert.match(markup, /整理上下文/);
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

test("loads protected media through the authenticated fetcher instead of a raw media URL", () => {
  const pageProps = props();
  pageProps.chatMessages = [
    {
      id: "assistant-video",
      role: "assistant",
      text: "视频已下载",
      ts: 1,
      artifacts: [
        {
          schema_version: 1,
          id: "artifact-video",
          filename: "clip.mp4",
          kind: "video",
          mime_type: "video/mp4",
          size_bytes: 1024,
          sha256: "a".repeat(64),
          download_url: "/v1/tasks/task-1/artifacts/artifact-video/content",
          preview_url:
            "/v1/tasks/task-1/artifacts/artifact-video/content?disposition=inline",
        },
      ],
    },
  ];

  const markup = renderToStaticMarkup(<ChatPage {...pageProps} />);

  assert.match(markup, /正在准备浏览器兼容版/);
  assert.doesNotMatch(markup, /src="\/v1\/tasks\/task-1\/artifacts/);
  assert.doesNotMatch(markup, /href="\/v1\/tasks\/task-1\/artifacts/);
});

test("keeps the send button aligned while allowing the shorter input to grow vertically", () => {
  const markup = renderToStaticMarkup(<ChatPage {...props()} />);

  assert.match(markup, /theme-input h-12 min-h-12 max-h-60 w-full resize-y sm:h-\[72px\] sm:min-h-\[72px\]/);
  assert.match(markup, /theme-accent-btn chat-send-btn min-h-12 min-w-16.*self-stretch.*sm:min-h-\[72px\]/);
});

test("labels busy attachment submissions as live updates instead of queued work", () => {
  const pageProps = props();
  pageProps.chatSending = true;
  pageProps.chatAttachments = [
    {
      kind: "file",
      name: "requirements.txt",
      mimeType: "text/plain",
      size: 12,
      dataUrl: "data:text/plain;base64,Zm9vPT0xLjAK",
    },
  ];

  const markup = renderToStaticMarkup(<ChatPage {...pageProps} />);

  assert.match(markup, />补充<\/button>/);
  assert.doesNotMatch(markup, />排队<\/button>/);
});

test("keeps a machine-control stop action available beside live message handling", () => {
  const pageProps = props();
  pageProps.chatSending = true;
  pageProps.chatCanStop = true;

  const markup = renderToStaticMarkup(<ChatPage {...pageProps} />);

  assert.match(markup, />停止任务<\/button>/);
  assert.match(markup, /title="停止当前任务并等待安全收尾"/);
  assert.doesNotMatch(markup, /停止任务<\/button[^>]*disabled/);
});

test("keeps the composer visible while scrolling only the chat interaction", () => {
  const markup = renderToStaticMarkup(<ChatPage {...props()} />);

  assert.match(markup, /md:h-full md:min-h-0.*md:overflow-hidden/);
  assert.match(markup, /min-h-80 flex-1.*overflow-y-auto.*md:min-h-0/);
  assert.match(markup, /shrink-0 pt-4/);
  assert.doesNotMatch(markup, /lg:min-h-\[32rem\]/);
});

test("offers browser-window maximize controls on the agent chat container", () => {
  const markup = renderToStaticMarkup(<ChatPage {...props()} />);

  assert.match(markup, /id="agent-chat-window"/);
  assert.match(markup, /title="双击标题栏占满浏览器窗口"/);
  assert.match(markup, /aria-label="占满浏览器窗口"/);
  assert.match(markup, /aria-pressed="false"/);
  assert.match(markup, /aria-controls="agent-chat-window"/);
  assert.match(markup, />全屏<\/button>/);
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

test("hides all voice controls when transcription is disabled by the runtime", () => {
  const markup = renderToStaticMarkup(
    <ChatPage
      {...props()}
      chatVoiceInputEnabled={false}
      chatVoiceRecordingSupported
      chatVoiceRecordingAvailability="available"
    />,
  );

  assert.doesNotMatch(markup, /按住发言/);
  assert.doesNotMatch(markup, /麦克风/);
  assert.doesNotMatch(markup, /语音不可用/);
  assert.doesNotMatch(markup, /语音需要 HTTPS/);
});

test("renders compact structured activity instead of a generic working label", () => {
  const pageProps = props();
  pageProps.chatSending = true;
  pageProps.chatWorking = true;
  pageProps.chatActivity = {
    ...emptyChatActivity(),
    stage: "running_tool",
    activeName: "media.download",
    commandPreview: "ffmpeg",
    llmCallCount: 2,
    roundNo: 3,
  };

  const markup = renderToStaticMarkup(<ChatPage {...pageProps} />);

  assert.match(markup, /正在运行系统命令：ffmpeg/);
  assert.match(markup, /\$ ffmpeg/);
  assert.match(markup, /LLM 2/);
  assert.match(markup, /第 3 轮/);
  assert.match(markup, /chat-activity-sweep/);
});

test("keeps live progress outside conversation history and immediately before the composer", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const pageProps = props();
  pageProps.chatWorking = true;
  pageProps.chatTeachingMode = true;
  pageProps.chatActivity = { ...emptyChatActivity(), stage: "llm_request", llmCallCount: 1 };
  pageProps.chatMessages = [{ id: "user-1", role: "user", text: "检查项目", ts: 1 }];
  let renderer!: ReactTestRenderer;
  await act(() => { renderer = create(<ChatPage {...pageProps} />); });
  try {
    const messages = renderer.root.findByProps({ "data-testid": "chat-message-list" });
    assert.equal(messages.findAllByProps({ "data-testid": "chat-working-indicator" }).length, 0);
    const progress = renderer.root.findByProps({ "data-testid": "chat-working-indicator" });
    assert.equal(progress.props.role, "status");
    assert.equal(progress.props["aria-live"], "polite");
    assert.match(progress.props.className, /shrink-0/);
    assert.equal(progress.findByProps({ title: "第 1 次 LLM 调用正在处理" }).children[0], "第 1 次 LLM 调用正在处理");
    const markup = renderToStaticMarkup(<ChatPage {...pageProps} />);
    assert.ok(markup.indexOf("教学模式已开启") < markup.indexOf('data-testid="chat-working-indicator"'));
    assert.ok(markup.indexOf('data-testid="chat-working-indicator"') < markup.indexOf('data-testid="chat-composer"'));
  } finally {
    await act(() => renderer.unmount());
  }
});

test("updates request numbers and removes progress after completion or a task switch", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const pageProps = props();
  pageProps.chatSending = true;
  pageProps.chatActivity = { ...emptyChatActivity(), stage: "llm_request", llmCallCount: 1 };
  let renderer!: ReactTestRenderer;
  await act(() => { renderer = create(<ChatPage {...pageProps} />); });
  try {
    await act(() => renderer.update(<ChatPage {...pageProps} chatActivity={{ ...pageProps.chatActivity, stage: "llm_response", llmCallCount: 2 }} />));
    assert.equal(renderer.root.findAllByProps({ title: "第 1 次 LLM 调用正在处理" }).length, 0);
    assert.equal(renderer.root.findByProps({ title: "第 2 次 LLM 调用正在生成回复" }).children[0], "第 2 次 LLM 调用正在生成回复");
    await act(() => renderer.update(<ChatPage {...pageProps} chatSending={false} chatWorking={false} />));
    assert.equal(renderer.root.findAllByProps({ "data-testid": "chat-working-indicator" }).length, 0);
    await act(() => renderer.update(<ChatPage {...props()} activeChatThreadId="other-thread" />));
    assert.equal(renderer.root.findAllByProps({ "data-testid": "chat-working-indicator" }).length, 0);
    const markup = renderToStaticMarkup(<ChatPage {...pageProps} t={(_zh, en) => en} />);
    assert.match(markup, /LLM call 1 is processing/);
    assert.match(markup, /Task progress/);
  } finally {
    await act(() => renderer.unmount());
  }
});

test("busy tasks keep live-update and attachment controls enabled without a browser execution queue", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const pageProps = props();
  let renderer!: ReactTestRenderer;
  await act(() => { renderer = create(<ChatPage {...pageProps} chatSending chatWorking chatInput="next" />); });
  try {
    const send = renderer.root.findAllByType("button").find(button => button.props.className?.includes("chat-send-btn"))!;
    assert.equal(send.props.disabled, false);
    assert.ok(send.children.includes("补充"));
    const upload = renderer.root.findAllByType("button").find(button => button.children.includes("上传图片/文件"))!;
    assert.equal(upload.props.disabled, false);
    const history = renderer.root.findByProps({ "data-testid": "chat-message-list" });
    assert.equal(history.findAllByProps({ "data-testid": "chat-message-queue" }).length, 0);
  } finally { await act(() => renderer.unmount()); }
});

test("offers an explicit defer mode and durable deferred-message actions", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const pageProps = props();
  const modes: string[] = [];
  const activated: string[] = [];
  const withdrawn: string[] = [];
  pageProps.chatDeliveryMode = "defer";
  pageProps.chatInput = "later";
  pageProps.chatDeferredInputs = [{
    inputId: "input-later",
    text: "review this after the current task",
    attachmentNames: [],
    acceptedAt: 1,
  }];
  pageProps.onChatDeliveryModeChange = mode => modes.push(mode);
  pageProps.onActivateDeferredInput = inputId => activated.push(inputId);
  pageProps.onWithdrawDeferredInput = inputId => withdrawn.push(inputId);
  let renderer!: ReactTestRenderer;
  await act(() => { renderer = create(<ChatPage {...pageProps} />); });
  try {
    assert.equal(renderer.root.findByProps({ "data-testid": "chat-deferred-inputs" }) !== null, true);
    const runLater = renderer.root.findByProps({ "aria-pressed": true });
    assert.ok(runLater.children.includes("稍后处理"));
    const send = renderer.root.findAllByType("button").find(button => button.props.className?.includes("chat-send-btn"))!;
    assert.ok(send.children.includes("延后"));
    const modeGroup = renderer.root.findByProps({ role: "group", "aria-label": "消息处理方式" });
    await act(() => modeGroup.findAllByType("button").find(button => button.props["aria-pressed"] === false)!.props.onClick());
    assert.deepEqual(modes, ["auto"]);
    const deferredPanel = renderer.root.findByProps({ "data-testid": "chat-deferred-inputs" });
    await act(() => deferredPanel.findAllByType("button").find(button => button.children.includes("立即处理"))!.props.onClick());
    await act(() => deferredPanel.findAllByType("button").find(button => button.children.includes("撤回"))!.props.onClick());
    assert.deepEqual(activated, ["input-later"]);
    assert.deepEqual(withdrawn, ["input-later"]);
  } finally {
    await act(() => renderer.unmount());
  }
});
