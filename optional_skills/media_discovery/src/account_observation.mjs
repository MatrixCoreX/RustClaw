import { writeAccount } from "./account_sessions.mjs";

const AUTH = {
  douyin: ["douyin.com", ["sessionid", "sessionid_ss", "sid_guard"]],
  xiaohongshu: ["xiaohongshu.com", ["web_session"]],
  kuaishou: ["kuaishou.com", ["kuaishou.server.web_st", "kuaishou.server.webday7_st"]],
  toutiao: ["toutiao.com", ["sessionid", "sid_guard"]],
  weibo: ["weibo.com", ["SUB"]],
  tiktok: ["tiktok.com", ["sessionid", "sessionid_ss", "sid_tt"]],
  youtube: ["youtube.com", ["SID", "SAPISID", "__Secure-3PSID"]],
  bilibili: ["bilibili.com", ["SESSDATA"]],
};

export function selfAccountName(platform, url, value) {
  let parsed;
  try { parsed = new URL(url); } catch { return null; }
  const domain = AUTH[platform]?.[0];
  if (!domain || !(parsed.hostname === domain || parsed.hostname.endsWith(`.${domain}`))) return null;
  const endpoint = parsed.pathname.replace(/\/$/u, "");
  let user = null;
  if (platform === "douyin" && endpoint === "/aweme/v1/web/user/profile/self") user = value?.user;
  if (platform === "xiaohongshu" && endpoint === "/api/sns/web/v1/user/selfinfo") user = value?.data?.basic_info || value?.data;
  if (platform === "weibo" && endpoint === "/ajax/config") user = value?.data?.user;
  if (platform === "tiktok" && endpoint === "/passport/web/account/info") user = value?.data;
  if (platform === "bilibili" && endpoint === "/x/web-interface/nav" && value?.data?.isLogin === true) user = value.data;
  const name = user?.nickname || user?.screen_name || user?.uname;
  return typeof name === "string" && name.trim() && name.length <= 256 && !/[\u0000-\u001f\u007f]/u.test(name) ? name.trim() : null;
}

async function optionalMetadata(promise, fallback) {
  let timer;
  try {
    return await Promise.race([promise.catch(() => fallback),
      new Promise(resolve => { timer = setTimeout(() => resolve(fallback), 750); })]);
  } finally { clearTimeout(timer); }
}

export function observeAccount(root, platform, context) {
  const [domain, authNames] = AUTH[platform];
  let attached = false;
  let displayName = null;
  const pending = new Set();
  const track = promise => { pending.add(promise); void promise.finally(() => pending.delete(promise)); };
  const ownedUrl = raw => {
    try { const host = new URL(raw).hostname; return host === domain || host.endsWith(`.${domain}`); }
    catch { return false; }
  };
  const request = value => {
    if (!ownedUrl(value.url())) return;
    track(value.allHeaders().then(headers => {
      const names = new Set((headers.cookie || "").split(";").flatMap(item => {
        const separator = item.indexOf("=");
        return separator > 0 && item.slice(separator + 1).trim() ? [item.slice(0, separator).trim()] : [];
      }));
      if (authNames.some(name => names.has(name))) attached = true;
    }).catch(() => {}));
  };
  const response = value => {
    if (!ownedUrl(value.url())) return;
    const pathname = new URL(value.url()).pathname.replace(/\/$/u, "");
    if (!["/aweme/v1/web/user/profile/self", "/api/sns/web/v1/user/selfinfo", "/ajax/config", "/passport/web/account/info", "/x/web-interface/nav"].includes(pathname)) return;
    track(value.text().then(text => {
      if (text.length > 262144) return;
      const name = selfAccountName(platform, value.url(), JSON.parse(text));
      if (name) displayName = name;
    }).catch(() => {}));
  };
  context.on("request", request);
  context.on("response", response);
  return {
    async capture() {
      // Account metadata is optional; a stalled telemetry response must not hold
      // a media result or browser shutdown open. This is not a task deadline.
      await optionalMetadata(Promise.allSettled([...pending]), []);
      let saved = false;
      try {
        const now = Date.now() / 1000;
        saved = (await optionalMetadata(context.cookies(), [])).some(cookie => authNames.includes(cookie.name) && cookie.value
          && (cookie.expires <= 0 || cookie.expires > now)
          && (cookie.domain.replace(/^\./u, "") === domain || cookie.domain.endsWith(`.${domain}`)));
      } catch { /* An inaccessible cookie store is unknown, not signed out. */ }
      const account = { schema_version: 1, platform,
        status: displayName && attached ? "verified" : saved || attached ? "saved_session" : "unknown",
        display_name: displayName && attached ? displayName : null,
        checked_at: Math.floor(Date.now() / 1000), session_present: saved || attached ? true : null };
      try { await writeAccount(root, account); } catch { /* Metadata cannot drop media results. */ }
      return { schema_version: 1, platform, mode: attached ? "authenticated_browser" : "unknown", account };
    },
    close() { context.off("request", request); context.off("response", response); },
  };
}
