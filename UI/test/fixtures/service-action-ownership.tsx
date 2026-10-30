import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { UiDialogProvider } from "../../src/components/UiDialogProvider";
import { useServiceActionsRuntime } from "../../src/hooks/useServiceActionsRuntime";
import "../../src/index.css";

const query = new URLSearchParams(location.search);
document.documentElement.dataset.theme = query.get("theme") || "light";
document.documentElement.lang = query.get("lang") || "en";
const services = ["telegramd", "whatsappd", "whatsapp_webd", "wechatd", "feishud", "larkd"] as const;
const actions = ["start", "stop", "restart", "reset"] as const;
function Fixture() {
  const [apiBase, setBase] = useState("https://device-a.example.test");
  const [ready, setReady] = useState(true), [isAdminIdentity, setAdmin] = useState(true), [epoch, setEpoch] = useState(0);
  const [reads, setReads] = useState<string[]>([]), [results, setResults] = useState<boolean[]>([]);
  const requestScope = ready ? JSON.stringify([apiBase, isAdminIdentity, epoch]) : "";
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
  const read = (kind: string) => {
    setReads(values => [...values, `${apiBase}:${kind}`]);
    if (query.get("refresh") === "throw") throw new Error("fixture_refresh_failed");
    return query.get("refresh") === "reject" ? Promise.reject(new Error("fixture_refresh_failed")) : Promise.resolve();
  };
  const runtime = useServiceActionsRuntime({
    apiFetch: (path, init) => fetch(`${apiBase}${path}`, init), requestScope, isAdminIdentity,
    t: (zh, en) => query.get("lang") === "zh" ? zh : en,
    onHealthRefresh: () => read("health"), onConfigRefresh: () => read("config"),
  });
  const fields = {
    scope: `${apiBase}:${ready}:${isAdminIdentity}:${epoch}`,
    notice: runtime.serviceActionMessage?.text || "none", tone: runtime.serviceActionMessage?.tone || "none",
    busy: JSON.stringify(runtime.serviceActionLoading), reads: JSON.stringify(reads), results: JSON.stringify(results),
  };
  return <main className="theme-shell min-h-screen p-4">
    <div className="grid gap-3">{services.map(service => <section key={service} className="flex flex-wrap items-center gap-2">
      <h2 className="text-sm">{service}</h2>{actions.map(action => <button key={action}
        className="theme-secondary-btn px-3 py-2 text-xs" data-action={`${service}:${action}`}
        onClick={() => void runtime.controlService(service, action).then(value => setResults(values => [...values, value]))}>
        {service} {action}</button>)}</section>)}</div>
    <dl className="mt-4 grid grid-cols-[minmax(0,1fr)_minmax(0,3fr)] gap-3">{Object.entries(fields).map(([key, value]) =>
      <React.Fragment key={key}><dt className="break-words">{key}</dt><dd className="break-all" data-testid={key}>{value}</dd></React.Fragment>)}</dl>
  </main>;
}
createRoot(document.getElementById("root")!).render(<UiDialogProvider><Fixture /></UiDialogProvider>);
