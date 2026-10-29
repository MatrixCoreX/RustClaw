import { useState } from "react";
import { createRoot } from "react-dom/client";
import { ArrowLeftRight } from "lucide-react";
import { TaskEvidenceList } from "../../src/components/TaskEvidenceList";
import "../../src/index.css";

const rows = Array.from({ length: 25 }, (_, i) => ({
  sourceKind: "knowledge", sourceRef: `kb:fixture/${i}:` + "long-reference-".repeat(24),
  score: 0.5, reason: "An exact retained reference from the selected task.",
}));
function Fixture() {
  const [task, setTask] = useState(1);
  return <main className="mx-auto max-w-3xl p-4">
    <button className="theme-btn-secondary inline-flex items-center gap-2 rounded-md border px-3 py-2" onClick={() => setTask(task + 1)}><ArrowLeftRight size={16} />Switch task</button>
    <section className="mt-3 rounded-md border border-white/10 bg-black/20 p-3">
      <h1 className="text-sm">Memory and KB context policy</h1>
      <TaskEvidenceList key={task} rows={rows} t={(_zh, en) => en} />
    </section>
  </main>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
