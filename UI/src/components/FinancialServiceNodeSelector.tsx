import { Check, LoaderCircle, Network, Plus, X } from "lucide-react";
import { useState } from "react";

type Translate = (zh: string, en: string) => string;

function nodeLabel(nodeUrl: string): string {
  try {
    const url = new URL(nodeUrl);
    return url.port ? `${url.hostname}:${url.port}` : url.hostname;
  } catch {
    return nodeUrl;
  }
}

export function normalizeCustomFinancialNodeUrl(value: string): string | null {
  const raw = value.trim();
  if (!raw) return null;
  try {
    const url = new URL(raw);
    if ((url.protocol !== "https:" && url.protocol !== "http:") || !url.hostname) return null;
    if (url.username || url.password || url.search || url.hash) return null;
    let pathname = url.pathname.replace(/\/+$/, "");
    if (pathname === "/v1") pathname = "";
    return `${url.protocol}//${url.host}${pathname}`;
  } catch {
    return null;
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
  onAddNode,
}: {
  t: Translate;
  service: "bancor" | "assets";
  nodes: readonly string[];
  selectedNodeUrl: string;
  saving: boolean;
  error: string | null;
  disabled?: boolean;
  onChange: (nodeUrl: string) => Promise<boolean>;
  onAddNode?: (nodeUrl: string) => Promise<boolean>;
}) {
  const [adding, setAdding] = useState(false);
  const [draftNodeUrl, setDraftNodeUrl] = useState("");
  const [draftError, setDraftError] = useState<string | null>(null);
  const unavailable = disabled || saving || nodes.length < 2;
  const actionUnavailable = disabled || saving;
  const title = t("节点切换", "Switch node");
  const closeAddNode = () => {
    setAdding(false);
    setDraftNodeUrl("");
    setDraftError(null);
  };
  const submitAddNode = async () => {
    const normalized = normalizeCustomFinancialNodeUrl(draftNodeUrl);
    if (!normalized) {
      setDraftError(t("请输入有效的 HTTP 或 HTTPS 节点地址。", "Enter a valid HTTP or HTTPS node URL."));
      return;
    }
    setDraftError(null);
    if (await onAddNode?.(normalized)) closeAddNode();
  };
  return (
    <div className="flex justify-end" data-financial-service-node-selector={service}>
      <div className="w-full max-w-md rounded-lg border border-[var(--theme-border)] bg-[var(--theme-surface-muted)] px-3 py-2.5">
        <div className="flex items-center gap-2">
          <Network className="h-4 w-4 shrink-0 text-[var(--theme-icon-accent-color)]" />
          <label className="min-w-0 flex-1">
            <span className="block text-xs font-semibold text-[var(--theme-text-strong)]">
              {title}
            </span>
          </label>
          <div className="relative shrink-0">
            <select
              className="theme-input h-9 max-w-44 appearance-none py-1 pl-2.5 pr-8 text-xs disabled:cursor-not-allowed disabled:opacity-60"
              value={selectedNodeUrl}
              disabled={unavailable}
              aria-label={t("选择节点", "Select node")}
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
          {onAddNode ? (
            <button
              type="button"
              className="theme-icon-btn h-9 w-9 shrink-0"
              disabled={actionUnavailable}
              title={t("添加自定义节点", "Add custom node")}
              aria-label={t("添加自定义节点", "Add custom node")}
              onClick={() => {
                setAdding(true);
                setDraftError(null);
              }}
            >
              <Plus className="h-4 w-4" />
            </button>
          ) : null}
        </div>
        {adding ? (
          <form
            className="mt-2 flex items-start gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              void submitAddNode();
            }}
          >
            <div className="min-w-0 flex-1">
              <input
                autoFocus
                type="url"
                className="theme-input h-9 w-full px-2.5 text-xs"
                value={draftNodeUrl}
                placeholder="https://api.example.com"
                aria-label={t("自定义节点地址", "Custom node URL")}
                disabled={actionUnavailable}
                onChange={(event) => {
                  setDraftNodeUrl(event.target.value);
                  setDraftError(null);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Escape") closeAddNode();
                }}
              />
              {draftError ? <p className="mt-1 text-xs text-red-300" role="alert">{draftError}</p> : null}
            </div>
            <button
              type="submit"
              className="theme-icon-btn h-9 w-9 shrink-0"
              disabled={actionUnavailable || !draftNodeUrl.trim()}
              title={t("添加并切换", "Add and switch")}
              aria-label={t("添加并切换", "Add and switch")}
            >
              {saving ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Check className="h-4 w-4" />}
            </button>
            <button
              type="button"
              className="theme-icon-btn h-9 w-9 shrink-0"
              disabled={saving}
              title={t("取消", "Cancel")}
              aria-label={t("取消", "Cancel")}
              onClick={closeAddNode}
            >
              <X className="h-4 w-4" />
            </button>
          </form>
        ) : null}
        {error ? <p className="mt-2 text-xs text-red-300" role="alert">{error}</p> : null}
      </div>
    </div>
  );
}
