import assert from "node:assert/strict";
import test from "node:test";
import { createRef, type ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { LogsPage } from "../components/LogsPage";

const t = (zh: string, _en: string) => zh;

function props(): ComponentProps<typeof LogsPage> {
  return {
    t,
    tSlash: (text) => text,
    logFiles: ["runtime-current.log", "webd-device.log"],
    logFilesLoading: false,
    logFilesError: null,
    selectedLogFile: "runtime-current.log",
    logTailLines: 200,
    logFollowTail: true,
    logLastUpdated: null,
    logLoading: false,
    logError: null,
    logText: "ready",
    logContainerRef: createRef<HTMLPreElement>(),
    toLocalTime: () => "刚刚",
    onSelectedLogFileChange: () => {},
    onLogTailLinesChange: () => {},
    onLogFollowTailChange: () => {},
    onRefreshLogs: () => {},
  };
}

test("renders only log files discovered by the backend", () => {
  const markup = renderToStaticMarkup(<LogsPage {...props()} />);

  assert.match(markup, /runtime-current\.log/);
  assert.match(markup, /webd-device\.log/);
  assert.doesNotMatch(markup, /agent_trace\.log/);
  assert.doesNotMatch(markup, /telegramd\.log/);
});

test("shows an empty state when the log directory has no readable logs", () => {
  const markup = renderToStaticMarkup(
    <LogsPage {...props()} logFiles={[]} selectedLogFile="" />,
  );

  assert.match(markup, /没有可用日志/);
  assert.match(markup, /disabled=""/);
});
