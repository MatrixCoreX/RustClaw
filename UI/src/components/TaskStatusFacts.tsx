import { CheckCircle2, CircleHelp, Clock3, TriangleAlert } from "lucide-react";
import { buildTaskStatusFacts } from "../lib/task-status-facts";
import type { TaskLifecycleLang } from "../lib/task-lifecycle";
import type { TaskQueryResponse } from "../types/api";

export function TaskStatusFacts({ task, lang }: { task: TaskQueryResponse; lang: TaskLifecycleLang }) {
  const facts = buildTaskStatusFacts(task, lang);
  if (!facts.length) return null;
  return <dl className="my-3 grid grid-cols-1 gap-x-6 gap-y-2 text-xs sm:grid-cols-3" aria-label={lang === "zh" ? "任务状态" : "Task status"}>
    {facts.map(fact => {
      const Icon = fact.tone === "ok" ? CheckCircle2 : fact.tone === "running" ? Clock3 : fact.tone === "attention" ? TriangleAlert : CircleHelp;
      return <div key={fact.kind} className="min-w-0">
        <dt className="text-white/50">{fact.label}</dt>
        <dd className="mt-1 flex items-start gap-1.5 break-words text-white/85">
          <Icon className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          <span>{fact.text}</span>
        </dd>
      </div>;
    })}
  </dl>;
}
