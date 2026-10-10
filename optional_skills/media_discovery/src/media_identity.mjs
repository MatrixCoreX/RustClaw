import { createHash } from "node:crypto";
import { isDetailUrl, platformItemId } from "./platforms.mjs";

export function imageSourceIdentity(platform, source) {
  if (typeof source !== "string" || !source) return null;
  try {
    const url = new URL(source);
    // Only the reviewed CDN object name is stable across signed/processed variants.
    if (platform === "xiaohongshu" && /^https?:$/.test(url.protocol)
      && (url.hostname === "xhscdn.com" || url.hostname.endsWith(".xhscdn.com"))) {
      const object = url.pathname.split("/").at(-1).split("!")[0];
      if (/^1040[a-zA-Z0-9]{20,124}$/.test(object)) return `xhscdn:${object}`;
    }
    if (platform === "toutiao" && /^https?:$/.test(url.protocol)
      && ["toutiaoimg.com", "byteimg.com", "pstatp.com"].some(domain =>
        url.hostname === domain || url.hostname.endsWith(`.${domain}`))) {
      return `toutiao-image:${url.pathname.split("~", 1)[0]}`;
    }
    if (platform === "weibo" && /^https?:$/.test(url.protocol)
      && (url.hostname === "sinaimg.cn" || url.hostname.endsWith(".sinaimg.cn"))) {
      const parts = url.pathname.split("/").filter(Boolean);
      return parts.length > 1 ? `weibo-image:/${parts.slice(1).join("/")}` : url.href;
    }
    return url.href;
  } catch {
    return source;
  }
}

export function identityDigest(identity) {
  return createHash("sha256").update(identity).digest("hex");
}

export function recordPostIdentity(record) {
  const url = record.source_page_url || record.video_page_url;
  if (url && isDetailUrl(record.platform, url)) return platformItemId(record.platform, url);
  if (typeof record.item_id === "string" && record.item_id) {
    return record.item_id.startsWith(`${record.platform}:`)
      ? record.item_id : `${record.platform}:${record.item_id}`;
  }
  return null;
}

export function recordIdentity(record) {
  const post = recordPostIdentity(record);
  if (post && record.kind === "video") return `${post}:video`;
  const source = imageSourceIdentity(record.platform, record.image_url);
  if (post && record.kind === "image" && source) return `${post}:image:${identityDigest(source)}`;
  return `${record.platform}:${record.kind}:legacy:${record.dedup_key}`;
}
