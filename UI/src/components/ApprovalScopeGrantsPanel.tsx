import { Loader2, RefreshCw, ShieldCheck, Trash2 } from "lucide-react";

import type { ApprovalScopeGrantView } from "../types/api";

type Translate = (zh: string, en: string) => string;

export interface ApprovalScopeGrantsPanelProps {
  t: Translate;
  grants: ApprovalScopeGrantView[];
  loading: boolean;
  error: string | null;
  revokingGrantId: string | null;
  onRefresh: () => unknown | Promise<unknown>;
  onRevoke: (grantId: string) => unknown | Promise<unknown>;
}

export function ApprovalScopeGrantsPanel({
  t,
  grants,
  loading,
  error,
  revokingGrantId,
  onRefresh,
  onRevoke,
}: ApprovalScopeGrantsPanelProps) {
  const nowSeconds = Date.now() / 1000;
  const activeGrants = grants.filter(
    (grant) => grant.revoked_at == null && grant.expires_at > nowSeconds,
  );

  return (
    <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="text-lg font-semibold">{t("会话授权", "Session approvals")}</h3>
          <p className="mt-1 text-sm text-white/60">
            {t(
              "这里仅显示仍有效的限定授权。每项只覆盖原会话、相同操作和相同资源。",
              "Only active bounded approvals appear here. Each covers the original session, operation, and exact resources.",
            )}
          </p>
        </div>
        <button
          type="button"
          onClick={() => void onRefresh()}
          disabled={loading}
          className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
          title={t("刷新授权", "Refresh approvals")}
        >
          {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
          {t("刷新", "Refresh")}
        </button>
      </div>

      {error ? (
        <p className="mt-3 rounded-md border border-red-400/25 bg-red-500/10 px-3 py-2 text-sm text-red-100">
          {error}
        </p>
      ) : null}

      {activeGrants.length === 0 ? (
        <div className="mt-4 flex items-center gap-2 text-sm text-white/55">
          <ShieldCheck className="h-4 w-4" />
          {t("当前没有有效的会话授权。", "There are no active session approvals.")}
        </div>
      ) : (
        <div className="mt-4 grid gap-3">
          {activeGrants.map((grant) => {
            const resources =
              grant.scope?.entries?.flatMap((entry) => entry.resources ?? []) ?? [];
            const capabilities =
              grant.scope?.entries?.flatMap((entry) =>
                entry.capability ? [entry.capability] : [],
              ) ?? [];
            const revoking = revokingGrantId === grant.grant_id;
            return (
              <div key={grant.grant_id} className="rounded-lg border border-white/10 bg-black/20 px-3 py-3">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0">
                    <p className="font-medium">
                      {capabilities.join(", ") || grant.scope_kind}
                    </p>
                    <p className="mt-1 break-all font-mono text-[11px] text-white/45">
                      {grant.grant_id}
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => void onRevoke(grant.grant_id)}
                    disabled={revoking}
                    className="inline-flex items-center justify-center gap-2 rounded-md border border-red-300/20 bg-red-500/10 px-3 py-2 text-xs font-medium text-red-100 transition hover:bg-red-500/15 disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {revoking ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}
                    {t("撤销", "Revoke")}
                  </button>
                </div>
                <div className="mt-2 flex flex-wrap gap-2 text-[11px] text-white/65">
                  {resources.map((resource) => (
                    <span key={resource} className="rounded-md border border-white/10 px-2 py-1 font-mono">
                      {resource}
                    </span>
                  ))}
                  <span className="rounded-md border border-white/10 px-2 py-1">
                    {t("到期", "Expires")}: {new Date(grant.expires_at * 1000).toLocaleString()}
                  </span>
                  <span className="rounded-md border border-white/10 px-2 py-1">
                    {t("已使用", "Used")}: {grant.use_count}
                  </span>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
