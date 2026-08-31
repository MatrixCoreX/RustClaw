export const WEBD_CSRF_HEADER = "x-agent-csrf-token";

export type BrowserAuthMode = "key" | "webd" | null;

export function isUnsafeHttpMethod(method: string | undefined): boolean {
  const normalized = (method || "GET").toUpperCase();
  return normalized !== "GET" && normalized !== "HEAD" && normalized !== "OPTIONS";
}

export function normalizeWebdCsrfToken(value: unknown): string | null {
  return typeof value === "string" && /^[0-9a-f]{32}$/.test(value) ? value : null;
}

export function runtimeRequestCredentials(
  withAuth: boolean,
  authMode: BrowserAuthMode,
  requested?: RequestCredentials,
): RequestCredentials {
  if (withAuth && authMode === "webd") return "include";
  return requested ?? "omit";
}

export function buildRuntimeRequestHeaders({
  initialHeaders,
  directAuthHeaders,
  withAuth,
  authMode,
  method,
  csrfToken,
}: {
  initialHeaders?: HeadersInit;
  directAuthHeaders: Record<string, string>;
  withAuth: boolean;
  authMode: BrowserAuthMode;
  method?: string;
  csrfToken: string;
}): Headers {
  const headers = new Headers(initialHeaders);
  if (withAuth && authMode !== "webd") {
    for (const [name, value] of Object.entries(directAuthHeaders)) {
      headers.set(name, value);
    }
  }
  if (withAuth && authMode === "webd" && isUnsafeHttpMethod(method)) {
    const normalized = normalizeWebdCsrfToken(csrfToken);
    if (normalized) headers.set(WEBD_CSRF_HEADER, normalized);
  }
  return headers;
}
