import type { LocalMdnsStatus, NginxUiStatus, WebdExposureStatus } from "../types/api";

export function normalizeLocalMdnsHostname(value: string): string | null {
  const normalized = value.trim().toLowerCase();
  const hostname = normalized.endsWith(".local") ? normalized.slice(0, -6) : normalized;
  if (
    hostname.length < 1
    || hostname.length > 63
    || hostname.startsWith("-")
    || hostname.endsWith("-")
    || !/^[a-z0-9-]+$/.test(hostname)
  ) {
    return null;
  }
  return hostname;
}

export interface LocalMdnsAddresses {
  http: string | null;
  https: string | null;
}

export function buildLocalMdnsAddresses(
  status: LocalMdnsStatus | null,
  nginx: NginxUiStatus | null,
  webd: WebdExposureStatus | null,
): LocalMdnsAddresses {
  if (!status?.mdns_name) return { http: null, https: null };
  const nginxReady = Boolean(nginx?.running && nginx.configured && nginx.ui_deployed);
  return {
    http: nginxReady
      ? `http://${status.mdns_name}/`
      : webd?.externally_accessible
        ? `http://${status.mdns_name}:${webd.port}/`
        : null,
    https: nginxReady && nginx?.local_https_enabled
      ? `https://${status.mdns_name}/`
      : null,
  };
}
