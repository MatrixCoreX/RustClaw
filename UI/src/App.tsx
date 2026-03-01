import { useEffect, useMemo, useState } from "react";
import {
  AlertCircle,
  CheckCircle2,
  Clock3,
  Database,
  Loader2,
  RefreshCw,
  Server,
  Timer,
} from "lucide-react";

interface ApiResponse<T> {
  ok: boolean;
  data?: T;
  error?: string;
}

interface HealthResponse {
  version: string;
  queue_length: number;
  worker_state: string;
  uptime_seconds: number;
  memory_rss_bytes?: number | null;
  running_length: number;
  task_timeout_seconds: number;
  running_oldest_age_seconds: number;
  telegramd_healthy?: boolean | null;
  telegramd_process_count?: number | null;
}

interface TaskQueryResponse {
  task_id: string;
  status: "queued" | "running" | "succeeded" | "failed" | "canceled" | "timeout";
  result_json?: unknown | null;
  error_text?: string | null;
}

interface Snapshot {
  ts: number;
  queue: number;
  running: number;
  memory: number | null;
}

function formatBytes(value?: number | null): string {
  if (value == null || Number.isNaN(value)) return "--";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = value;
  let idx = 0;
  while (size >= 1024 && idx < units.length - 1) {
    size /= 1024;
    idx += 1;
  }
  return `${size.toFixed(idx === 0 ? 0 : 2)} ${units[idx]}`;
}

