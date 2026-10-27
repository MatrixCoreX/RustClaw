import { useState } from "react";
import { createRoot } from "react-dom/client";
import { FileText, Pause, UserRound } from "lucide-react";
import { useTaskRuntime } from "../src/hooks/useTaskRuntime";
import { TaskStatusFacts } from "../src/components/TaskStatusFacts";
import "../src/index.css";

async function fixtureApi(path: string, init?: RequestInit): Promise<Response> {
  const taskId = path.includes("slow") ? "slow" : "fast";
  await new Promise(resolve => setTimeout(resolve, taskId === "slow" ? 350 : 20));
  if (init?.method === "POST") {
    const body = JSON.parse(String(init.body ?? "{}"));
    if (body.idempotency_key && !/[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(body.idempotency_key)) {
      return new Response(JSON.stringify({ ok: false, error: "test_uuid_invalid" }), { status: 400 });
    }
  }
  const data = path === "/v1/tasks/active" ? { tasks: [] }
    : init?.method === "POST" ? { status: "paused" }
      : { task_id: taskId, status: "succeeded", result_json: { task_journal: { summary: {
        execution_status: "succeeded", answer_verification: { status: "passed" },
        delivery: { status: "unavailable", can_reconcile: false },
      } } } };
  return new Response(JSON.stringify({ ok: true, data }));
}

function Fixture() {
  const [owner, setOwner] = useState("one");
  const runtime = useTaskRuntime({ apiFetch: fixtureApi, t: zh => zh,
    apiBase: "fixture", uiAuthReady: true, currentPage: "dashboard",
    interactionUserId: 1, interactionChatId: 2, activeUserKey: owner, activeIdentityIds: {} });
  return <main className="mx-auto max-w-3xl space-y-5 p-4">
    <h1 className="text-lg">任务状态</h1>
    <div className="flex flex-wrap gap-2">
      <button className="theme-btn-secondary inline-flex items-center gap-2 rounded-md border px-3 py-2" onClick={() => void runtime.viewTask("slow")}><FileText size={16} />慢任务</button>
      <button className="theme-btn-secondary inline-flex items-center gap-2 rounded-md border px-3 py-2" onClick={() => void runtime.viewTask("fast")}><FileText size={16} />快任务</button>
      <button className="theme-btn-secondary inline-flex items-center gap-2 rounded-md border px-3 py-2" onClick={() => setOwner(owner === "one" ? "two" : "one")}><UserRound size={16} />切换身份</button>
      <button className="theme-btn-secondary inline-flex items-center gap-2 rounded-md border px-3 py-2" onClick={() => void runtime.controlTaskById("pause", "fast")}><Pause size={16} />暂停任务</button>
    </div>
    <dl className="grid grid-cols-2 gap-2 text-sm">
      <dt>身份</dt><dd data-testid="owner">{owner}</dd>
      <dt>当前结果</dt><dd data-testid="task">{runtime.taskResult?.task_id ?? "none"}</dd>
      <dt>正在读取</dt><dd data-testid="loading">{String(runtime.taskLoading)}</dd>
    </dl>
    {runtime.taskResult && <TaskStatusFacts task={runtime.taskResult} lang="zh" />}
    <output className="block text-sm" data-testid="control">{runtime.taskControlMessage}</output>
    <output className="block text-sm" data-testid="error">{runtime.taskError ?? runtime.taskControlError}</output>
  </main>;
}

createRoot(document.getElementById("root")!).render(<Fixture />);
