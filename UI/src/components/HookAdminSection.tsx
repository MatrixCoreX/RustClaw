import { CircleAlert, CircleCheck, Loader2, RefreshCw, ShieldCheck } from "lucide-react";

import type { HookAdminStatus } from "../types/api";

type Translate = (zh: string, en: string) => string;

export interface HookAdminSectionProps {
  t: Translate;
  canManage: boolean;
  status: HookAdminStatus | null;
  loading: boolean;
  error: string | null;
  onRefresh: () => unknown | Promise<unknown>;
}

function statusTone(status: string): string {
  if (status === "ready") return "border-emerald-400/30 bg-emerald-500/10 text-emerald-100";
  if (status === "invalid") return "border-red-400/30 bg-red-500/10 text-red-100";
  return "border-white/10 bg-white/5 text-white/60";
}

export function HookAdminSection({
  t,
  canManage,
  status,
  loading,
  error,
  onRefresh,
}: HookAdminSectionProps) {
  if (!canManage) return null;

  return (
    <section className="mt-4 scroll-mt-20 border-t border-white/10 pt-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="max-w-2xl">
          <p className="theme-kicker text-[10px] uppercase tracking-normal">Hooks</p>
          <h2 className="mt-1 text-lg font-semibold">{t("生命周期扩展", "Lifecycle extensions")}</h2>
          <p className="mt-2 text-sm leading-6 text-white/60">
            {t(
              "可信 Hook 可以观察或阻止机器动作。默认保持关闭；启用前必须由维护者核对来源、文件哈希和权限范围。",
              "Trusted hooks can observe or block machine actions. Keep them disabled by default; a maintainer must review provenance, content hash, and permission scope before enabling one.",
            )}
          </p>
        </div>
        <button
          type="button"
          className="theme-secondary-btn px-3 py-2 text-xs"
          onClick={() => void onRefresh()}
          disabled={loading}
        >
          {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
          {t("刷新状态", "Refresh status")}
        </button>
      </div>

      {error ? (
        <p className="mt-4 flex items-start gap-2 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-100">
          <CircleAlert className="mt-0.5 h-4 w-4 shrink-0" />
          {t("无法读取 Hook 状态：", "Unable to read hook status: ")}
          <span className="font-mono text-xs">{error}</span>
        </p>
      ) : null}

      {status ? (
        <>
          <div className="mt-5 grid gap-px overflow-hidden rounded-md border border-white/10 bg-white/10 sm:grid-cols-4">
            {[
              [t("配置数量", "Configured"), status.handler_count],
              [t("已启用", "Enabled"), status.enabled_handler_count],
              [t("验证通过", "Ready"), status.valid_handler_count],
              [t("需要处理", "Needs attention"), status.invalid_handler_count],
            ].map(([label, value]) => (
              <div key={String(label)} className="bg-[var(--theme-card-strong)] px-3 py-3">
                <p className="text-xs text-white/50">{label}</p>
                <p className="mt-1 text-lg font-semibold">{value}</p>
              </div>
            ))}
          </div>

          <div className="mt-4 flex items-start gap-3 rounded-md border border-white/10 bg-white/5 px-3 py-3">
            {status.fail_closed ? (
              <CircleAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-200" />
            ) : (
              <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-emerald-200" />
            )}
            <div className="min-w-0">
              <p className="text-sm font-medium">
                {status.setup_state === "disabled_baseline" || status.setup_state === "configured_disabled"
                  ? t("当前未启用 Hook", "Hooks are currently disabled")
                  : status.fail_closed
                    ? t("配置需要维护者处理", "Configuration needs maintainer attention")
                    : t("启用的 Hook 已通过验证", "Enabled hooks passed validation")}
              </p>
              <p className="mt-1 break-words text-xs text-white/50">
                {t("配置入口", "Configuration")}: <span className="font-mono">{status.config_path}</span>
                {status.config_error_code ? ` · ${status.config_error_code}` : ""}
              </p>
              {!status.setup.ui_enable_supported ? (
                <p className="mt-1 text-xs text-white/45">
                  {t(
                    "此页面只读取和验证状态，不会替你授予信任或启用脚本。",
                    "This page reads and validates status only; it never grants trust or enables a script.",
                  )}
                </p>
              ) : null}
            </div>
          </div>

          <div className="mt-5 divide-y divide-white/10 border-y border-white/10">
            {status.handlers.length === 0 ? (
              <div className="flex min-h-24 items-center justify-center gap-2 px-4 py-6 text-center text-sm text-white/55">
                <CircleCheck className="h-4 w-4 text-emerald-200" />
                {t("没有配置 Handler，运行时保持默认安全行为。", "No handlers are configured; runtime stays on the safe default.")}
              </div>
            ) : null}
            {status.handlers.map((handler) => (
              <div key={handler.id} className="py-4">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <p className="font-mono text-sm font-medium">{handler.id}</p>
                    <p className="mt-1 text-xs text-white/50">
                      {handler.stage} · {handler.kind} · {handler.blocking ? "blocking" : "observe"}
                    </p>
                  </div>
                  <span className={`rounded-md border px-2 py-1 text-xs ${statusTone(handler.status)}`}>
                    {handler.status}
                  </span>
                </div>
                <div className="mt-3 flex flex-wrap gap-2 text-xs text-white/55">
                  <span>trust={handler.trust_status}</span>
                  <span>hash={handler.content_hash_configured ? "configured" : "missing"}</span>
                  {handler.error_code ? <span className="text-red-200">error={handler.error_code}</span> : null}
                </div>
                <details className="mt-3 rounded-md border border-white/10 bg-black/20 p-3">
                  <summary className="cursor-pointer text-xs font-medium text-white/60">
                    {t("查看脱敏机器配置", "View redacted machine config")}
                  </summary>
                  <pre className="mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-all text-[11px] leading-5 text-white/65">
                    {JSON.stringify(handler.redacted_config, null, 2)}
                  </pre>
                </details>
              </div>
            ))}
          </div>
        </>
      ) : null}
    </section>
  );
}
