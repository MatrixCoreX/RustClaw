import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { UiDialogProvider, useUiDialog } from "../../src/components/UiDialogProvider";
import { useSystemRuntime } from "../../src/hooks/useSystemRuntime";
import "../../src/index.css";

const query = new URLSearchParams(location.search);
document.documentElement.dataset.theme = query.get("theme") || "light";
document.documentElement.lang = query.get("lang") || "en";
const noop = () => undefined;
function Fixture() {
  const dialog = useUiDialog();
  const [apiBase, setBase] = useState("https://device-a.example.test");
  const [uiAuthReady, setReady] = useState(true);
  const [isAdminIdentity, setAdmin] = useState(true);
  const [epoch, setEpoch] = useState(0);
  const [health, setHealth] = useState("none");
  const [configReads, setConfigReads] = useState(0);
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
  const configRead = () => { setConfigReads(value => value + 1); };
  const runtime = useSystemRuntime({
    apiFetch: (path, init) => fetch(`${apiBase}${path}`, init), apiBase, uiAuthReady, isAdminIdentity,
    requestEpoch: epoch, currentPage: "chat", t: (zh, en) => query.get("lang") === "zh" ? zh : en,
    fetchHealth: async () => undefined, setHealth: data => setHealth(data.version), setError: noop,
    clearLlmConfigError: noop, clearSkillsConfigError: noop, fetchLlmConfig: configRead,
    fetchMultimodalConfig: configRead, fetchSkillsConfig: configRead, fetchSkills: configRead,
  });
  const fields = {
    scope: `${apiBase}:${uiAuthReady}:${isAdminIdentity}:${epoch}`,
    "workspace-version": runtime.workspaceUpdateStatus?.current_version || "empty",
    "workspace-error": runtime.workspaceUpdateMessage || "none",
    "workspace-loading": String(runtime.workspaceUpdateLoading),
    "workspace-canceling": String(runtime.workspaceUpdateCanceling),
    "webd-platform": runtime.webdExposureStatus?.platform || "empty",
    "webd-message": runtime.webdExposureMessage || "none",
    "webd-error": runtime.webdExposureError || "none",
    "webd-updating": String(runtime.webdExposureUpdating),
    "restart-message": runtime.systemRestartMessage || "none",
    "restart-busy": String(runtime.systemRestarting), health, "config-reads": String(configReads),
    "diagnostic-error": runtime.systemDiagnosticsExportError || "none",
  };
  const buttons: Record<string, () => unknown> = {
    "Prompt": () => dialog.prompt({ title: "Fixture prompt", message: "Fixture prompt message", inputLabel: "Fixture label" }),
    "Expose port": () => runtime.setWebdExternalAccess(true),
    "Build UI": () => runtime.startWorkspaceUpdate("ui_only"),
    "Build all": () => runtime.startWorkspaceUpdate("full"),
    "Stop update": runtime.cancelWorkspaceUpdate,
    "Read workspace": runtime.fetchWorkspaceUpdateStatus,
    "Read webd": runtime.fetchWebdExposureStatus,
    "Restart system": runtime.restartSystem,
    "Export diagnostics": runtime.exportSystemDiagnostics,
  };
  return <main className="theme-shell min-h-screen p-4">
    <div className="flex flex-wrap gap-2">{Object.entries(buttons).map(([label, call]) =>
      <button key={label} className="theme-secondary-btn px-3 py-2" onClick={() => void call()}>{label}</button>)}</div>
    <dl className="mt-4 grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)] gap-3">{Object.entries(fields).map(([key, value]) =>
      <React.Fragment key={key}><dt className="break-words">{key}</dt><dd className="break-words" data-testid={key}>{value}</dd></React.Fragment>)}</dl>
  </main>;
}
createRoot(document.getElementById("root")!).render(<UiDialogProvider><Fixture /></UiDialogProvider>);
