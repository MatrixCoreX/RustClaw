import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { useChannelConfigRuntime } from "../../src/hooks/useChannelConfigRuntime";
import "../../src/index.css";

const query = new URLSearchParams(location.search);
document.documentElement.dataset.theme = query.get("theme") || "light";
function Fixture() {
  const [base, setBase] = useState("https://device-a.example.test");
  const [ready, setReady] = useState(true), [admin, setAdmin] = useState(true), [epoch, setEpoch] = useState(0);
  const runtime = useChannelConfigRuntime({ apiFetch: (path, init) => fetch(`${base}${path}`, init),
    requestScope: ready ? JSON.stringify([base, admin, epoch]) : "", isAdminIdentity: admin, t: (_zh, en) => en });
  useEffect(() => {
    const change = (event: Event) => {
      switch ((event as CustomEvent).detail) {
        case "device": setBase("https://device-b.example.test"); break;
        case "return": setBase("https://device-a.example.test"); break;
        case "logout": setReady(false); break;
        case "role": setAdmin(false); break;
        case "epoch": setEpoch(value => value + 1); break;
      }
    };
    window.addEventListener("fixture-scope", change);
    return () => window.removeEventListener("fixture-scope", change);
  }, []);
  const reads = { wechat: runtime.fetchWechatConfig, feishu: runtime.fetchFeishuConfig,
    lark: runtime.fetchLarkConfig, telegram: runtime.fetchTelegramConfig };
  const state = {
    scope: [base, ready, admin, epoch],
    wechat: { data: runtime.wechatConfigData, loading: runtime.wechatConfigLoading, saving: runtime.wechatConfigSaving, error: runtime.wechatConfigError },
    feishu: { data: runtime.feishuConfigData, loading: runtime.feishuConfigLoading, error: runtime.feishuConfigError },
    lark: { data: runtime.larkConfigData, loading: runtime.larkConfigLoading, error: runtime.larkConfigError },
    telegram: { data: runtime.telegramConfigData, loading: runtime.telegramConfigLoading, saving: runtime.telegramConfigSaving,
      error: runtime.telegramConfigError, dirty: runtime.hasUnsavedTelegramConfigChanges, message: runtime.telegramConfigSaveMessage,
      draftName: runtime.primaryTelegramBot.name, draftToken: runtime.primaryTelegramBot.bot_token },
  };
  return <main className="theme-shell min-h-screen space-y-3 p-4">
    <div className="flex flex-wrap gap-2">{Object.entries(reads).map(([channel, read]) =>
      <button className="theme-secondary-btn px-3 py-2" key={channel} data-read={channel} onClick={() => void read()}>Read {channel}</button>)}
      <button className="theme-secondary-btn px-3 py-2" data-write="wechat" onClick={() => void runtime.setWechatEnabled(true)}>Enable WeChat</button>
      <button className="theme-secondary-btn px-3 py-2" data-write="telegram" onClick={() => void runtime.saveTelegramConfig()}>Save Telegram</button>
    </div>
    <label className="block">Draft name<input className="theme-input" aria-label="Draft name" value={runtime.primaryTelegramBot.name}
      onChange={event => runtime.setTelegramPrimaryBotDraftField("name", event.target.value)} /></label>
    <label className="block">Draft token<input className="theme-input" aria-label="Draft token" value={runtime.primaryTelegramBot.bot_token}
      onChange={event => runtime.setTelegramPrimaryBotDraftField("bot_token", event.target.value)} /></label>
    <pre className="whitespace-pre-wrap break-all text-xs" id="state">{JSON.stringify(state)}</pre>
  </main>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
