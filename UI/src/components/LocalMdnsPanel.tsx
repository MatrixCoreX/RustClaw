import { useCallback, useEffect, useMemo, useState } from "react";
import { ExternalLink, Loader2, Network, RefreshCw, Save } from "lucide-react";

import { buildLocalMdnsAddresses, normalizeLocalMdnsHostname } from "../lib/local-mdns";
import type {
  ApiResponse,
  LocalMdnsStatus,
  LocalMdnsUpdateResult,
  NginxUiStatus,
  WebdExposureStatus,
} from "../types/api";
import type { GitRemoteSetupPanelProps } from "./GitRemoteSetupPanel";
import { useUiDialog } from "./UiDialogProvider";

type Translate = (zh: string, en: string) => string;

interface LocalMdnsPanelProps {
  t: Translate;
  apiFetch: GitRemoteSetupPanelProps["apiFetch"];
  nginxStatus: NginxUiStatus | null;
  webdExposureStatus: WebdExposureStatus | null;
  disabled: boolean;
  onUpdated: () => unknown | Promise<unknown>;
}

function localMdnsErrorMessage(error: string, t: Translate): string {
  const messages: Record<string, [string, string]> = {
    local_mdns_hostname_invalid: ["名称只能包含小写字母、数字和连字符，长度不能超过 63 个字符。", "Use only letters, numbers, and hyphens, up to 63 characters."],
    local_mdns_update_in_progress: ["名称正在修改，请稍后刷新。", "The local name is already being changed. Refresh shortly."],
    local_mdns_update_failed: ["设备名称没有修改成功，请检查系统的 mDNS 服务。", "The device name was not changed. Check the system mDNS service."],
    local_mdns_command_failed: ["设备无法运行名称配置程序，请检查安装文件。", "The device could not run the local-name setup program. Check the installation files."],
    admin_required: ["只有管理员可以修改局域网名称。", "Only an administrator can change the local network name."],
  };
  const copy = messages[error];
  return copy ? t(copy[0], copy[1]) : t("操作未完成，请刷新后重试。", "The change was not completed. Refresh and try again.");
}

