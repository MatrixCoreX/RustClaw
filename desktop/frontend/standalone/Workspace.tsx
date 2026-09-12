import { copy, LanguageToggle, useLanguage } from "../i18n";
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { PRODUCT_DISPLAY_NAME } from "../../../UI/src/lib/product-identity";
import { UiDialogProvider } from "../../../UI/src/components/UiDialogProvider";
import { walletError } from "../wallet/errors";
import { clearStandaloneRuntime } from "../runtime";
import { connectNode, type AssetConnection, type NodeList, type AssetNode } from "./network";
import { Pages } from "./Pages";
import { ThemeToggle } from "../ThemeToggle";
import { StandaloneAssetViewContext } from "../wallet/context";
import "../wallet/wallet.css";

export default function Workspace({ initialPage, onHome }: { initialPage: "assets" | "bancor"; onHome: () => void }) {
  useLanguage();
  const [page, setPage] = useState<"assets" | "bancor">(initialPage);
  const [nodes, setNodes] = useState<AssetNode[]>([]);
  const [selectedNode, setSelectedNode] = useState("");
  const [connection, setConnection] = useState<AssetConnection | null>(null);
  const [busy, setBusy] = useState(true);
  const [optimizing, setOptimizing] = useState(true);
  const running = useRef(false);
  const mounted = useRef(true);
  const [error, setError] = useState("");
  const [theme, setTheme] = useState(() => document.documentElement.dataset.theme || "dark");
  const connect = async (nodeId?: string) => {
    if (running.current) return false;
    running.current = true; setBusy(true); setOptimizing(!nodeId); setError("");
    try {
      const result = await connectNode(nodeId);
      if (mounted.current) { setSelectedNode(result.node.id); setConnection(result); }
      return true;
    } catch (e) { if (mounted.current) setError(walletError(e)); return false; }
    finally { running.current = false; if (mounted.current) setBusy(false); }
  };
  useEffect(() => {
    mounted.current = true;
    void invoke<NodeList>("wallet_nodes").then(result => {
      if (!mounted.current) return;
      setNodes(result.nodes); setSelectedNode(result.selected);
      void connect();
    }).catch(e => { setError(walletError(e)); setBusy(false); });
    return () => { mounted.current = false; };
  }, []);
  const onNode = async (origin: string) => {
    const node = nodes.find(n => n.origin === origin);
    return node ? connect(node.id) : false;
  };
  const onAddNode = async (origin: string) => {
    if (running.current) return false;
    try {
      const node = await invoke<AssetNode>("wallet_add_node", { origin });
      setNodes(current => current.some(n => n.id === node.id) ? current : [...current, node]);
      return await connect(node.id);
    } catch (e) { setError(walletError(e)); return false; }
  };
  const home = async () => {
    if (busy) return;
    setBusy(true);
    try { await invoke("wallet_disconnect_node"); clearStandaloneRuntime(); onHome(); }
    catch (e) { setError(walletError(e)); setBusy(false); }
  };
  return <div className="desktop-asset-workspace desktop-console theme-shell" data-standalone-page={page}>
    <header className="desktop-asset-header">
      <button className="theme-secondary-btn" disabled={busy} onClick={() => void home()}>{copy("← 首页")}</button>
      <div><strong>{PRODUCT_DISPLAY_NAME}  {copy("· 本地资产")}</strong><span>{connection?.node.origin || copy("连接资产服务")}
        {connection && <> · {connection.automatic ? copy("已优选") : copy("手动选择")}  {copy("· 响应")} {connection.response_ms} ms</>}</span></div>
      <nav aria-label={copy("本地资产导航")}><button aria-current={page === "assets" ? "page" : undefined} disabled={busy} onClick={() => setPage("assets")}>{copy("资产 / 转账")}</button>
        <button aria-current={page === "bancor" ? "page" : undefined} disabled={busy} onClick={() => setPage("bancor")}>Bancor</button></nav>
      <button className="theme-secondary-btn" data-prefer-node disabled={busy} onClick={() => void connect()}
        title={copy("重新检查已保存节点，优先连接同一账本内响应快的可用节点")}>{busy && optimizing ? copy("正在优选…") : copy("优选节点")}</button>
      <LanguageToggle className="theme-secondary-btn" />
      <ThemeToggle className="theme-secondary-btn" theme={theme} onToggle={() => {
        const next = theme === "light" ? "dark" : "light";
        setTheme(next); document.documentElement.dataset.theme = next;
        localStorage.setItem("agent-runtime.monitor.themeMode", next);
      }} />
    </header>
    <main className="desktop-asset-content">
      {connection && error && <p className="wallet-error" role="alert">{copy(error)}</p>}
      {connection ? <StandaloneAssetViewContext.Provider value={true}><UiDialogProvider><Pages key={connection.id} connection={connection} nodes={nodes} page={page}
        onPage={setPage} onNode={onNode} onAddNode={onAddNode} nodeBusy={busy} nodeError={copy(error)} /></UiDialogProvider></StandaloneAssetViewContext.Provider>
        : <section className="theme-panel-soft desktop-node-connect">
          <h1>{busy ? (optimizing ? copy("正在优选资产节点…") : copy("正在连接资产服务…")) : copy("选择资产服务")}</h1>
          <p>{copy("自动检查已保存节点，优先连接响应快的可用节点。也可以手动选择。")}</p>
          <label>{copy("资产节点")}<select className="theme-input" disabled={busy} value={selectedNode} onChange={e => setSelectedNode(e.target.value)}>
            {nodes.map(node => <option value={node.id} key={node.id}>{node.origin}</option>)}
          </select></label>
          <button className="theme-primary-btn" disabled={busy || !selectedNode} onClick={() => void connect(selectedNode)}>{busy ? copy("正在连接…") : copy("连接")}</button>
          {!busy && <form onSubmit={e => { e.preventDefault(); const value = new FormData(e.currentTarget).get("node"); if (typeof value === "string") void onAddNode(value); }}>
            <label>{copy("添加其他节点")}<input className="theme-input" name="node" type="url" required placeholder="https://node.example.com" /></label>
            <button className="theme-secondary-btn">{copy("添加并连接")}</button>
          </form>}
          {error && <p className="wallet-error" role="alert">{copy(error)}</p>}
        </section>}
    </main>
  </div>;
}
