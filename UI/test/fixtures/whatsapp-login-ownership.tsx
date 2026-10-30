import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { useWhatsappWebRuntime } from "../../src/hooks/useWhatsappWebRuntime";
import type { ServiceActionNotice } from "../../src/types/api";
import "../../src/index.css";

const query = new URLSearchParams(location.search);
document.documentElement.dataset.theme = query.get("theme") || "light";
let visibility = "hidden";
Object.defineProperty(document, "visibilityState", { configurable: true, get: () => visibility });
function Fixture() {
  const [base, setBase] = useState("https://device-a.example.test");
  const [ready, setReady] = useState(true), [admin, setAdmin] = useState(true), [epoch, setEpoch] = useState(0);
  const [notices, setNotices] = useState<{ scope: string; notice: ServiceActionNotice | null }[]>([]);
  const runtime = useWhatsappWebRuntime({ apiFetch: (path, init) => fetch(`${base}${path}`, init), apiBase: base,
    requestScope: ready ? JSON.stringify([base, admin, epoch]) : "", uiAuthReady: ready,
    whatsappWebHealthy: true, statusPollingEnabled: true, t: (_zh, en) => en,
    setServiceActionMessage: notice => setNotices(values => [...values, { scope: base, notice }]),
  });
  const capturedSetter = useRef(runtime.setWaLoginDialogOpen).current;
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
    const show = (event: Event) => { visibility = (event as CustomEvent<string>).detail; document.dispatchEvent(new Event("visibilitychange")); };
    window.addEventListener("fixture-scope", change); window.addEventListener("fixture-visibility", show);
    return () => { window.removeEventListener("fixture-scope", change); window.removeEventListener("fixture-visibility", show); };
  }, []);
  const state = { scope: [base, ready, admin, epoch], status: runtime.waLoginStatus,
    loading: runtime.waLoginLoading, error: runtime.waLoginError, panel: runtime.waLoginDialogOpen,
    reachable: runtime.waWebBridgeReachable, loggingOut: runtime.waLogoutLoading, notices };
  return <main className="theme-shell min-h-screen space-y-3 p-4">
    <div className="flex flex-wrap gap-2">
      <button className="theme-secondary-btn px-3 py-2" data-read onClick={() => void runtime.fetchWhatsappWebLoginStatus()}>Read status</button>
      <button className="theme-secondary-btn px-3 py-2" data-silent onClick={() => void runtime.fetchWhatsappWebLoginStatus(true)}>Poll status</button>
      <button className="theme-secondary-btn px-3 py-2" data-logout onClick={() => void runtime.logoutWhatsappWeb()}>Sign out</button>
      <button className="theme-secondary-btn px-3 py-2" data-open onClick={() => runtime.setWaLoginDialogOpen(true)}>Open QR</button>
      <button className="theme-secondary-btn px-3 py-2" data-toggle onClick={() => runtime.setWaLoginDialogOpen(value => !value)}>Toggle QR</button>
      <button className="theme-secondary-btn px-3 py-2" data-old-open onClick={() => capturedSetter(true)}>Old open</button>
      <button className="theme-secondary-btn px-3 py-2" data-old-close onClick={() => capturedSetter(false)}>Old close</button>
    </div>
    <pre className="whitespace-pre-wrap break-all text-xs" id="state">{JSON.stringify(state)}</pre>
  </main>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
