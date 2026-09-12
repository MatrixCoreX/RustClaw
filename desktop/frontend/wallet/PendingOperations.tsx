import { copy } from "../i18n";
import { useDesktopAssetAccount } from "./context";

export function PendingOperations() {
  const local = useDesktopAssetAccount();
  const records = local?.runtime.records.filter(r => r.status === "pending") ?? [];
  if (!local || !records.length) return null;
  return <section className="theme-panel-soft mb-4 p-4" aria-label={copy("待核实的资产操作")}>
    <h2 className="font-medium">{copy("有操作需要核实结果")}</h2>
    <p className="mt-1 text-sm text-[var(--theme-text-muted)]">{copy("先查询原操作结果，确认后再进行下一笔交易。")}</p>
    {records.map(record => <div key={record.operation_id} className="mt-3 flex flex-wrap items-center justify-between gap-3">
      <code className="break-all text-xs">{record.operation_id}</code>
      <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={local.runtime.busy}
        onClick={() => void local.runtime.check(record.operation_id)}>{copy("核实结果")}</button>
    </div>)}
  </section>;
}
