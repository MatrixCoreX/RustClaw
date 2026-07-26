export { writeTextToClipboard } from "./clipboard";

export async function copyAuthKeyValue(options: {
  keyId?: number | null;
  plaintextKey?: string | null;
  fetchFullAuthKey: (keyId: number) => Promise<string>;
  writeClipboard: (value: string) => Promise<void>;
}): Promise<string> {
  const plaintextKey = options.plaintextKey?.trim() ?? "";
  if (plaintextKey) {
    await options.writeClipboard(plaintextKey);
    return plaintextKey;
  }

  if (options.keyId == null) {
    throw new Error("missing auth key id");
  }

  const fullKey = (await options.fetchFullAuthKey(options.keyId)).trim();
  if (!fullKey) {
    throw new Error("empty auth key");
  }

  await options.writeClipboard(fullKey);
  return fullKey;
}

export function maskStoredKey(value: string, keep = 6): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  const visible = trimmed.slice(0, Math.max(1, keep));
  return `${visible}${"*".repeat(Math.max(4, trimmed.length - visible.length))}`;
}

const EXPIRED_AUTH_CODES = new Set(["auth_key_required", "auth_key_invalid"]);

type Translate = (zh: string, en: string) => string;

export function formatAuthenticationError(
  error: string | null | undefined,
  status: number,
  t: Translate,
): string {
  const code = error?.trim() ?? "";
  if (code === "auth_key_required") {
    return t("请先登录。", "Please sign in first.");
  }
  if (code === "auth_key_invalid") {
    return t("访问凭证无效或已停用，请重新登录。", "Your access credential is invalid or disabled. Please sign in again.");
  }
  if (code === "webd_login_upstream_unavailable") {
    return t("登录服务暂时无法连接核心服务，请稍后重试。", "The sign-in service cannot reach the core service right now. Try again shortly.");
  }
  if (
    code === "webd_login_upstream_body_read_failed" ||
    code === "webd_login_upstream_response_invalid"
  ) {
    return t("登录服务收到无效响应，请检查服务状态后重试。", "The sign-in service received an invalid response. Check service status and try again.");
  }
  return code || t(`身份验证失败 (${status})`, `Authentication failed (${status})`);
}

export async function responseIndicatesExpiredAuthentication(
  response: Response,
): Promise<boolean> {
  if (response.status !== 401) return false;
  try {
    const body = (await response.clone().json()) as {
      error?: unknown;
      data?: { error_code?: unknown; status_code?: unknown } | null;
    };
    const candidates = [body.data?.error_code, body.data?.status_code, body.error];
    return candidates.some(
      (value) => typeof value === "string" && EXPIRED_AUTH_CODES.has(value.trim()),
    );
  } catch {
    return false;
  }
}
