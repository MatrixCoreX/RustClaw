import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { UiDialogProvider } from "../../src/components/UiDialogProvider";
import { useChannelBindRuntime } from "../../src/hooks/useFeishuBindRuntime";
import type { AgentAppChannel } from "../../src/lib/feishu-bind";
import "../../src/index.css";

const query = new URLSearchParams(location.search);
document.documentElement.dataset.theme = query.get("theme") || "light";
function Fixture() {
  const [base, setBase] = useState("https://device-a.example.test");
  const [platform, setPlatform] = useState<AgentAppChannel>(query.get("platform") === "lark" ? "lark" : "feishu");
  const [ready, setReady] = useState(true), [admin, setAdmin] = useState(true), [epoch, setEpoch] = useState(0);
  const refresh = async (kind: string) => {
    const response = await fetch(`${base}/fixture/${platform}/${kind}`);
    if (!response.ok) throw new Error("fixture_refresh_unavailable");
  };
  const runtime = useChannelBindRuntime({ apiFetch: (path, init) => fetch(`${base}${path}`, init), platform,
    requestScope: JSON.stringify([base, admin, epoch]), uiAuthReady: ready, isAdminIdentity: admin,
    t: (_zh, en) => en, onConfigRefresh: () => refresh("config"), onHealthRefresh: () => refresh("health") });
  const captured = useRef(runtime).current;
  useEffect(() => {
    const change = (event: Event) => {
      switch ((event as CustomEvent).detail) {
        case "device": setBase("https://device-b.example.test"); break;
        case "return": setBase("https://device-a.example.test"); break;
        case "logout": setReady(false); break;
        case "role": setAdmin(false); break;
        case "epoch": setEpoch(value => value + 1); break;
        case "platform": setPlatform(value => value === "lark" ? "feishu" : "lark"); break;
      }
    };
    window.addEventListener("fixture-scope", change);
    return () => window.removeEventListener("fixture-scope", change);
  }, []);
  const state = { scope: [base, ready, admin, epoch, platform], loading: runtime.bindLoading,
    error: runtime.bindError, session: runtime.bindSession, qr: runtime.bindQrDataUrl, resetting: runtime.resetLoading };
  return <main className="theme-shell min-h-screen space-y-3 p-4">
    <div className="flex flex-wrap gap-2">
      <button className="theme-secondary-btn px-3 py-2" data-begin onClick={() => void runtime.beginBind()}>Begin binding</button>
      <button className="theme-secondary-btn px-3 py-2" data-reset onClick={() => void runtime.resetSetup()}>Reset setup</button>
      <button className="theme-secondary-btn px-3 py-2" data-old-begin onClick={() => void captured.beginBind()}>Old begin</button>
      <button className="theme-secondary-btn px-3 py-2" data-old-reset onClick={() => void captured.resetSetup()}>Old reset</button>
    </div>
    {runtime.bindQrDataUrl && <img className="h-36 w-36" alt="Binding QR" src={runtime.bindQrDataUrl} />}
    <pre className="whitespace-pre-wrap break-all text-xs" id="state">{JSON.stringify(state)}</pre>
  </main>;
}
createRoot(document.getElementById("root")!).render(<UiDialogProvider><Fixture /></UiDialogProvider>);