function formatDuration(totalSeconds?: number): string {
  if (typeof totalSeconds !== "number" || Number.isNaN(totalSeconds)) return "--";
  const days = Math.floor(totalSeconds / 86400);
  const hours = Math.floor((totalSeconds % 86400) / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = Math.floor(totalSeconds % 60);
  if (days > 0) return `${days}d ${hours}h ${minutes}m`;
  if (hours > 0) return `${hours}h ${minutes}m ${seconds}s`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

function toLocalTime(ts: number): string {
  return new Date(ts).toLocaleTimeString();
}

function StatCard({
  title,
  value,
  hint,
}: {
  title: string;
  value: string | number;
  hint?: string;
}) {
  return (
    <div className="rounded-2xl border border-white/10 bg-white/5 p-5">
      <p className="text-xs uppercase tracking-widest text-white/50">{title}</p>
      <p className="mt-2 text-2xl font-bold text-white">{value}</p>
      {hint ? <p className="mt-1 text-xs text-white/50">{hint}</p> : null}
    </div>
  );
}

export default function App() {
  const [baseUrl, setBaseUrl] = useState("http://127.0.0.1:8787");
  const [pollingSeconds, setPollingSeconds] = useState(5);
  const [loading, setLoading] = useState(false);
  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdated, setLastUpdated] = useState<number | null>(null);
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);

  const [taskId, setTaskId] = useState("");
  const [taskLoading, setTaskLoading] = useState(false);
  const [taskResult, setTaskResult] = useState<TaskQueryResponse | null>(null);
  const [taskError, setTaskError] = useState<string | null>(null);

  const isOnline = Boolean(health) && !error;

  const fetchHealth = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch(`${baseUrl.replace(/\/$/, "")}/v1/health`);
      const body = (await res.json()) as ApiResponse<HealthResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `health 请求失败 (${res.status})`);
      }
      setHealth(body.data);
      setLastUpdated(Date.now());
      setSnapshots((prev) => {
        const next: Snapshot[] = [
          ...prev,
          {
            ts: Date.now(),
            queue: body.data.queue_length,
            running: body.data.running_length,
            memory: body.data.memory_rss_bytes ?? null,
          },
        ];
        return next.slice(-24);
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setError(message);
    } finally {
      setLoading(false);
    }
  };

  const queryTask = async () => {
    if (!taskId.trim()) return;
    setTaskLoading(true);
    setTaskError(null);
    setTaskResult(null);
    try {
      const res = await fetch(`${baseUrl.replace(/\/$/, "")}/v1/tasks/${taskId.trim()}`);
      const body = (await res.json()) as ApiResponse<TaskQueryResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `任务查询失败 (${res.status})`);
      }
      setTaskResult(body.data);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setTaskError(message);
    } finally {
      setTaskLoading(false);
    }
  };

  useEffect(() => {
    void fetchHealth();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (pollingSeconds <= 0) return;
    const timer = window.setInterval(() => {
      void fetchHealth();
    }, pollingSeconds * 1000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [baseUrl, pollingSeconds]);

  const timeline = useMemo(() => snapshots.slice().reverse(), [snapshots]);

  return (
    <div className="min-h-screen bg-[#0f1116] text-white selection:bg-[#f74c00]/30">
      <header className="sticky top-0 z-40 border-b border-white/10 bg-[#0f1116]/90 backdrop-blur px-6 py-4">
        <div className="mx-auto flex max-w-7xl flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-2xl font-bold tracking-tight">RustClaw 运行监控</h1>
            <p className="mt-1 text-sm text-white/60">实时查看 clawd 健康状态、任务队列与服务运行信息</p>
          </div>

          <div className="flex items-center gap-3 rounded-xl border border-white/10 bg-white/5 px-4 py-2">
            {isOnline ? (
              <CheckCircle2 className="h-4 w-4 text-emerald-400" />
            ) : (
              <AlertCircle className="h-4 w-4 text-red-400" />
            )}
            <span className="text-sm">{isOnline ? "在线" : "离线/异常"}</span>
            {lastUpdated ? (
              <span className="text-xs text-white/50">更新于 {toLocalTime(lastUpdated)}</span>
            ) : null}
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-7xl space-y-6 p-6">
        <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
          <div className="grid gap-4 md:grid-cols-[2fr_1fr_1fr_auto]">
            <label className="space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">clawd API 地址</span>
              <input
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder="http://127.0.0.1:8787"
              />
            </label>

            <label className="space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">自动刷新</span>
              <select
                className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
                value={pollingSeconds}
                onChange={(e) => setPollingSeconds(Number(e.target.value))}
              >
                <option value={3}>3 秒</option>
                <option value={5}>5 秒</option>
                <option value={10}>10 秒</option>
                <option value={0}>关闭</option>
              </select>
            </label>

            <div className="flex items-end">
              <button
                onClick={() => void fetchHealth()}
                disabled={loading}
                className="inline-flex w-full items-center justify-center gap-2 rounded-xl bg-[#f74c00] px-4 py-2 font-medium text-white transition hover:bg-[#ff5c1a] disabled:cursor-not-allowed disabled:opacity-60"
              >
                {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                立即刷新
              </button>
            </div>

            <div className="flex items-end text-xs text-white/50">
              {pollingSeconds > 0 ? `每 ${pollingSeconds}s 自动轮询` : "自动轮询已关闭"}
            </div>
          </div>
          {error ? (
            <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
              接口错误：{error}
            </p>
          ) : null}
        </section>

        <section className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          <StatCard title="服务版本" value={health?.version || "--"} />
          <StatCard title="运行时长" value={formatDuration(health?.uptime_seconds)} />
          <StatCard title="队列任务数" value={health?.queue_length ?? "--"} hint="status=queued" />
          <StatCard title="执行中任务数" value={health?.running_length ?? "--"} hint="status=running" />
          <StatCard title="最久运行任务" value={formatDuration(health?.running_oldest_age_seconds)} />
          <StatCard title="任务超时阈值" value={formatDuration(health?.task_timeout_seconds)} />
          <StatCard title="进程内存 RSS" value={formatBytes(health?.memory_rss_bytes ?? null)} />
          <StatCard title="Worker 状态" value={health?.worker_state || "--"} />
        </section>

        <section className="grid gap-6 lg:grid-cols-2">
          <div className="rounded-2xl border border-white/10 bg-white/5 p-5">
            <h2 className="mb-4 flex items-center gap-2 text-lg font-semibold">
              <Server className="h-5 w-5 text-[#f74c00]" />
              服务健康
            </h2>
            <div className="space-y-3">
              <div className="flex items-center justify-between rounded-xl border border-white/10 bg-black/20 px-4 py-3">
                <div className="flex items-center gap-2">
                  <Database className="h-4 w-4 text-white/70" />
                  <span>clawd /v1/health</span>
                </div>
                <span className={isOnline ? "text-emerald-300" : "text-red-300"}>
                  {isOnline ? "正常" : "不可达"}
                </span>
              </div>

              <div className="flex items-center justify-between rounded-xl border border-white/10 bg-black/20 px-4 py-3">
                <div className="flex items-center gap-2">
                  <Server className="h-4 w-4 text-white/70" />
                  <span>telegramd</span>
                </div>
                <span
                  className={
                    health?.telegramd_healthy === true
                      ? "text-emerald-300"
                      : health?.telegramd_healthy === false
                        ? "text-amber-300"
                        : "text-white/50"
                  }
                >
                  {health?.telegramd_healthy === true
                    ? "运行中"
                    : health?.telegramd_healthy === false
                      ? "未检测到"
                      : "未知"}
                </span>
              </div>

              <div className="flex items-center justify-between rounded-xl border border-white/10 bg-black/20 px-4 py-3">
                <div className="flex items-center gap-2">
                  <Timer className="h-4 w-4 text-white/70" />
                  <span>telegramd 进程数</span>
                </div>
                <span className="font-mono text-white/80">
                  {health?.telegramd_process_count == null ? "--" : health.telegramd_process_count}
                </span>
              </div>
            </div>
          </div>

          <div className="rounded-2xl border border-white/10 bg-white/5 p-5">
            <h2 className="mb-4 flex items-center gap-2 text-lg font-semibold">
              <Clock3 className="h-5 w-5 text-[#f74c00]" />
              最近采样（最多 24 条）
            </h2>
            <div className="max-h-[280px] overflow-auto rounded-xl border border-white/10 bg-black/20">
              <table className="w-full text-sm">
                <thead className="sticky top-0 bg-[#151923] text-left text-white/60">
                  <tr>
                    <th className="px-3 py-2">时间</th>
                    <th className="px-3 py-2">队列</th>
                    <th className="px-3 py-2">运行中</th>
                    <th className="px-3 py-2">内存</th>
                  </tr>
                </thead>
                <tbody>
                  {timeline.length === 0 ? (
                    <tr>
                      <td className="px-3 py-4 text-white/40" colSpan={4}>
                        暂无采样数据
                      </td>
                    </tr>
                  ) : (
                    timeline.map((item) => (
                      <tr key={item.ts} className="border-t border-white/5">
                        <td className="px-3 py-2 font-mono text-white/70">{toLocalTime(item.ts)}</td>
                        <td className="px-3 py-2 text-white/80">{item.queue}</td>
                        <td className="px-3 py-2 text-white/80">{item.running}</td>
                        <td className="px-3 py-2 text-white/80">{formatBytes(item.memory)}</td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
          </div>
        </section>

        <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
          <h2 className="mb-4 text-lg font-semibold">任务查询</h2>
          <div className="grid gap-4 md:grid-cols-[1fr_auto]">
            <input
              className="w-full rounded-xl border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none ring-[#f74c00] focus:ring-2"
              placeholder="输入 task_id（UUID）"
              value={taskId}
              onChange={(e) => setTaskId(e.target.value)}
            />
            <button
              onClick={() => void queryTask()}
              disabled={taskLoading || !taskId.trim()}
              className="inline-flex items-center justify-center gap-2 rounded-xl bg-white/10 px-4 py-2 text-sm font-medium transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {taskLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
              查询任务
            </button>
          </div>

          {taskError ? (
            <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
              查询失败：{taskError}
            </p>
          ) : null}

          {taskResult ? (
            <div className="mt-4 rounded-xl border border-white/10 bg-black/30 p-4 text-sm">
              <p className="mb-1 text-white/60">任务 ID</p>
              <p className="font-mono text-white">{taskResult.task_id}</p>
              <div className="mt-3 grid gap-3 md:grid-cols-2">
                <div>
                  <p className="mb-1 text-white/60">状态</p>
                  <p className="inline-block rounded-md bg-[#f74c00]/20 px-2 py-1 font-mono text-[#ffb08a]">
                    {taskResult.status}
                  </p>
                </div>
                <div>
                  <p className="mb-1 text-white/60">错误信息</p>
                  <p className="text-red-200">{taskResult.error_text || "--"}</p>
                </div>
              </div>
              <p className="mb-1 mt-4 text-white/60">结果 JSON</p>
              <pre className="max-h-72 overflow-auto rounded-lg border border-white/10 bg-[#12151f] p-3 text-xs text-white/80">
                {JSON.stringify(taskResult.result_json ?? null, null, 2)}
              </pre>
            </div>
          ) : null}
        </section>
      </main>
    </div>
  );
}
