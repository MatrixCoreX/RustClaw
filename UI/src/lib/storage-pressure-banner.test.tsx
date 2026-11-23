import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { StoragePressureBanner } from "../components/StoragePressureBanner";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

test("ignoring the disk warning survives a reload, is scoped to the server, and expires", async context => {
  context.mock.timers.enable({ apis: ["Date", "setTimeout"], now: 100_000 });
  const originalWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  const stored = new Map<string, string>();
  Object.defineProperty(globalThis, "window", { configurable: true, value: {
    location: { href: "https://console.example.test" },
    localStorage: {
      getItem: (key: string) => stored.get(key) ?? null,
      setItem: (key: string, value: string) => stored.set(key, value),
    },
  } });
  const firstServer = "https://first.example.test";
  const banner = (apiBase = firstServer, state: "warning" | "critical" = "warning") => (
    <StoragePressureBanner key={apiBase} apiBase={apiBase}
      pressure={{ state, write_admission: state === "warning", sampled_at_ts: 100, volumes: [] }}
      t={zh => zh} />
  );
  let renderer: ReactTestRenderer | undefined;
  try {
    await act(async () => { renderer = create(banner()); });
    const current = () => renderer!;
    const button = current().root.findByType("button");
    assert.deepEqual(button.children, ["忽略 30 分钟"]);
    await act(async () => { button.props.onClick(); });
    assert.equal(current().toJSON(), null);

    await act(async () => { current().unmount(); renderer = create(banner()); });
    assert.equal(current().toJSON(), null);
    await act(async () => { current().update(banner(firstServer, "critical")); });
    assert.match(JSON.stringify(current().toJSON()), /已暂停新的文件写入/);
    assert.equal(current().root.findAllByType("button").length, 0);

    await act(async () => { current().update(banner("https://second.example.test")); });
    assert.equal(current().root.findAllByType("button").length, 1);
    await act(async () => { current().update(banner()); });
    assert.equal(current().toJSON(), null);
    await act(async () => { context.mock.timers.tick(30 * 60_000 - 1); });
    assert.equal(current().toJSON(), null);
    await act(async () => { context.mock.timers.tick(1); });
    assert.match(JSON.stringify(current().toJSON()), /磁盘可用空间偏低/);
  } finally {
    await act(async () => { renderer?.unmount(); });
    if (originalWindow) Object.defineProperty(globalThis, "window", originalWindow);
    else Reflect.deleteProperty(globalThis, "window");
    context.mock.timers.reset();
  }
});
