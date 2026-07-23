export type BrowserLocation = Pick<Location, "href" | "hostname" | "port" | "protocol">;

const LOCAL_FRONTEND_PORTS = new Set(["3000", "4173"]);

function originWithPort(location: BrowserLocation, port: string): string | null {
  try {
    const url = new URL(location.href);
    url.port = port;
    return url.origin;
  } catch {
    const hostname = location.hostname.trim();
    if (!hostname) return null;
    const protocol =
      location.protocol && location.protocol !== "file:" ? location.protocol : "http:";
    const formattedHost = hostname.includes(":") ? `[${hostname}]` : hostname;
    return `${protocol}//${formattedHost}:${port}`;
  }
}

function currentHttpOrigin(location: BrowserLocation): string | null {
  if (location.protocol !== "http:" && location.protocol !== "https:") return null;
  try {
    return new URL(location.href).origin;
  } catch {
    return null;
  }
}

export function defaultClawdBaseUrl(
  location?: BrowserLocation,
): string {
  if (!location) return "http://127.0.0.1:8787";
  if (LOCAL_FRONTEND_PORTS.has(location.port)) {
    return originWithPort(location, "8787") ?? "http://127.0.0.1:8787";
  }
  return currentHttpOrigin(location) ??
    originWithPort(location, "8787") ??
    "http://127.0.0.1:8787";
}

export function defaultWebdBaseUrl(
  location?: BrowserLocation,
): string {
  if (!location) return "http://127.0.0.1:8788";
  if (LOCAL_FRONTEND_PORTS.has(location.port) || location.port === "8787") {
    return originWithPort(location, "8788") ?? "http://127.0.0.1:8788";
  }
  return currentHttpOrigin(location) ??
    originWithPort(location, "8788") ??
    "http://127.0.0.1:8788";
}

function preferredServiceBaseUrl(
  stored: string | null,
  location: BrowserLocation | undefined,
  defaultUrl: string,
  legacyPort: string,
): string {
  const normalized = stored?.trim() ?? "";
  if (!normalized) return defaultUrl;
  const legacyDefault = location ? originWithPort(location, legacyPort) : null;
  if (legacyDefault && normalized === legacyDefault && legacyDefault !== defaultUrl) {
    return defaultUrl;
  }
  return normalized;
}

export function preferredClawdBaseUrl(
  stored: string | null,
  location?: BrowserLocation,
): string {
  return preferredServiceBaseUrl(
    stored,
    location,
    defaultClawdBaseUrl(location),
    "8787",
  );
}

export function preferredWebdBaseUrl(
  stored: string | null,
  location?: BrowserLocation,
): string {
  return preferredServiceBaseUrl(
    stored,
    location,
    defaultWebdBaseUrl(location),
    "8788",
  );
}
