import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { UiDialogProvider } from "../../src/components/UiDialogProvider";
import { useSystemRuntime } from "../../src/hooks/useSystemRuntime";
import "../../src/index.css";

document.documentElement.dataset.theme = new URLSearchParams(location.search).get("theme") || "light";
const noop = () => undefined;
function Fixture() {
  const [apiBase, setApiBase] = useState("https://device-a.example.test");
  const [uiAuthReady, setReady] = useState(true);
  const [isAdminIdentity, setAdmin] = useState(true);
  const runtime = useSystemRuntime({
    apiFetch: (path, init) => fetch(`${apiBase}${path}`, init),
    apiBase, uiAuthReady, isAdminIdentity, currentPage: "chat", t: (zh) => zh,
    fetchHealth: async () => undefined, setHealth: noop, setError: noop,
    clearLlmConfigError: noop, clearSkillsConfigError: noop, fetchLlmConfig: noop,
    fetchMultimodalConfig: noop, fetchSkillsConfig: noop, fetchSkills: noop,
  });
  return <main className="theme-shell min-h-screen p-4">
    <div className="flex flex-wrap gap-2">
      <button onClick={() => void runtime.fetchHostSystemSummary()}>Summary</button>
      <button onClick={() => void runtime.fetchHostDependencies()}>Dependencies</button>
      <button onClick={() => void runtime.fetchHostDependencies(true)}>Poll dependencies</button>
      <button onClick={() => setApiBase("https://device-b.example.test")}>Switch device</button>
      <button onClick={() => setReady(false)}>Sign out</button>
      <button onClick={() => setAdmin(false)}>Use ordinary role</button>
    </div>
    <dl className="mt-4 grid grid-cols-2 gap-2">
      <dt>Summary</dt><dd data-testid="summary">{runtime.hostSystemSummary?.architecture || "empty"}</dd>
      <dt>Summary loading</dt><dd data-testid="summary-loading">{String(runtime.hostSystemLoading)}</dd>
      <dt>Summary error</dt><dd data-testid="summary-error">{runtime.hostSystemErrorCode || "none"}</dd>
      <dt>Dependencies</dt><dd data-testid="dependencies">{runtime.hostDependencies?.platform || "empty"}</dd>
      <dt>Dependencies loading</dt><dd data-testid="dependencies-loading">{String(runtime.hostDependenciesLoading)}</dd>
      <dt>Dependencies error</dt><dd data-testid="dependencies-error">{runtime.hostDependenciesErrorCode || "none"}</dd>
    </dl>
  </main>;
}
createRoot(document.getElementById("root")!).render(<UiDialogProvider><Fixture /></UiDialogProvider>);
