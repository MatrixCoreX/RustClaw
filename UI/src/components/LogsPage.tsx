import type { RefObject } from "react";
import { Loader2, RefreshCw } from "lucide-react";

type Translate = (zh: string, en: string) => string;

export interface LogsPageProps {
  t: Translate;
  tSlash: (mixed: string) => string;
  selectedLogFile: string;
  logTailLines: number;
  logFollowTail: boolean;
  logLastUpdated: number | null;
  logLoading: boolean;
  logError: string | null;
  logText: string;
  logContainerRef: RefObject<HTMLPreElement | null>;
  toLocalTime: (value: number | null | undefined) => string;
  onSelectedLogFileChange: (value: string) => void;
  onLogTailLinesChange: (value: number) => void;
  onLogFollowTailChange: (value: boolean) => void;
  onFetchLatestLog: () => void | Promise<void>;
}

export function LogsPage({
  t,
  tSlash,
  selectedLogFile,
  logTailLines,
  logFollowTail,
  logLastUpdated,
  logLoading,
  logError,
  logText,
  logContainerRef,
  toLocalTime,
  onSelectedLogFileChange,
  onLogTailLinesChange,
  onLogFollowTailChange,
  onFetchLatestLog,
}: LogsPageProps) {
  return (
    <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <h3 className="text-base font-semibold">{t("最新日志", "Latest Logs")}</h3>
        <button
          onClick={() => void onFetchLatestLog()}
          disabled={logLoading}
          className="inline-flex items-center justify-center gap-2 rounded-xl bg-white/10 px-3 py-2 text-xs font-medium transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {logLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
          {tSlash("刷新 / Refresh")}
        </button>
      </div>

      <div className="mb-4 grid gap-3 md:grid-cols-4">
        <label className="space-y-2">
          <span className="text-[10px] uppercase tracking-widest text-white/50">{t("日志文件", "Log File")}</span>
          <select
            className="theme-input"
            value={selectedLogFile}
            onChange={(event) => onSelectedLogFileChange(event.target.value)}
          >
            <option value="agent_trace.log">agent_trace.log</option>
            <option value="model_io.log">model_io.log</option>
            <option value="routing.log">routing.log</option>
            <option value="act_plan.log">act_plan.log</option>
            <option value="clawd.log">clawd.log</option>
            <option value="nni.log">nni.log</option>
            <option value="nni-server.log">nni-server.log</option>
            <option value="telegramd.log">telegramd.log</option>
            <option value="whatsappd.log">whatsappd.log</option>
            <option value="whatsapp_webd.log">whatsapp_webd.log</option>
          </select>
        </label>

        <label className="space-y-2">
          <span className="text-[10px] uppercase tracking-widest text-white/50">{t("尾部行数", "Tail Lines")}</span>
          <select
            className="theme-input"
            value={logTailLines}
            onChange={(event) => onLogTailLinesChange(Number(event.target.value))}
          >
            <option value={100}>100</option>
            <option value={200}>200</option>
            <option value={500}>500</option>
            <option value={1000}>1000</option>
          </select>
        </label>

        <div className="flex items-end">
          <label className="inline-flex items-center gap-2 text-sm text-white/80">
            <input type="checkbox" checked={logFollowTail} onChange={(event) => onLogFollowTailChange(event.target.checked)} />
            {t("跟随到底部", "Follow tail")}
          </label>
        </div>

        <div className="flex items-end text-xs text-white/50">
          {logLastUpdated ? `${t("更新时间", "Updated")}: ${toLocalTime(logLastUpdated)}` : t("尚未加载", "Not loaded yet")}
        </div>
      </div>

      {logError ? (
        <p className="mb-4 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {t("日志读取失败", "Log read failed")}: {logError}
        </p>
      ) : null}

      <pre
        ref={logContainerRef}
        className="h-[70vh] overflow-auto rounded-xl border border-white/10 bg-[#12151f] p-3 text-xs text-white/85"
      >
        {logText || t("日志为空", "Log is empty")}
      </pre>
    </section>
  );
}
