import { useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import type { MemoryTraceEvidenceRow } from "../lib/task-llm-trace";

type Translate = (zh: string, en: string) => string;
const PAGE_SIZE = 12;

export function TaskEvidenceList({ rows, t }: { rows: MemoryTraceEvidenceRow[]; t: Translate }) {
  const [requestedPage, setPage] = useState(0);
  const pages = Math.ceil(rows.length / PAGE_SIZE);
  const page = Math.min(requestedPage, Math.max(0, pages - 1));
  const start = page * PAGE_SIZE;
  if (rows.length === 0) return (
    <p className="mt-3 text-[11px] text-white/45">
      {t("本轮没有纳入记忆或知识库证据。", "No memory or knowledge-base evidence was included in this turn.")}
    </p>
  );
  return (
    <div className="mt-3 min-w-0">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-[11px] text-white/65">
        <span>{t("已纳入证据", "Included evidence")}: {rows.length}</span>
        <span aria-live="polite">{start + 1}–{Math.min(start + PAGE_SIZE, rows.length)} / {rows.length}</span>
      </div>
      <div className="grid gap-2">
        {rows.slice(start, start + PAGE_SIZE).map((item, index) => (
          <details key={`${start + index}:${item.sourceKind}:${item.sourceRef}`} className="min-w-0 rounded-md border border-white/10 bg-black/20 px-3 py-2 text-[11px]">
            <summary className="cursor-pointer break-all text-emerald-100">
              <span className="mr-2 text-white/45">#{start + index + 1}</span>
              {item.sourceKind}
              <span className="ml-2 font-mono text-white/55">{item.score == null ? "score=--" : `score=${item.score.toFixed(4)}`}</span>
            </summary>
            <dl className="mt-2 grid min-w-0 gap-1">
              <dt className="text-white/45">{t("完整来源引用", "Full source reference")}</dt>
              <dd className="select-text whitespace-pre-wrap break-all font-mono text-white/75">{item.sourceRef}</dd>
              {item.reason ? <>
                <dt className="mt-1 text-white/45">{t("纳入依据", "Included because")}</dt>
                <dd className="whitespace-pre-wrap break-words text-white/65">{item.reason}</dd>
              </> : null}
            </dl>
          </details>
        ))}
      </div>
      {pages > 1 ? (
        <nav aria-label={t("证据分页", "Evidence pages")} className="mt-2 flex items-center justify-end gap-2">
          <button type="button" title={t("上一页证据", "Previous evidence page")} aria-label={t("上一页证据", "Previous evidence page")} disabled={page === 0} onClick={() => setPage(page - 1)} className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-white/15 disabled:opacity-35">
            <ChevronLeft className="h-4 w-4" />
          </button>
          <span className="text-[11px] text-white/65">{page + 1} / {pages}</span>
          <button type="button" title={t("下一页证据", "Next evidence page")} aria-label={t("下一页证据", "Next evidence page")} disabled={page + 1 === pages} onClick={() => setPage(page + 1)} className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-white/15 disabled:opacity-35">
            <ChevronRight className="h-4 w-4" />
          </button>
        </nav>
      ) : null}
    </div>
  );
}
