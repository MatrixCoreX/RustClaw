import {
  ArrowDownLeft,
  ArrowLeftRight,
  ArrowUpRight,
  ChevronLeft,
  ChevronRight,
  CircleDollarSign,
  Coins,
  History,
  RefreshCw,
  SendHorizontal,
  Settings2,
  WalletCards,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import type {
  NniAssetTransferHistoryRecord,
  NniAssetTransferHistoryResponse,
  NniBancorAccountResponse,
  NniBancorMarketResponse,
} from "../types/api";
import {
  buildAssetAccountOptions,
  formatAssetAccountOption,
  type AssetAccountOption,
} from "../lib/asset-account-options";
import {
  formatBancorBalanceAmount,
  formatBancorBalanceHoverAmount,
  formatBancorTradeHistoryAmount,
} from "../lib/bancor-amount-display";
import type { AssetTransferAsset } from "../lib/asset-transfer";
import {
  ASSET_HISTORY_DISPLAY_PAGE_SIZE,
  assetHistoryDisplayTotalPages,
  assetHistoryLocalTransactionOffset,
  assetHistoryRemotePage,
  type AssetHistoryDirectionFilter,
  type AssetHistoryLoadOptions,
  type AssetHistorySourceFilter,
} from "../lib/asset-transfer-history";
import { NniPublicKeyDisplay } from "./NniPublicKeyDisplay";
import { AssetTransferDialog } from "./AssetTransferDialog";
import type { AssetTransferInput } from "../hooks/useAssetTransferRuntime";
import { FinancialServiceNodeSelector } from "./FinancialServiceNodeSelector";

type Translate = (zh: string, en: string) => string;
const EMPTY_ASSET_ACCOUNTS: readonly AssetAccountOption[] = [];
export const ASSET_TRANSFER_HISTORY_DEFER_MS = 600;

interface FixedDecimal {
  units: bigint;
  scale: number;
}

export interface AssetPortfolioValues {
  aicValueUsd: string;
  totalValueUsd: string;
}

export interface AssetTransferHistoryEntry {
  id: string;
  transactionId: string;
  direction: "incoming" | "outgoing";
  counterparty: string;
  counterpartyKind: "asset_owner" | "pool" | "fee" | "system";
  transactionClass: "peer_transfer" | "market_trade" | "system_issuance" | "other";
  asset: "AIC" | "USD";
  amount: string;
  memo: string | null;
  createdAtUnix: number;
}

export function assetTransferHistoryAutoLoadDelay(
  hasSelectedAccount: boolean,
  selectedUsesLoadedAccount: boolean,
  accountLoading: boolean,
): number | null {
  if (!hasSelectedAccount) return null;
  if (selectedUsesLoadedAccount && accountLoading) return null;
  return selectedUsesLoadedAccount ? ASSET_TRANSFER_HISTORY_DEFER_MS : 0;
}

export function buildAssetTransferHistoryEntries(
  transactions: readonly NniAssetTransferHistoryRecord[],
  ownerPublicKey: string,
): AssetTransferHistoryEntry[] {
  const entries: AssetTransferHistoryEntry[] = [];
  for (const transaction of transactions) {
    for (const flow of transaction.flows) {
      const outgoing = flow.from.address === ownerPublicKey;
      const incoming = flow.to.address === ownerPublicKey;
      if (!outgoing && !incoming) continue;
      entries.push({
        id: `${transaction.transaction_id}:${flow.flow_index}`,
        transactionId: transaction.transaction_id,
        direction: outgoing ? "outgoing" : "incoming",
        counterparty: (outgoing ? flow.to.address : flow.from.address) ?? "",
        counterpartyKind: outgoing ? flow.to.account_kind : flow.from.account_kind,
        transactionClass: transaction.transaction_class,
        asset: flow.asset,
        amount: flow.amount,
        memo: transaction.memo,
        createdAtUnix: transaction.created_at_unix,
      });
    }
  }
  return entries;
}

function formatTransferHistoryTime(createdAtUnix: number): string {
  if (!Number.isSafeInteger(createdAtUnix) || createdAtUnix < 0) return "--";
  const date = new Date(createdAtUnix * 1000);
  if (Number.isNaN(date.getTime())) return "--";
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

function parseUnsignedDecimal(value: string): FixedDecimal | null {
  const match = /^(\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!match) return null;
  const fraction = match[2] ?? "";
  return {
    units: BigInt(`${match[1]}${fraction}`),
    scale: fraction.length,
  };
}

function rescaleDecimal(value: FixedDecimal, targetScale: number): bigint {
  if (value.scale === targetScale) return value.units;
  if (value.scale < targetScale) {
    return value.units * (10n ** BigInt(targetScale - value.scale));
  }
  return value.units / (10n ** BigInt(value.scale - targetScale));
}

function formatDecimal(units: bigint, scale: number): string {
  if (scale === 0) return String(units);
  const divisor = 10n ** BigInt(scale);
  return `${units / divisor}.${String(units % divisor).padStart(scale, "0")}`;
}

export function calculateAssetPortfolioValues(
  account: NniBancorAccountResponse,
  market: NniBancorMarketResponse,
): AssetPortfolioValues | null {
  const aicBalance = parseUnsignedDecimal(account.aic_balance);
  const usdBalance = parseUnsignedDecimal(account.usd_balance);
  const aicPrice = parseUnsignedDecimal(market.marginal_price_usd_per_aic);
  if (!aicBalance || !usdBalance || !aicPrice) return null;

  const displayScale = 8;
  const aicValueUnits = rescaleDecimal({
    units: aicBalance.units * aicPrice.units,
    scale: aicBalance.scale + aicPrice.scale,
  }, displayScale);
  const totalValueUnits = aicValueUnits + rescaleDecimal(usdBalance, displayScale);
  return {
    aicValueUsd: formatDecimal(aicValueUnits, displayScale),
    totalValueUsd: formatDecimal(totalValueUnits, displayScale),
  };
}

function WalletAmount({ value, suffix }: { value: string; suffix: string }) {
  const visible = formatBancorBalanceAmount(value);
  const full = formatBancorBalanceHoverAmount(value);
  return (
    <span className="min-w-0 text-right" title={`${full} ${suffix}`} data-assets-full-value={full}>
      <span className="block truncate text-base font-semibold text-[var(--theme-text-strong)] sm:text-lg">
        {visible}
      </span>
      <span className="block text-[11px] font-medium text-[var(--theme-text-muted)]">{suffix}</span>
    </span>
  );
}

export function AssetsPage({
  t,
  account,
  market,
  assetOwnerPubkey,
  additionalAssetAccounts = EMPTY_ASSET_ACCOUNTS,
  signingDeviceReady,
  accountLoading,
  marketLoading,
  error,
  hardwareAccountAccessUnavailable,
  transferLoading,
  transferError,
  transferMessage,
  transferHistory,
  transferHistoryLoading,
  transferHistoryError,
  assetServiceNodes = [],
  assetServiceNodeUrl = "",
  assetServiceNodeSaving = false,
  assetServiceNodeError = null,
  onTransfer,
  onLoadTransferHistory,
  onClearTransferFeedback,
  onAssetServiceNodeChange = async () => false,
  onAddAssetServiceNode = async () => false,
  onRefresh,
  onOpenBancor,
  onOpenNni,
}: {
  t: Translate;
  account: NniBancorAccountResponse | null;
  market: NniBancorMarketResponse | null;
  assetOwnerPubkey?: string | null;
  additionalAssetAccounts?: readonly AssetAccountOption[];
  signingDeviceReady: boolean;
  accountLoading: boolean;
  marketLoading: boolean;
  error: string | null;
  hardwareAccountAccessUnavailable: boolean;
  transferLoading: boolean;
  transferError: string | null;
  transferMessage: string | null;
  transferHistory: NniAssetTransferHistoryResponse | null;
  transferHistoryLoading: boolean;
  transferHistoryError: string | null;
  assetServiceNodes?: readonly string[];
  assetServiceNodeUrl?: string;
  assetServiceNodeSaving?: boolean;
  assetServiceNodeError?: string | null;
  onTransfer: (input: AssetTransferInput) => Promise<unknown>;
  onLoadTransferHistory: (
    ownerPublicKey: string,
    options?: AssetHistoryLoadOptions,
  ) => Promise<unknown>;
  onClearTransferFeedback: () => void;
  onAssetServiceNodeChange?: (nodeUrl: string) => Promise<boolean>;
  onAddAssetServiceNode?: (nodeUrl: string) => Promise<boolean>;
  onRefresh: () => void | Promise<unknown>;
  onOpenBancor: () => void;
  onOpenNni: () => void;
}) {
  const assetAccountOptions = useMemo(
    () => buildAssetAccountOptions(assetOwnerPubkey, additionalAssetAccounts),
    [additionalAssetAccounts, assetOwnerPubkey],
  );
  const [preferredAssetAccountId, setPreferredAssetAccountId] = useState(
    assetAccountOptions[0]?.id ?? "",
  );
  const [transferDialogOpen, setTransferDialogOpen] = useState(false);
  const [transferAsset, setTransferAsset] = useState<AssetTransferAsset>("AIC");
  const [historySource, setHistorySource] = useState<AssetHistorySourceFilter>("all");
  const [historyDirection, setHistoryDirection] = useState<AssetHistoryDirectionFilter>("all");
  const [historyDisplayPage, setHistoryDisplayPage] = useState(1);
  const selectedAssetAccount = assetAccountOptions.find(
    (accountOption) => accountOption.id === preferredAssetAccountId,
  ) ?? assetAccountOptions[0] ?? null;
  const selectedUsesLoadedAccount = selectedAssetAccount?.source === "local_binding"
    && selectedAssetAccount.publicKey === assetOwnerPubkey?.trim();
  const selectedAccount = selectedUsesLoadedAccount ? account : null;
  const portfolio = selectedAccount && market
    ? calculateAssetPortfolioValues(selectedAccount, market)
    : null;
  const loading = accountLoading || marketLoading;
  const accountAvailable = Boolean(selectedAssetAccount && selectedAccount);
  const expectedHistoryRemotePage = assetHistoryRemotePage(historyDisplayPage);
  const matchingHistoryScope = transferHistory !== null
    && selectedAssetAccount !== null
    && transferHistory.owner_pubkey === selectedAssetAccount.publicKey
    && transferHistory.source_filter === historySource
    && transferHistory.direction_filter === historyDirection
    ? transferHistory
    : null;
  const visibleTransferHistory = matchingHistoryScope?.page === expectedHistoryRemotePage
    ? matchingHistoryScope
    : null;
  const historyBatchOffset = assetHistoryLocalTransactionOffset(historyDisplayPage);
  const visibleHistoryTransactions = visibleTransferHistory?.transactions.slice(
    historyBatchOffset,
    historyBatchOffset + ASSET_HISTORY_DISPLAY_PAGE_SIZE,
  ) ?? [];
  const transferHistoryEntries = buildAssetTransferHistoryEntries(
    visibleHistoryTransactions,
    selectedAssetAccount?.publicKey ?? "",
  );
  const historyTotalDisplayPages = assetHistoryDisplayTotalPages(
    matchingHistoryScope?.total_transactions ?? 0,
  );

  const openTransferDialog = (asset: AssetTransferAsset) => {
    onClearTransferFeedback();
    setTransferAsset(asset);
    setTransferDialogOpen(true);
  };

  useEffect(() => {
    if (assetAccountOptions.some((option) => option.id === preferredAssetAccountId)) return;
    setPreferredAssetAccountId(assetAccountOptions[0]?.id ?? "");
    setHistoryDisplayPage(1);
  }, [assetAccountOptions, preferredAssetAccountId]);

  useEffect(() => {
    const publicKey = selectedAssetAccount?.publicKey ?? "";
    if (!publicKey) {
      void onLoadTransferHistory("");
      return;
    }
    const delay = assetTransferHistoryAutoLoadDelay(
      true,
      selectedUsesLoadedAccount,
      accountLoading,
    );
    if (delay === null) return;
    const timer = window.setTimeout(() => {
      void onLoadTransferHistory(publicKey, {
        source: historySource,
        direction: historyDirection,
        displayPage: historyDisplayPage,
      });
    }, delay);
    return () => window.clearTimeout(timer);
  }, [
    accountLoading,
    historyDirection,
    historyDisplayPage,
    historySource,
    onLoadTransferHistory,
    selectedAssetAccount?.publicKey,
    selectedUsesLoadedAccount,
  ]);

  const submitTransfer = async (input: AssetTransferInput) => {
    const result = await onTransfer(input);
    if (result && selectedAssetAccount) {
      void onLoadTransferHistory(selectedAssetAccount.publicKey, {
        source: historySource,
        direction: historyDirection,
        displayPage: historyDisplayPage,
        force: true,
      });
    }
    return result;
  };

  const activateAssetServiceNode = async (
    nodeUrl: string,
    persist: (value: string) => Promise<boolean>,
  ) => {
    if (!(await persist(nodeUrl))) return false;
    await onRefresh();
    if (selectedAssetAccount) {
      await onLoadTransferHistory(selectedAssetAccount.publicKey, {
        source: historySource,
        direction: historyDirection,
        displayPage: historyDisplayPage,
        force: true,
      });
    }
    return true;
  };
  const changeAssetServiceNode = (nodeUrl: string) =>
    activateAssetServiceNode(nodeUrl, onAssetServiceNodeChange);
  const addAssetServiceNode = (nodeUrl: string) =>
    activateAssetServiceNode(nodeUrl, onAddAssetServiceNode);

  const statusMessage = selectedAssetAccount?.source === "external"
    ? t("这个外部账户的余额尚未同步。", "Balances for this external account have not been synchronized yet.")
    : !assetOwnerPubkey
    ? t(
      "尚未绑定资产账户。请前往 NNI 页面创建或绑定资产账户。",
      "No asset account is bound yet. Go to the NNI page to create or bind one.",
    )
    : !signingDeviceReady
      ? t("当前设备无法签名读取私有余额。", "This device cannot sign a private balance request.")
      : hardwareAccountAccessUnavailable
        ? t("当前硬件授权暂时无法读取这个资产账户。", "The current hardware authorization cannot read this asset account right now.")
        : error && !account
          ? error
          : t("资产余额暂不可用，请刷新后重试。", "Asset balances are unavailable. Refresh and try again.");

  return (
    <div className="space-y-5" data-assets-page="true">
      <header className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <div className="flex items-center gap-2 text-[var(--theme-icon-accent-color)]">
            <WalletCards className="h-5 w-5" />
            <span className="text-xs font-semibold uppercase tracking-wide">{t("资产总览", "Asset overview")}</span>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" onClick={onOpenNni}>
            <Settings2 className="h-4 w-4" />
            {t("管理账户", "Manage account")}
          </button>
          <button type="button" className="theme-accent-btn px-3 py-2 text-sm" onClick={onOpenBancor}>
            <ArrowLeftRight className="h-4 w-4" />
            {t("交易", "Trade")}
          </button>
        </div>
      </header>

      <section className="theme-panel overflow-hidden p-5 sm:p-6" aria-labelledby="asset-overview-title">
        <div className="flex flex-col gap-5 lg:flex-row lg:items-end lg:justify-between">
          <div className="min-w-0">
            <p id="asset-overview-title" className="text-sm font-medium text-[var(--theme-text-muted)]">
              {t("总资产估值", "Estimated portfolio value")}
            </p>
            <div className="mt-2 min-h-12">
              {portfolio ? (
                <p
                  className="break-all text-3xl font-semibold text-[var(--theme-text-strong)] sm:text-4xl"
                  title={`${formatBancorBalanceHoverAmount(portfolio.totalValueUsd)} USD`}
                  data-assets-total-value={portfolio.totalValueUsd}
                >
                  {formatBancorBalanceAmount(portfolio.totalValueUsd)}
                  <span className="ml-2 text-base font-medium text-[var(--theme-text-muted)]">USD</span>
                </p>
              ) : (
                <p className="text-3xl font-semibold text-[var(--theme-text-faint)]">--</p>
              )}
            </div>
            <p className="mt-2 text-xs leading-5 text-[var(--theme-text-faint)]">
              {t("按当前 BANCOR 边际价格估算，不代表实际成交金额。", "Estimated using the current BANCOR marginal price; this is not an executable quote.")}
            </p>
          </div>
          <div className="flex flex-wrap gap-2 lg:justify-end" data-assets-overview-actions="true">
            <button
              type="button"
              className="theme-secondary-btn shrink-0 px-3 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-50"
              disabled={loading || !selectedUsesLoadedAccount || !signingDeviceReady}
              onClick={() => void onRefresh()}
            >
              <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
              {loading ? t("读取中", "Loading") : t("刷新资产", "Refresh assets")}
            </button>
          </div>
        </div>

        <div className="mt-5 border-t border-[var(--theme-border)] pt-4">
          {selectedAssetAccount ? (
            <label className="grid gap-1.5" data-assets-account-selector="true">
              <span className="text-xs font-medium text-[var(--theme-text-muted)]">{t("资产账户", "Asset account")}</span>
              <select
                className="theme-input w-full font-mono text-xs"
                value={selectedAssetAccount.id}
                aria-label={t("选择资产账户", "Select asset account")}
                onChange={(event) => {
                  setPreferredAssetAccountId(event.target.value);
                  setHistoryDisplayPage(1);
                }}
              >
                {assetAccountOptions.map((accountOption) => (
                  <option key={accountOption.id} value={accountOption.id}>
                    {formatAssetAccountOption(accountOption, t, { fullPublicKey: true })}
                  </option>
                ))}
              </select>
              <NniPublicKeyDisplay
                value={selectedAssetAccount.publicKey}
                t={t}
                className="mt-0.5"
                valueClassName="text-[10px] leading-4 text-[var(--theme-text-muted)]"
                allowFormatSwitch={false}
              />
            </label>
          ) : (
            <div>
              <p className="text-xs font-medium text-[var(--theme-text-muted)]">{t("资产账户", "Asset account")}</p>
              <p className="mt-2 text-sm text-[var(--theme-text-faint)]">{t("未绑定", "Not bound")}</p>
            </div>
          )}
        </div>
      </section>

      <section className="theme-panel overflow-hidden" aria-labelledby="asset-list-title">
        <div className="flex items-center justify-between border-b border-[var(--theme-border)] px-5 py-4 sm:px-6">
          <div>
            <h3 id="asset-list-title" className="text-base font-semibold text-[var(--theme-text-strong)]">
              {t("资产列表", "Asset list")}
            </h3>
            <p className="mt-1 text-xs text-[var(--theme-text-muted)]">{t("当前账户支持的资产", "Assets supported by this account")}</p>
          </div>
          <span className="rounded-full border border-[var(--theme-border)] bg-[var(--theme-card-strong)] px-2.5 py-1 text-xs text-[var(--theme-text-muted)]">
            {accountAvailable ? t("已同步", "Synced") : t("待同步", "Not synced")}
          </span>
        </div>

        {accountAvailable && selectedAccount ? (
          <div className="divide-y divide-[var(--theme-border)]" data-assets-list-ready="true">
            <div className="grid min-h-24 grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-5 py-4 sm:grid-cols-[minmax(180px,1fr)_minmax(150px,0.7fr)_auto] sm:px-6">
              <div className="flex min-w-0 items-center gap-3">
                <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border border-emerald-400/25 bg-emerald-500/10 text-emerald-300">
                  <Coins className="h-5 w-5" />
                </span>
                <div className="min-w-0">
                  <p className="font-semibold text-[var(--theme-text-strong)]">AIC</p>
                  <p className="truncate text-xs text-[var(--theme-text-muted)]">{t("网络原生资产", "Network asset")}</p>
                </div>
              </div>
              <div className="hidden min-w-0 text-right sm:block">
                <p className="text-xs text-[var(--theme-text-muted)]">{t("参考价格", "Reference price")}</p>
                <p className="mt-1 truncate text-sm text-[var(--theme-text-body)]">
                  {market ? `${formatBancorTradeHistoryAmount(market.marginal_price_usd_per_aic)} USD` : "--"}
                </p>
                <p className="mt-0.5 text-[11px] text-[var(--theme-text-faint)]">
                  {portfolio ? `≈ ${formatBancorBalanceAmount(portfolio.aicValueUsd)} USD` : ""}
                </p>
              </div>
              <div className="flex items-center justify-end gap-2">
                <WalletAmount value={selectedAccount.aic_balance} suffix="AIC" />
                <button
                  type="button"
                  className="theme-secondary-btn h-9 shrink-0 px-2.5 text-xs"
                  data-asset-transfer="AIC"
                  onClick={() => openTransferDialog("AIC")}
                >
                  <SendHorizontal className="h-4 w-4" />
                  {t("转账", "Transfer")}
                </button>
              </div>
            </div>

            <div className="grid min-h-24 grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-5 py-4 sm:grid-cols-[minmax(180px,1fr)_minmax(150px,0.7fr)_auto] sm:px-6">
              <div className="flex min-w-0 items-center gap-3">
                <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border border-amber-400/25 bg-amber-500/10 text-amber-300">
                  <CircleDollarSign className="h-5 w-5" />
                </span>
                <div className="min-w-0">
                  <p className="font-semibold text-[var(--theme-text-strong)]">USD</p>
                  <p className="truncate text-xs text-[var(--theme-text-muted)]">{t("美元记账资产", "USD ledger asset")}</p>
                </div>
              </div>
              <div className="hidden min-w-0 text-right sm:block">
                <p className="text-xs text-[var(--theme-text-muted)]">{t("资产估值", "Asset value")}</p>
                <p className="mt-1 text-sm text-[var(--theme-text-body)]">
                  ≈ {formatBancorBalanceAmount(selectedAccount.usd_balance)} USD
                </p>
              </div>
              <div className="flex items-center justify-end gap-2">
                <WalletAmount value={selectedAccount.usd_balance} suffix="USD" />
                <button
                  type="button"
                  className="theme-secondary-btn h-9 shrink-0 px-2.5 text-xs"
                  data-asset-transfer="USD"
                  onClick={() => openTransferDialog("USD")}
                >
                  <SendHorizontal className="h-4 w-4" />
                  {t("转账", "Transfer")}
                </button>
              </div>
            </div>
          </div>
        ) : (
          <div className="px-5 py-10 text-center sm:px-6" data-assets-empty-state="true">
            <WalletCards className="mx-auto h-8 w-8 text-[var(--theme-text-faint)]" />
            <p className="mx-auto mt-3 max-w-lg text-sm leading-6 text-[var(--theme-text-muted)]">{statusMessage}</p>
            <div className="mt-4 flex flex-wrap justify-center gap-2">
              <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" onClick={onOpenNni}>
                <Settings2 className="h-4 w-4" />
                {assetOwnerPubkey
                  ? t("管理资产账户", "Manage asset account")
                  : t("前往 NNI 绑定", "Bind in NNI")}
              </button>
              {selectedUsesLoadedAccount && signingDeviceReady ? (
                <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={loading} onClick={() => void onRefresh()}>
                  <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
                  {t("重试", "Retry")}
                </button>
              ) : null}
            </div>
          </div>
        )}
      </section>

      <section className="theme-panel overflow-hidden" aria-labelledby="asset-transfer-history-title">
        <div className="flex items-center justify-between gap-3 border-b border-[var(--theme-border)] px-5 py-4 sm:px-6">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <History className="h-4 w-4 text-[var(--theme-icon-accent-color)]" />
              <h3 id="asset-transfer-history-title" className="text-base font-semibold text-[var(--theme-text-strong)]">
                {t("资产流水", "Asset activity")}
              </h3>
            </div>
            <p className="mt-1 text-xs text-[var(--theme-text-muted)]">
              {t("当前选定账户的转账、交易和系统发放记录", "Transfers, trades, and system issuance for the selected account")}
            </p>
          </div>
          <button
            type="button"
            className="theme-secondary-btn h-9 shrink-0 px-2.5 text-xs disabled:cursor-not-allowed disabled:opacity-50"
            disabled={!selectedAssetAccount || transferHistoryLoading}
            onClick={() => void onLoadTransferHistory(selectedAssetAccount?.publicKey ?? "", {
              source: historySource,
              direction: historyDirection,
              displayPage: historyDisplayPage,
              force: true,
            })}
          >
            <RefreshCw className={`h-4 w-4 ${transferHistoryLoading ? "animate-spin" : ""}`} />
            {t("刷新", "Refresh")}
          </button>
        </div>

        <div className="grid gap-3 border-b border-[var(--theme-border)] bg-[var(--theme-card-strong)]/40 px-5 py-3 sm:grid-cols-2 sm:px-6">
          <label className="grid gap-1">
            <span className="text-[11px] font-medium text-[var(--theme-text-muted)]">
              {t("记录类型", "Activity type")}
            </span>
            <select
              className="theme-input h-9 text-xs"
              value={historySource}
              data-asset-history-source-filter="true"
              onChange={(event) => {
                setHistorySource(event.target.value as AssetHistorySourceFilter);
                setHistoryDisplayPage(1);
              }}
            >
              <option value="all">{t("全部", "All")}</option>
              <option value="transfer">{t("转账", "Transfers")}</option>
              <option value="trade">{t("交易", "Trades")}</option>
              <option value="issuance">{t("系统发放", "System issuance")}</option>
            </select>
          </label>
          <label className="grid gap-1">
            <span className="text-[11px] font-medium text-[var(--theme-text-muted)]">
              {t("资金方向", "Direction")}
            </span>
            <select
              className="theme-input h-9 text-xs"
              value={historyDirection}
              data-asset-history-direction-filter="true"
              onChange={(event) => {
                setHistoryDirection(event.target.value as AssetHistoryDirectionFilter);
                setHistoryDisplayPage(1);
              }}
            >
              <option value="all">{t("全部", "All")}</option>
              <option value="incoming">{t("转入", "Incoming")}</option>
              <option value="outgoing">{t("转出", "Outgoing")}</option>
            </select>
          </label>
        </div>

        {!selectedAssetAccount ? (
          <p className="px-5 py-8 text-center text-sm text-[var(--theme-text-muted)] sm:px-6">
            {t("绑定资产账户后可查看资产流水。", "Bind an asset account to view asset activity.")}
          </p>
        ) : transferHistoryLoading && !visibleTransferHistory ? (
          <p className="px-5 py-8 text-center text-sm text-[var(--theme-text-muted)] sm:px-6" role="status">
            {t("正在读取资产流水…", "Loading asset activity…")}
          </p>
        ) : transferHistoryError ? (
          <div className="px-5 py-7 text-center sm:px-6">
            <p className="text-sm text-[var(--theme-text-muted)]" role="alert">{transferHistoryError}</p>
            <button
              type="button"
              className="theme-secondary-btn mt-3 px-3 py-2 text-sm"
              onClick={() => void onLoadTransferHistory(selectedAssetAccount.publicKey, {
                source: historySource,
                direction: historyDirection,
                displayPage: historyDisplayPage,
                force: true,
              })}
            >
              <RefreshCw className="h-4 w-4" />
              {t("重试", "Retry")}
            </button>
          </div>
        ) : transferHistoryEntries.length === 0 ? (
          <p className="px-5 py-8 text-center text-sm text-[var(--theme-text-muted)] sm:px-6">
            {t("当前筛选条件下暂无资产流水。", "There is no asset activity for these filters.")}
          </p>
        ) : (
          <div className="divide-y divide-[var(--theme-border)]" data-asset-transfer-history="true">
            {transferHistoryEntries.map((entry) => {
              const incoming = entry.direction === "incoming";
              const DirectionIcon = incoming ? ArrowDownLeft : ArrowUpRight;
              const counterparty = entry.counterparty || (
                entry.counterpartyKind === "system"
                  ? t("系统账户", "System account")
                  : entry.counterpartyKind === "pool"
                    ? t("市场资金池", "Market pool")
                    : entry.counterpartyKind === "fee"
                      ? t("手续费账户", "Fee account")
                      : t("资产账户", "Asset account")
              );
              const activityLabel = entry.transactionClass === "peer_transfer"
                ? t("转账", "Transfer")
                : entry.transactionClass === "market_trade"
                  ? t("交易", "Trade")
                  : entry.transactionClass === "system_issuance"
                    ? t("系统发放", "System issuance")
                    : t("其他", "Other");
              return (
                <div
                  key={entry.id}
                  className="grid gap-3 px-5 py-4 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center sm:px-6"
                  data-transfer-direction={entry.direction}
                >
                  <div className="flex min-w-0 items-start gap-3">
                    <span className={`mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full border ${incoming
                      ? "border-emerald-400/25 bg-emerald-500/10 text-emerald-300"
                      : "border-amber-400/25 bg-amber-500/10 text-amber-300"}`}
                    >
                      <DirectionIcon className="h-4 w-4" />
                    </span>
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
                        <span className="text-sm font-semibold text-[var(--theme-text-strong)]">
                          {incoming ? t("转入", "Received") : t("转出", "Sent")}
                        </span>
                        <span className="text-xs text-[var(--theme-text-muted)]">
                          {incoming ? t("来自", "From") : t("发往", "To")}
                        </span>
                        <span className="rounded border border-[var(--theme-border)] px-1.5 py-0.5 text-[10px] text-[var(--theme-text-muted)]">
                          {activityLabel}
                        </span>
                        <span className="max-w-full truncate font-mono text-xs text-[var(--theme-text-body)]" title={counterparty}>
                          {counterparty}
                        </span>
                      </div>
                      <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-[var(--theme-text-faint)]">
                        <span>{formatTransferHistoryTime(entry.createdAtUnix)}</span>
                        <span className="font-mono" title={entry.transactionId}>
                          {t("交易", "Transaction")} {entry.transactionId.slice(0, 12)}…
                        </span>
                      </div>
                      {entry.memo ? (
                        <p className="mt-1.5 break-words text-xs text-[var(--theme-text-muted)]">
                          Memo: {entry.memo}
                        </p>
                      ) : null}
                    </div>
                  </div>
                  <p className={`text-right text-base font-semibold ${incoming ? "text-emerald-300" : "text-amber-300"}`}>
                    {incoming ? "+" : "-"}{formatBancorTradeHistoryAmount(entry.amount)} {entry.asset}
                  </p>
                </div>
              );
            })}
          </div>
        )}
        {historyTotalDisplayPages > 1 ? (
          <div className="flex items-center justify-between border-t border-[var(--theme-border)] px-5 py-3 sm:px-6" data-asset-history-pagination="true">
            <button
              type="button"
              className="theme-secondary-btn h-8 px-2 text-xs disabled:cursor-not-allowed disabled:opacity-40"
              disabled={historyDisplayPage <= 1 || transferHistoryLoading}
              aria-label={t("上一页", "Previous page")}
              onClick={() => setHistoryDisplayPage((page) => Math.max(1, page - 1))}
            >
              <ChevronLeft className="h-4 w-4" />
            </button>
            <span className="text-xs text-[var(--theme-text-muted)]">
              {historyDisplayPage} / {historyTotalDisplayPages}
            </span>
            <button
              type="button"
              className="theme-secondary-btn h-8 px-2 text-xs disabled:cursor-not-allowed disabled:opacity-40"
              disabled={historyDisplayPage >= historyTotalDisplayPages || transferHistoryLoading}
              aria-label={t("下一页", "Next page")}
              onClick={() => setHistoryDisplayPage((page) => Math.min(historyTotalDisplayPages, page + 1))}
            >
              <ChevronRight className="h-4 w-4" />
            </button>
          </div>
        ) : null}
      </section>

      {transferMessage && !transferDialogOpen ? (
        <p className="rounded-md border border-emerald-400/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-100" role="status">
          {transferMessage}
        </p>
      ) : null}

      <FinancialServiceNodeSelector
        t={t}
        service="assets"
        nodes={assetServiceNodes}
        selectedNodeUrl={assetServiceNodeUrl}
        saving={assetServiceNodeSaving}
        error={assetServiceNodeError}
        disabled={loading || transferLoading || transferHistoryLoading}
        onChange={changeAssetServiceNode}
        onAddNode={addAssetServiceNode}
      />

      <AssetTransferDialog
        open={transferDialogOpen}
        asset={transferAsset}
        t={t}
        sourcePublicKey={selectedUsesLoadedAccount ? selectedAssetAccount?.publicKey ?? "" : ""}
        aicBalance={selectedAccount?.aic_balance ?? "0.00000000"}
        usdBalance={selectedAccount?.usd_balance ?? "0.00000000"}
        signingDeviceReady={signingDeviceReady}
        loading={transferLoading}
        remoteError={transferError}
        onClose={() => {
          if (!transferLoading) setTransferDialogOpen(false);
        }}
        onSubmit={submitTransfer}
      />
    </div>
  );
}
