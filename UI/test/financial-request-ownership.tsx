import { useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { UiDialogProvider } from "../src/components/UiDialogProvider";
import { useAssetTransferHistoryRuntime } from "../src/hooks/useAssetTransferHistoryRuntime";
import { useAssetTransferRuntime } from "../src/hooks/useAssetTransferRuntime";
import { useBancorRuntime } from "../src/hooks/useBancorRuntime";
import { useNniRuntime } from "../src/hooks/useNniRuntime";
import "../src/index.css";

const nodeA = "https://node.example.test";
const nodeB = "https://other-node.example.test";
const targetOwner = "5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M";

function Fixture() {
  const [scope, setScope] = useState("A");
  const [requestCount, setRequestCount] = useState(0);
  const delay = useRef(400);
  const config = useRef({
    joined: false, remote_nodes: [nodeA, nodeB], asset_owner_pubkey: "owner",
    selected_node_url: nodeA, asset_service_node_url: nodeA, bancor_service_node_url: nodeA,
  });
  const writes = useRef({ pending: 0, peak: 0, signatures: 0 });
  const apiFetch = async (path: string, init?: RequestInit) => {
    setRequestCount(count => count + 1);
    const isConfigWrite = path === "/v1/nni/config" && init?.method === "POST";
    if (isConfigWrite) {
      writes.current.pending++;
      writes.current.peak = Math.max(writes.current.peak, writes.current.pending);
    }
    if (path === "/v1/nni/device/action") writes.current.signatures++;
    await new Promise(resolve => setTimeout(resolve, delay.current));
    if (isConfigWrite) {
      config.current = { ...config.current, ...JSON.parse(String(init?.body)) };
      writes.current.pending--;
    }
    const data = path.includes("/transfers?") ? {
      owner_pubkey: "owner", page: 1, per_page: 100, source_filter: "all",
      direction_filter: "all", transactions: [],
    } : path === "/v1/nni/config" ? config.current
      : path.startsWith("/v1/nni/device/status") ? { signature_chip_present: true }
      : path === "/v1/nni/join/request" ? {
        task_id: "fixture-authorization", node_url: nodeA, challenge: "fixture",
        owner_signature_required: true,
      } : path === "/v1/nni/device/action" ? { payload: { signature: "ab".repeat(64) } }
      : path === "/v1/nni/assets/transfer" ? {
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
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={() => { delay.current = 10; void nni.fetchNniDeviceStatus(); }}>Prepare signer</button>
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={() => { delay.current = 400; void nni.startNniCustomOwnerAuthorization(targetOwner); }}>Slow authorization</button>
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={nni.cancelNniOwnerAuthorization}>Cancel authorization</button>
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={() => {
        delay.current = 10;
        void nni.updateNniAssetServiceNodeUrl(nodeB);
        void nni.updateNniBancorServiceNodeUrl(nodeB);
      }}>Save service nodes</button>
    </div>
    <dl className="grid grid-cols-2 gap-2 text-sm">
      <dt>Owner</dt><dd data-testid="owner">{scope || "signed-out"}</dd>
      <dt>Market</dt><dd data-testid="market">{bank.market?.market_id ?? "none"}</dd>
      <dt>History</dt><dd data-testid="history">{history.history?.owner_pubkey ?? "none"}</dd>
      <dt>Binding</dt><dd data-testid="binding">{nni.nniAssetOwnerPubkey ?? "none"}</dd>
      <dt>Transfer</dt><dd data-testid="transfer">{transfer.lastTransfer?.status ?? "none"}</dd>
      <dt>Loading</dt><dd data-testid="loading">{String(bank.marketLoading || history.loading || nni.nniConfigLoading || transfer.loading)}</dd>
      <dt>Requests</dt><dd data-testid="requests">{requestCount}</dd>
      <dt>Signer ready</dt><dd data-testid="signer-ready">{String(nni.nniStatus?.signature_chip_present === true)}</dd>
      <dt>Authorization</dt><dd data-testid="authorization">{nni.nniOwnerAuthorizationChallenge?.taskId ?? "none"}</dd>
      <dt>Owner action</dt><dd data-testid="owner-action">{nni.nniOwnerActionLoading ?? "none"}</dd>
      <dt>Signatures</dt><dd data-testid="signatures">{writes.current.signatures}</dd>
      <dt>Peak writes</dt><dd data-testid="peak-writes">{writes.current.peak}</dd>
      <dt>Asset node</dt><dd className="break-all" data-testid="asset-node">{nni.nniAssetServiceNodeUrl}</dd>
      <dt>Bancor node</dt><dd className="break-all" data-testid="bancor-node">{nni.nniBancorServiceNodeUrl}</dd>
      <dt>Saving nodes</dt><dd data-testid="saving-nodes">{String(nni.nniAssetServiceNodeSaving || nni.nniBancorServiceNodeSaving)}</dd>
    </dl>
    <output data-testid="error">{bank.error ?? history.error ?? transfer.error ?? nni.nniConfigError ?? ""}</output>
  </main>;
}

createRoot(document.getElementById("root")!).render(<UiDialogProvider><Fixture /></UiDialogProvider>);
