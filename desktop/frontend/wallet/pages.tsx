import { useCallback, useState, type ComponentProps } from "react";
import { AssetsPage as SharedAssetsPage } from "../../../UI/src/components/AssetsPage";
import {
  BancorPage as SharedBancorPage,
  BANCOR_CANDLE_AUTO_REFRESH_SECONDS,
} from "../../../UI/src/components/BancorPage";
import { parseBancorSlippagePercent } from "../../../UI/src/hooks/useBancorRuntime";
import { validateNniOwnerPublicKey } from "../../../UI/src/lib/nni-owner-public-key";
import type { NniBancorAccountResponse } from "../../../UI/src/types/api";
import { DesktopAssetAccountContext } from "./context";
import { amountUnits, displayUnits } from "./amounts";
import { openWallet, useAccountContext } from "./store";
import { useAssetAccount } from "./useAssetAccount";
import type { ReadResult, WalletAccount } from "./types";
import { PendingOperations } from "./PendingOperations";
import "./wallet.css";
export { BANCOR_CANDLE_AUTO_REFRESH_SECONDS };
type AssetProps = ComponentProps<typeof SharedAssetsPage> & {
  onRefreshMarket?: () => Promise<unknown>;
  fullHistory?: boolean;
};
type BancorProps = ComponentProps<typeof SharedBancorPage>;

