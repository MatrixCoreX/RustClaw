import { useState } from "react";
import { createRoot } from "react-dom/client";
import { TaskDeliveryRecovery } from "../src/components/TaskDeliveryRecovery";
import { TaskStatusFacts } from "../src/components/TaskStatusFacts";
import { UiDialogProvider } from "../src/components/UiDialogProvider";
import type { TaskQueryResponse } from "../src/types/api";
import "../src/index.css";

function Fixture() {
  const [decision, setDecision] = useState("");
  const task: TaskQueryResponse = {
    task_id: "00000000-0000-4000-8000-000000000001", status: "succeeded", error_text: null,
    result_json: { task_journal: { summary: {
      execution_status: "succeeded", answer_verification: { status: "passed" },
      delivery: { status: "query_required", can_reconcile: true, snapshot_digest: "a".repeat(64), unknown_count: 1, retryable_count: 0 },
    } } },
  };
  return <UiDialogProvider><main className="mx-auto max-w-4xl p-4">
    <h1 className="text-lg">投送状态</h1>
    <TaskStatusFacts task={task} lang="zh" />
    <TaskDeliveryRecovery task={task} lang="zh" busy={false} onAction={(_id, op, digest) => setDecision(`${op}:${digest}`)} />
    <output className="block break-all" data-testid="decision">{decision}</output>
  </main></UiDialogProvider>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
