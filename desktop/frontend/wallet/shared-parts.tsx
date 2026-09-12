import { displayUnits, shortPublic, movementSign } from "./amounts";
import { useDesktopAssetAccount } from "./context";
import { getLanguage } from "../i18n";
export { useDesktopAssetAccount, useStandaloneAssetView } from "./context";
type Translate = (zh: string, en: string) => string;

export function NativeTransferAuthorization({ t }: { t: Translate }) {
  const local = useDesktopAssetAccount();
  if (!local) return null;
  return (
    <fieldset>
      <legend className="text-sm font-medium text-[var(--theme-text-strong)]">
        {t("签名方式", "Signing method")}
      </legend>
      <div className="mt-2 grid gap-2 sm:grid-cols-2">
        <button
          type="button"
          aria-pressed="true"
          className="theme-accent-btn justify-start px-3 py-3"
          disabled={!local.runtime.ready}
        >
          {t("桌面密钥库签名", "Desktop vault signing")}
        </button>
      </div>
      <p className="mt-2 text-xs text-[var(--theme-text-muted)]">
        {t(
          "确认后将在本地安全窗口核对和签名。",
          "Review and sign in the local secure window after continuing.",
        )}
      </p>
      <details className="mt-3 text-xs text-[var(--theme-text-muted)]">
        <summary>{t("手续费上限", "Fee limit")}</summary>
        <label className="mt-2 grid max-w-md gap-1.5">
          {t("最多接受的手续费（%）", "Maximum accepted fee (%)")}
          <input
            className="theme-input text-sm"
            inputMode="decimal"
            value={local.transferFeeLimit ?? "0"}
            onChange={(e) => local.setTransferFeeLimit?.(e.target.value)}
          />
        </label>
      </details>
    </fieldset>
  );
}

/** Native history supplies asset movements, not the web API's full two-sided trade receipts. */
export function LocalAccountHistory({
  t,
  tradesOnly = false,
}: {
  t: Translate;
  tradesOnly?: boolean;
}) {
  const local = useDesktopAssetAccount();
  if (!local) return null;
  const { data, records, busy, error, refresh, check } = local.runtime;
  const history = (data?.records ?? []).filter(
    (r) => !tradesOnly || r.kind === "bancor_buy" || r.kind === "bancor_sell",
  );
  const label = (kind: string) =>
    ({
      bancor_buy: t("Bancor 买入", "Bancor buy"),
      bancor_sell: t("Bancor 卖出", "Bancor sell"),
      transfer_in: t("转入", "Received"),
      transfer_out: t("转出", "Sent"),
    })[kind] ?? t("资产变动", "Asset movement");
  return (
    <div
      data-desktop-account-history="true"
      className={tradesOnly ? "mt-4" : "px-5 py-4 sm:px-6"}
    >
      {records.length > 0 && (
        <details className="mb-4 text-xs text-[var(--theme-text-muted)]">
          <summary>
            {t("本机提交记录", "Local submissions")} · {records.length}
          </summary>
          {records
            .slice()
            .reverse()
            .map((record) => (
              <div
                key={record.operation_id}
                className="mt-2 flex flex-wrap items-center justify-between gap-2 rounded-lg border border-[var(--theme-border)] p-3"
              >
                <span>
                  {record.kind === "bancor_trade"
                    ? "Bancor"
                    : t("转账", "Transfer")}{" "}
                  ·{" "}
                  {record.status === "succeeded"
                    ? t("已完成", "Completed")
                    : record.status === "failed"
                      ? t("失败", "Failed")
                      : record.status === "expired"
                        ? t("已过期", "Expired")
                        : t("结果待核实", "Result unconfirmed")}
                </span>
                <code className="break-all">{record.operation_id}</code>
                {record.status === "pending" && (
                  <button
                    type="button"
                    className="theme-secondary-btn px-2 py-1"
                    disabled={busy}
                    onClick={() => void check(record.operation_id)}
                  >
                    {t("核实结果", "Check result")}
                  </button>
                )}
              </div>
            ))}
        </details>
      )}
      {error ? (
        <p
          role="alert"
          className="py-6 text-center text-sm text-[var(--theme-text-muted)]"
        >
          {error}
        </p>
      ) : !data ? (
        <p className="py-6 text-center text-sm text-[var(--theme-text-muted)]">
          {local.statusMessage}
        </p>
      ) : history.length === 0 ? (
        <p className="py-6 text-center text-sm text-[var(--theme-text-muted)]">
          {t("本页暂无账户记录。", "No account activity on this page.")}
        </p>
      ) : (
        <div className="grid gap-2">
          {history.map((record, index) => (
            <div
              key={`${record.operation_id}:${index}`}
              className="grid gap-2 rounded-xl border border-[var(--theme-border)] px-4 py-3 text-sm sm:grid-cols-[1fr_auto] sm:items-center"
            >
              <div>
                <span className="font-medium text-[var(--theme-text-strong)]">
                  {label(record.kind)}
                </span>
                <p className="mt-1 text-xs text-[var(--theme-text-muted)]">
                  {new Date(record.created_at_unix * 1000).toLocaleString(getLanguage() === "zh" ? "zh-CN" : "en-US")}
                  {record.counterparty &&
                    ` · ${shortPublic(record.counterparty)}`}
                </p>
              </div>
              <span className="font-mono text-[var(--theme-text-body)]">
                {movementSign(record.kind, record.asset)}{displayUnits(record.amount_units)} {record.asset}
              </span>
            </div>
          ))}
        </div>
      )}
      {data && data.total_pages > 1 && (
        <div className="mt-4 flex items-center justify-between gap-3">
          <button
            type="button"
            className="theme-secondary-btn px-3 py-2 text-xs"
            disabled={busy || data.page <= 1}
            onClick={() => void refresh(data.page - 1)}
          >
            {t("上一页", "Previous")}
          </button>
          <span className="text-xs text-[var(--theme-text-muted)]">
            {data.page} / {data.total_pages}
          </span>
          <button
            type="button"
            className="theme-secondary-btn px-3 py-2 text-xs"
            disabled={busy || data.page >= data.total_pages}
            onClick={() => void refresh(data.page + 1)}
          >
            {t("下一页", "Next")}
          </button>
        </div>
      )}
    </div>
  );
}