// The shared components consume the balance fields. Local history uses its native
// record contract through a dedicated slot, never fabricated web trade receipts.
function balanceView(data: ReadResult | null): NniBancorAccountResponse | null {
  if (!data) return null;
  return {
    schema_version: 1,
    status: "desktop_balance_view",
    device_pubkey: "",
    aic_balance_units: data.aic_balance_units,
    aic_balance: displayUnits(data.aic_balance_units),
    usd_balance_units: data.usd_balance_units,
    usd_balance: displayUnits(data.usd_balance_units),
    account_version: 0,
    page: data.page,
    per_page: 0,
    total: 0,
    total_pages: data.total_pages,
    trades: [],
    node_url: data.node_url,
  };
}
function useLocalStatus(
  runtime: ReturnType<typeof useAssetAccount>,
  t: AssetProps["t"],
) {
  return runtime.error ||
        t(
          "资产尚未同步，请刷新后重试。",
          "Assets have not been synchronized. Refresh to try again.",
        );
}
export function AssetsPage(props: AssetProps) {
  const wallet = useAccountContext(`assets:${props.assetServiceNodeUrl ?? ""}`);
  const account = wallet.accounts.find((a) => a.id === wallet.selectedId);
  return (
    <div className="desktop-wallet-page">
      {account ? (
        <LocalAssets
          key={account.id}
          {...props}
          localAccount={account}
          contextReady={wallet.contextReady}
        />
      ) : (
        <SharedAssetsPage {...props} />
      )}
    </div>
  );
}
export function BancorPage(props: BancorProps) {
  const wallet = useAccountContext(
    `bancor:${props.bancorServiceNodeUrl ?? ""}`,
  );
  const account = wallet.accounts.find((a) => a.id === wallet.selectedId);
  return (
    <div className="desktop-wallet-page">
      {account ? (
        <LocalBancor
          key={account.id}
          {...props}
          localAccount={account}
          contextReady={wallet.contextReady}
        />
      ) : (
        <SharedBancorPage {...props} />
      )}
    </div>
  );
}
function LocalAssets(
  props: AssetProps & { localAccount: WalletAccount; contextReady: boolean },
) {
  const account = props.localAccount;
  const runtime = useAssetAccount(
    account,
    "assets",
    props.assetServiceNodeUrl ?? "",
    props.contextReady,
  );
  const statusMessage = useLocalStatus(runtime, props.t);
  const [feeLimit, setFeeLimit] = useState("0");
  const [draftError, setDraftError] = useState<string | null>(null);
  const refreshHistory = useCallback(async () => {
    await runtime.refresh(1);
  }, [runtime.refresh]);
  const transfer: AssetProps["onTransfer"] = async (input) => {
    setDraftError(null);
    const units = amountUnits(input.amount);
    const key = validateNniOwnerPublicKey(input.recipientPublicKey);
    const fee = parseBancorSlippagePercent(feeLimit);
    if (!runtime.ready || !runtime.cap?.actions.includes("transfer"))
      return null;
    if (
      !units ||
      !key.ok ||
      key.normalized === account.public_key ||
      fee === null ||
      new TextEncoder().encode(input.memo).length > 256
    ) {
      setDraftError(
        props.t(
          "请检查金额、收款公钥和手续费上限。",
          "Check the amount, recipient key and fee limit.",
        ),
      );
      return null;
    }
    const balance =
      input.asset === "AIC"
        ? runtime.data?.aic_balance_units
        : runtime.data?.usd_balance_units;
    if (balance === undefined || BigInt(units) > BigInt(balance)) {
      setDraftError(
        props.t("当前账户余额不足。", "Insufficient account balance."),
      );
      return null;
    }
    return runtime.prepare({
      kind: "transfer",
      asset: input.asset,
      amount_units: units,
      recipient: key.normalized,
      memo: input.memo,
      max_fee_bps: fee,
    });
  };
  return (
    <DesktopAssetAccountContext.Provider
      value={{
        account,
        hardwarePublicKey: props.assetOwnerPubkey,
        runtime,
        statusMessage,
        transferFeeLimit: feeLimit,
        setTransferFeeLimit: setFeeLimit,
        fullHistory: props.fullHistory,
      }}
    >
      {props.fullHistory && <PendingOperations />}
      <SharedAssetsPage
        {...props}
        account={balanceView(runtime.data)}
        market={
          props.market?.node_url === runtime.cap?.node_url ? props.market : null
        }
        assetOwnerPubkey={account.public_key}
        additionalAssetAccounts={[]}
        signingDeviceReady={
          runtime.ready && Boolean(runtime.cap?.actions.includes("transfer"))
        }
        accountLoading={runtime.busy}
        error={runtime.error}
        hardwareAccountAccessUnavailable={false}
        transferLoading={runtime.busy}
        transferError={draftError || runtime.error}
        transferMessage={runtime.message}
        transferHistory={props.fullHistory ? props.transferHistory : null}
        transferHistoryLoading={props.fullHistory ? props.transferHistoryLoading : runtime.busy}
        transferHistoryError={props.fullHistory ? props.transferHistoryError : runtime.error}
        onTransfer={transfer}
        onLoadTransferHistory={props.fullHistory ? props.onLoadTransferHistory : refreshHistory}
        onClearTransferFeedback={() => {
          setDraftError(null);
          runtime.clearFeedback();
        }}
        onRefresh={() =>
          Promise.allSettled([runtime.refresh(), props.onRefreshMarket?.()])
        }
        onOpenNni={() => void openWallet()}
      />
    </DesktopAssetAccountContext.Provider>
  );
}
function LocalBancor(
  props: BancorProps & { localAccount: WalletAccount; contextReady: boolean },
) {
  const account = props.localAccount;
  const runtime = useAssetAccount(
    account,
    "bancor",
    props.bancorServiceNodeUrl ?? "",
    props.contextReady,
  );
  const statusMessage = useLocalStatus(runtime, props.t);
  const [draftError, setDraftError] = useState<string | null>(null);
  const market = props.runtime.market;
  const marketMatches = Boolean(
    market && runtime.cap && market.node_url === runtime.cap.node_url,
  );
  const ready =
    runtime.ready &&
    marketMatches &&
    Boolean(runtime.cap?.actions.includes("bancor_trade"));
  const preview: BancorProps["runtime"]["preview"] = async (
    side,
    amount,
    slippage = 300,
  ) => {
    setDraftError(null);
    const units = amountUnits(amount);
    const balance =
      side === "buy"
        ? runtime.data?.usd_balance_units
        : runtime.data?.aic_balance_units;
    if (!ready || !market || market.status !== "open") return null;
    if (
      !units ||
      !Number.isSafeInteger(slippage) ||
      slippage < 0 ||
      slippage > 5000 ||
      balance === undefined ||
      BigInt(units) > BigInt(balance)
    ) {
      setDraftError(
        props.t(
          "请检查金额、余额和滑点上限。",
          "Check the amount, balance and slippage limit.",
        ),
      );
      return null;
    }
    await runtime.prepare({
      kind: "bancor_trade",
      side,
      input_units: units,
      slippage_bps: slippage,
      max_fee_bps: market.fee_bps,
    });
    return null; // The native window owns quote review and confirmation.
  };
  return (
    <DesktopAssetAccountContext.Provider
      value={{
        account,
        hardwarePublicKey: props.assetOwnerPubkey,
        runtime,
        statusMessage,
      }}
    >
      <SharedBancorPage
        {...props}
        assetOwnerPubkey={account.public_key}
        signingDeviceReady={false}
        assetOwnerReady={ready}
        runtime={{
          ...props.runtime,
          account: balanceView(runtime.data),
          accountLoading: runtime.busy,
          quote: null,
          lastTrade: null,
          quoteLoading: runtime.busy,
          tradeLoading: false,
          error: draftError || runtime.error,
          message: runtime.message,
          assetOwnerRequired: false,
          assetOwnerAccessErrorCode: null,
          hardwareAccountAccessUnavailable: false,
          fetchAccount: async (page) => {
            await runtime.refresh(page);
            return null;
          },
          preview,
          trade: async () => null,
          clearQuote: () => {
            setDraftError(null);
            runtime.clearFeedback();
          },
        }}
      />
    </DesktopAssetAccountContext.Provider>
  );
}
