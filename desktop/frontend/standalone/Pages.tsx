import { useEffect, useMemo } from "react";
import { useBancorRuntime } from "../../../UI/src/hooks/useBancorRuntime";
import { useAssetTransferHistoryRuntime } from "../../../UI/src/hooks/useAssetTransferHistoryRuntime";
import { AssetsPage, BancorPage } from "../wallet/pages";
import { marketFetch, type AssetConnection, type AssetNode } from "./network";
import { openWallet, useWallet } from "../wallet/store";
import { useLanguage } from "../i18n";

export function Pages({ connection, nodes, page, onPage, onNode, onAddNode, nodeBusy, nodeError }: {
  connection: AssetConnection; nodes: AssetNode[]; page: "assets" | "bancor";
  onPage: (page: "assets" | "bancor") => void;
  onNode: (origin: string) => Promise<boolean>; onAddNode: (origin: string) => Promise<boolean>;
  nodeBusy: boolean; nodeError: string;
}) {
  const { lang, t } = useLanguage();
  const wallet = useWallet();
  const apiFetch = useMemo(() => marketFetch(connection), [connection]);
  const runtime = useBancorRuntime({ apiFetch, cacheScope: `asset-owner:${connection.node.id}`, t });
  const history = useAssetTransferHistoryRuntime({ apiFetch, t });
  useEffect(() => {
    void runtime.fetchMarket();
    if (page === "bancor") {
      void runtime.fetchCandles(); void runtime.fetchMarketTrades();
    }
    const timer = setInterval(() => {
      void runtime.fetchMarket(true);
      if (page === "bancor") {
        void runtime.fetchCandles(undefined, true); void runtime.fetchMarketTrades(true);
      }
    }, 30_000);
    return () => clearInterval(timer);
  }, [connection.id, page, t]);
  if (!wallet.accounts.some(a => a.id === wallet.selectedId)) return <section className="theme-panel-soft p-6">
    <p>{t("请先在首页选择一个本地资产账户。", "Select a local asset account on the home page first.")}</p>
  </section>;
  const nodeUrls = nodes.map(n => n.origin);
  return page === "bancor" ? <BancorPage t={t} runtime={runtime}
    formatUnixDateTime={value => value ? new Date(value * 1000).toLocaleString(lang === "zh" ? "zh-CN" : "en-US") : "—"}
    assetOwnerPubkey={null} assetOwnerReady={false} signingDeviceReady={false}
    bancorServiceNodes={nodeUrls} bancorServiceNodeUrl={connection.node.origin}
    bancorServiceNodeSaving={nodeBusy} bancorServiceNodeError={nodeError || null}
    onBancorServiceNodeChange={onNode} onAddBancorServiceNode={onAddNode}
    onOpenNni={() => void openWallet()} onOpenApr={() => undefined} />
    : <AssetsPage t={t} account={null} market={runtime.market} assetOwnerPubkey={null}
      signingDeviceReady={false} accountLoading={false} marketLoading={runtime.marketLoading}
      error={runtime.error} hardwareAccountAccessUnavailable={false}
      transferLoading={false} transferError={null} transferMessage={null}
      fullHistory transferHistory={history.history} transferHistoryLoading={history.loading} transferHistoryError={history.error}
      onTransfer={async () => null} onLoadTransferHistory={history.load} onClearTransferFeedback={() => {}}
      assetServiceNodes={nodeUrls} assetServiceNodeUrl={connection.node.origin}
      assetServiceNodeSaving={nodeBusy} assetServiceNodeError={nodeError || null}
      onAssetServiceNodeChange={onNode} onAddAssetServiceNode={onAddNode}
      onRefreshMarket={() => runtime.fetchMarket()} onRefresh={() => runtime.fetchMarket()}
      onOpenBancor={() => onPage("bancor")} onOpenNni={() => void openWallet()} />;
}
