import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { useWechatRuntime } from "../../src/hooks/useWechatRuntime";
import "../../src/index.css";

const query = new URLSearchParams(location.search);
document.documentElement.dataset.theme = query.get("theme") || "light";
let visibility = "hidden";
Object.defineProperty(document, "visibilityState", { configurable: true, get: () => visibility });
function Fixture() {
  const [base, setBase] = useState("https://device-a.example.test");
  const [ready, setReady] = useState(true), [admin, setAdmin] = useState(true), [epoch, setEpoch] = useState(0);
  const runtime = useWechatRuntime({ apiFetch: (path, init) => fetch(`${base}${path}`, init), apiBase: base,
    requestScope: ready ? JSON.stringify([base, admin, epoch]) : "", uiAuthReady: ready,
    enabled: true, serviceHealthy: true, statusPollingEnabled: true, t: (_zh, en) => en });
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
    const show = (event: Event) => {
      visibility = (event as CustomEvent<string>).detail;
      document.dispatchEvent(new Event("visibilitychange"));
    };
    window.addEventListener("fixture-scope", change);
    window.addEventListener("fixture-visibility", show);
    return () => { window.removeEventListener("fixture-scope", change); window.removeEventListener("fixture-visibility", show); };
  }, []);
  const state = { scope: [base, ready, admin, epoch], status: runtime.wechatLoginStatus,
    loading: runtime.wechatLoginLoading, error: runtime.wechatLoginError,
    starting: runtime.wechatQrStarting, preview: runtime.wechatQrPreviewRequested };
  return <main className="theme-shell min-h-screen space-y-3 p-4">
    <div className="flex flex-wrap gap-2">
      <button className="theme-secondary-btn px-3 py-2" data-read onClick={() => void runtime.fetchWechatLoginStatus()}>Read status</button>
      <button className="theme-secondary-btn px-3 py-2" data-start="normal" onClick={() => void runtime.startWechatQrLogin(false)}>Start QR</button>
      <button className="theme-secondary-btn px-3 py-2" data-start="force" onClick={() => void runtime.startWechatQrLogin(true)}>Refresh QR</button>
    </div>
    <pre className="whitespace-pre-wrap break-all text-xs" id="state">{JSON.stringify(state)}</pre>
  </main>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
