import { LoaderCircle, Network } from "lucide-react";

type Translate = (zh: string, en: string) => string;

function nodeLabel(nodeUrl: string): string {
  try {
    const url = new URL(nodeUrl);
    return url.port ? `${url.hostname}:${url.port}` : url.hostname;
  } catch {
    return nodeUrl;
  }
}

export function FinancialServiceNodeSelector({
  t,
  service,
  nodes,
  selectedNodeUrl,
  saving,
  error,
  disabled = false,
  onChange,
}: {
  t: Translate;
  service: "bancor" | "assets";
  nodes: readonly string[];
  selectedNodeUrl: string;
  saving: boolean;
  error: string | null;
  disabled?: boolean;
  onChange: (nodeUrl: string) => Promise<boolean>;
}) {
  if (nodes.length === 0) return null;
  const unavailable = disabled || saving || nodes.length < 2;
  const title = service === "bancor"
    ? t("BANCOR 节点", "BANCOR node")
    : t("资产节点", "Asset node");
  const description = service === "bancor"
    ? t("用于行情、报价与交易，不改变 NNI 和资产节点。", "Used for markets, quotes, and trades without changing NNI or asset nodes.")
    : t("用于资产余额、转账与历史，不改变 NNI 和 BANCOR 节点。", "Used for balances, transfers, and history without changing NNI or BANCOR nodes.");
  return (
    <div className="flex justify-end" data-financial-service-node-selector={service}>
      <div className="w-full max-w-md rounded-lg border border-[var(--theme-border)] bg-[var(--theme-surface-muted)] px-3 py-2.5">
        <div className="flex items-center gap-2">
          <Network className="h-4 w-4 shrink-0 text-[var(--theme-icon-accent-color)]" />
          <label className="min-w-0 flex-1">
            <span className="block text-xs font-semibold text-[var(--theme-text-strong)]">
              {title}
            </span>
            <span className="mt-0.5 block text-[11px] leading-4 text-[var(--theme-text-muted)]">
              {description}
            </span>
          </label>
          <div className="relative shrink-0">
            <select
              className="theme-input h-9 max-w-44 appearance-none py-1 pl-2.5 pr-8 text-xs disabled:cursor-not-allowed disabled:opacity-60"
              value={selectedNodeUrl}
              disabled={unavailable}
              aria-label={t(`选择${title}`, `Select ${title}`)}
              onChange={(event) => void onChange(event.target.value)}
            >
              {nodes.length === 0 ? (
                <option value="">{t("未配置", "Not configured")}</option>
              ) : nodes.map((nodeUrl) => (
                <option key={nodeUrl} value={nodeUrl} title={nodeUrl}>
                  {nodeLabel(nodeUrl)}
                </option>
              ))}
            </select>
            {saving ? (
              <LoaderCircle className="pointer-events-none absolute right-2.5 top-2.5 h-4 w-4 animate-spin text-[var(--theme-text-muted)]" />
            ) : null}
          </div>
        </div>
        {error ? <p className="mt-2 text-xs text-red-300" role="alert">{error}</p> : null}
      </div>
    </div>
  );
}
