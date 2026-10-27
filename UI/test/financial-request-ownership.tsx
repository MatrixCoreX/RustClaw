import { useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { UiDialogProvider } from "../src/components/UiDialogProvider";
import { useAssetTransferHistoryRuntime } from "../src/hooks/useAssetTransferHistoryRuntime";
import { useAssetTransferRuntime } from "../src/hooks/useAssetTransferRuntime";
import { useBancorRuntime } from "../src/hooks/useBancorRuntime";
import { useNniRuntime } from "../src/hooks/useNniRuntime";
import "../src/index.css";

function Fixture() {
  const [scope, setScope] = useState("A");
  const [requestCount, setRequestCount] = useState(0);
  const delay = useRef(400);
  const apiFetch = async (path: string) => {
    setRequestCount(count => count + 1);
    await new Promise(resolve => setTimeout(resolve, delay.current));
    const data = path.includes("/transfers?") ? {
      owner_pubkey: "owner", page: 1, per_page: 100, source_filter: "all",
      direction_filter: "all", transactions: [],
    } : path === "/v1/nni/config" ? {
      joined: false, remote_nodes: ["https://node.example.test"], asset_owner_pubkey: "owner",
    } : path === "/v1/nni/assets/transfer" ? {
      status: "asset_transfer_completed", transfer: { transfer_id: "mock-transfer" },
    } : { market_id: scope };
    return new Response(JSON.stringify({ ok: true, data }), { status: 200 });
  };
  const t = (_zh: string, en: string) => en;
  const bank = useBancorRuntime({ apiFetch, t, cacheScope: "mock-endpoint", requestScope: scope });
  const history = useAssetTransferHistoryRuntime({ apiFetch, t, requestScope: scope });
  const transfer = useAssetTransferRuntime({ apiFetch, t, requestScope: scope });
  const nni = useNniRuntime({ apiFetch, t, lang: "en", requestScope: scope });
  const start = (milliseconds: number) => {
    delay.current = milliseconds;
    void bank.fetchMarket();
    void history.load("owner");
    void nni.fetchNniConfig();
    void transfer.transfer({
      asset: "AIC", amount: "1", recipientPublicKey: "recipient", memo: "",
      authorizationMode: "delegated_hardware",
    });
  };
  return <main className="mx-auto max-w-3xl space-y-4 p-4">
    <h1 className="text-lg">Financial request ownership</h1>
    <div className="flex flex-wrap gap-2">
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={() => start(400)}>Slow requests</button>
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={() => start(10)}>Fresh requests</button>
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={() => setScope(scope === "A" ? "B" : "A")}>Switch owner</button>
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={() => setScope("")}>Sign out</button>
    </div>
    <dl className="grid grid-cols-2 gap-2 text-sm">
      <dt>Owner</dt><dd data-testid="owner">{scope || "signed-out"}</dd>
      <dt>Market</dt><dd data-testid="market">{bank.market?.market_id ?? "none"}</dd>
      <dt>History</dt><dd data-testid="history">{history.history?.owner_pubkey ?? "none"}</dd>
      <dt>Binding</dt><dd data-testid="binding">{nni.nniAssetOwnerPubkey ?? "none"}</dd>
      <dt>Transfer</dt><dd data-testid="transfer">{transfer.lastTransfer?.status ?? "none"}</dd>
      <dt>Loading</dt><dd data-testid="loading">{String(bank.marketLoading || history.loading || nni.nniConfigLoading || transfer.loading)}</dd>
      <dt>Requests</dt><dd data-testid="requests">{requestCount}</dd>
    </dl>
    <output data-testid="error">{bank.error ?? history.error ?? transfer.error ?? nni.nniConfigError ?? ""}</output>
  </main>;
}

createRoot(document.getElementById("root")!).render(<UiDialogProvider><Fixture /></UiDialogProvider>);