export function LocalMdnsPanel({
  t,
  apiFetch,
  nginxStatus,
  webdExposureStatus,
  disabled,
  onUpdated,
}: LocalMdnsPanelProps) {
  const { confirm } = useUiDialog();
  const [status, setStatus] = useState<LocalMdnsStatus | null>(null);
  const [hostname, setHostname] = useState("");
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const addresses = useMemo(
    () => buildLocalMdnsAddresses(status, nginxStatus, webdExposureStatus),
    [nginxStatus, status, webdExposureStatus],
  );

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const response = await apiFetch("/v1/admin/local-mdns");
      const body = (await response.json()) as ApiResponse<LocalMdnsStatus>;
      if (!response.ok || !body.ok || !body.data) throw new Error(body.error || "local_mdns_status_failed");
      setStatus(body.data);
      setHostname(body.data.hostname);
    } catch (caught) {
      setError(localMdnsErrorMessage(caught instanceof Error ? caught.message : "local_mdns_status_failed", t));
    } finally {
      setLoading(false);
    }
  }, [apiFetch, t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const save = async () => {
    const normalized = normalizeLocalMdnsHostname(hostname);
    if (!normalized) {
      setError(localMdnsErrorMessage("local_mdns_hostname_invalid", t));
      return;
    }
    if (normalized === status?.hostname) return;
    const accepted = await confirm({
      title: t("修改局域网访问名称", "Change local network name"),
      message: t(
        `修改后，旧地址 ${status?.mdns_name || ""} 将停止使用。设备 IP 地址仍然可用。确定改为 ${normalized}.local 吗？`,
        `After this change, the old address ${status?.mdns_name || ""} will stop working. The device IP remains available. Change it to ${normalized}.local?`,
      ),
      confirmLabel: t("确认修改", "Change name"),
    });
    if (!accepted) return;

    setSaving(true);
    setError(null);
    setMessage(null);
    try {
      const response = await apiFetch("/v1/admin/local-mdns", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ hostname: normalized }),
      });
      const body = (await response.json()) as ApiResponse<LocalMdnsUpdateResult>;
      if (!response.ok || !body.ok || !body.data) throw new Error(body.error || "local_mdns_update_failed");
      setStatus(body.data.status);
      setHostname(body.data.status.hostname);
      setMessage(body.data.https_refresh_error_code
        ? t(
          "局域网名称已修改，但 HTTPS 证书没有续签成功。HTTP 地址仍可使用，请重新运行 HTTPS 设置。",
          "The local name changed, but the HTTPS certificate could not be renewed. HTTP still works; run HTTPS setup again.",
        )
        : t(
          `名称已修改为 ${body.data.status.mdns_name}。HTTP 可直接使用，HTTPS 仍是可选功能。`,
          `The name is now ${body.data.status.mdns_name}. HTTP works directly; HTTPS remains optional.`,
        ));
      await onUpdated();
    } catch (caught) {
      setError(localMdnsErrorMessage(caught instanceof Error ? caught.message : "local_mdns_update_failed", t));
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="rounded-lg border border-violet-300/20 bg-violet-300/[0.05] p-4 sm:p-5" aria-labelledby="local-mdns-heading">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex min-w-0 items-start gap-3">
          <span className="rounded-lg bg-violet-300/10 p-2 text-violet-100">
            <Network className="h-5 w-5" />
          </span>
          <div className="min-w-0">
            <h4 id="local-mdns-heading" className="text-sm font-semibold text-white">
              {t("局域网访问名称", "Local network name")}
            </h4>
            <p className="mt-2 max-w-3xl text-sm leading-6 text-white/65">
              {t(
                "设置容易记住的 .local 地址。mDNS 不依赖 HTTPS，设置完成后可先通过 HTTP 使用；HTTPS 只在需要加密局域网传输时开启。",
                "Set an easy-to-remember .local address. mDNS does not depend on HTTPS, so HTTP works first; enable HTTPS only when you want encrypted LAN traffic.",
              )}
            </p>
          </div>
        </div>
        <button
          type="button"
          className="theme-topbar-btn px-3 py-2 text-sm"
          disabled={loading || saving || disabled}
          onClick={() => void refresh()}
        >
          {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
          {t("刷新状态", "Refresh status")}
        </button>
      </div>

      <div className="mt-4 grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(260px,0.7fr)]">
        <div>
          <label htmlFor="local-mdns-hostname" className="text-sm font-medium text-white/80">
            {t("设备名称", "Device name")}
          </label>
          <div className="mt-2 flex max-w-xl items-stretch">
            <input
              id="local-mdns-hostname"
              value={hostname}
              onChange={(event) => setHostname(event.target.value)}
              disabled={saving || disabled || status?.supported === false}
              className="theme-input min-w-0 flex-1 rounded-r-none"
              placeholder="home-agent"
              autoComplete="off"
              spellCheck={false}
            />
            <span className="flex items-center rounded-r-md border border-l-0 border-white/10 bg-black/15 px-3 text-sm text-white/55">
              .local
            </span>
          </div>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <button
              type="button"
              className="theme-primary-btn px-3 py-2 text-sm"
              disabled={saving || disabled || loading || status?.supported === false || normalizeLocalMdnsHostname(hostname) === status?.hostname}
              onClick={() => void save()}
            >
              {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Save className="h-4 w-4" />}
              {saving ? t("修改中", "Changing") : t("保存名称", "Save name")}
            </button>
            <span className={`rounded-full border px-2.5 py-1 text-xs ${status?.responder_running ? "border-emerald-400/20 bg-emerald-400/[0.08] text-emerald-100" : "border-amber-400/20 bg-amber-400/[0.08] text-amber-100"}`}>
              {status?.responder_running ? t("局域网广播正常", "Local discovery running") : t("局域网广播未运行", "Local discovery stopped")}
            </span>
          </div>
        </div>

        <div className="rounded-md border border-white/10 bg-black/10 p-3">
          <p className="text-xs font-medium text-white/55">{t("可用地址", "Available addresses")}</p>
          <div className="mt-2 space-y-2 text-sm">
            {[addresses.http, addresses.https].filter((value): value is string => Boolean(value)).map((address) => (
              <a key={address} href={address} className="flex items-center gap-2 break-all text-sky-200 hover:text-sky-100">
                <ExternalLink className="h-3.5 w-3.5 shrink-0" />
                {address}
              </a>
            ))}
            {!addresses.http && !addresses.https ? (
              <p className="leading-5 text-white/55">
                {t("请先启用 nginx，或开放 WEBD 对外端口。", "Enable nginx or expose the WEBD port first.")}
              </p>
            ) : null}
          </div>
        </div>
      </div>

      {status?.supported === false ? (
        <p className="mt-3 text-sm text-amber-100">{t("当前系统暂不支持自动修改 mDNS 名称。", "This system does not support automatic mDNS name changes.")}</p>
      ) : null}
      {error ? <p className="mt-3 text-sm text-amber-100">{error}</p> : null}
      {message ? <p className="mt-3 text-sm text-emerald-100">{message}</p> : null}
    </section>
  );
}
