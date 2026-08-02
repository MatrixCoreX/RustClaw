export type NniHistoryView = "overview" | "rewards" | "records" | "errors";

type Translate = (zh: string, en: string) => string;

export interface NniHistoryTabsProps {
  activeView: NniHistoryView;
  recordsTotal: number;
  errorsTotal: number;
  rewardsTotal: number;
  t: Translate;
  onChange: (view: NniHistoryView) => void;
}

export function NniHistoryTabs({
  activeView,
  recordsTotal,
  errorsTotal,
  rewardsTotal,
  t,
  onChange,
}: NniHistoryTabsProps) {
  return (
    <div
      className="grid w-full grid-cols-2 gap-2 rounded-2xl border border-white/10 bg-black/20 p-1.5 lg:grid-cols-4"
      role="tablist"
      aria-label={t("NNI 页面", "NNI pages")}
    >
      <button
        id="nni-overview-tab"
        type="button"
        role="tab"
        aria-selected={activeView === "overview"}
        aria-controls="nni-overview-primary-panel nni-overview-actions-panel"
        onClick={() => onChange("overview")}
        className={`${activeView === "overview" ? "theme-accent-btn" : "theme-secondary-btn"} justify-center px-4 py-2 text-sm`}
      >
        <span>{t("设备与运行", "Device & runtime")}</span>
      </button>
      <button
        id="nni-history-rewards-tab"
        type="button"
        role="tab"
        aria-selected={activeView === "rewards"}
        aria-controls="nni-history-rewards-panel"
        onClick={() => onChange("rewards")}
        className={`${activeView === "rewards" ? "theme-accent-btn" : "theme-secondary-btn"} justify-center px-4 py-2 text-sm`}
      >
        <span>{t("心跳奖励", "Heartbeat rewards")}</span>
        <span className="rounded-full border border-current/20 bg-black/15 px-2 py-0.5 text-[11px] font-semibold">
          {rewardsTotal}
        </span>
      </button>
      <button
        id="nni-history-records-tab"
        type="button"
        role="tab"
        aria-selected={activeView === "records"}
        aria-controls="nni-history-records-panel"
        onClick={() => onChange("records")}
        className={`${activeView === "records" ? "theme-accent-btn" : "theme-secondary-btn"} justify-center px-4 py-2 text-sm`}
      >
        <span>{t("请求记录", "Request records")}</span>
        <span className="rounded-full border border-current/20 bg-black/15 px-2 py-0.5 text-[11px] font-semibold">
          {recordsTotal}
        </span>
      </button>
      <button
        id="nni-history-errors-tab"
        type="button"
        role="tab"
        aria-selected={activeView === "errors"}
        aria-controls="nni-history-errors-panel"
        onClick={() => onChange("errors")}
        className={`${activeView === "errors" ? "theme-accent-btn" : "theme-secondary-btn"} justify-center px-4 py-2 text-sm`}
      >
        <span>{t("心跳错误", "Heartbeat errors")}</span>
        <span className="rounded-full border border-current/20 bg-black/15 px-2 py-0.5 text-[11px] font-semibold">
          {errorsTotal}
        </span>
      </button>
    </div>
  );
}
