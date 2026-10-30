import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { UiDialogProvider } from "../../src/components/UiDialogProvider";
import { useSystemRuntime } from "../../src/hooks/useSystemRuntime";
import type { ConsolePage } from "../../src/types/api";
import "../../src/index.css";

document.documentElement.dataset.theme = new URLSearchParams(location.search).get("theme") || "light";
const noop = () => undefined;
function Fixture() {
  const [apiBase, setBase] = useState("https://device-a.example.test");
  const [uiAuthReady, setReady] = useState(true);
  const [isAdminIdentity, setAdmin] = useState(true);
  const [currentPage, setPage] = useState<ConsolePage>("chat");
  const runtime = useSystemRuntime({
    apiFetch: (path, init) => fetch(`${apiBase}${path}`, init), apiBase, uiAuthReady, isAdminIdentity, currentPage,
    t: (zh) => zh, fetchHealth: async () => undefined, setHealth: noop, setError: noop,
    clearLlmConfigError: noop, clearSkillsConfigError: noop, fetchLlmConfig: noop,
    fetchMultimodalConfig: noop, fetchSkillsConfig: noop, fetchSkills: noop,
  });
  const fields = {
    "workspace-version": runtime.workspaceUpdateStatus?.current_version || "empty",
    "workspace-loading": String(runtime.workspaceUpdateLoading), "workspace-error": runtime.workspaceUpdateMessage || "none",
    "nginx-platform": runtime.nginxStatus?.platform || "empty", "nginx-error": runtime.nginxStatusError || "none",
    "webd-platform": runtime.webdExposureStatus?.platform || "empty", "webd-error": runtime.webdExposureError || "none",
    "hardware-model": runtime.piAppStatus?.model || "empty",
  };
  return <main className="theme-shell min-h-screen p-4">
    <div className="flex flex-wrap gap-2">
      <button onClick={() => void runtime.fetchWorkspaceUpdateStatus()}>Workspace</button>
      <button onClick={() => void runtime.fetchNginxStatus()}>Nginx</button>
      <button onClick={() => void runtime.fetchWebdExposureStatus()}>Webd</button>
      <button onClick={() => void runtime.fetchWorkspaceUpdateStatus(true)}>Poll workspace</button>
      <button onClick={() => setBase("https://device-b.example.test")}>Switch device</button>
      <button onClick={() => setReady(false)}>Sign out</button>
      <button onClick={() => setAdmin(false)}>Ordinary role</button>
      <button onClick={() => setPage("chat")}>Open chat</button>
      <button onClick={() => setPage("dashboard")}>Open dashboard</button>
    </div>
    <dl className="mt-4 grid grid-cols-2 gap-3">{Object.entries(fields).map(([key, value]) =>
      <React.Fragment key={key}><dt className="break-words">{key}</dt><dd data-testid={key}>{value}</dd></React.Fragment>)}</dl>
  </main>;
}
createRoot(document.getElementById("root")!).render(<UiDialogProvider><Fixture /></UiDialogProvider>);
