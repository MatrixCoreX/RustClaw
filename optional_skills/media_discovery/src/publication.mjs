const DATE_SELECTORS = {
  douyin: ['[data-e2e="video-create-time"]', '[data-e2e="video-publish-time"]'],
  xiaohongshu: [".bottom-container > .date", ".publish-date", '[data-testid="publish-time"]'],
  kuaishou: [".video-info-time", ".publish-time"],
};

export function normalizePublication(value, now = Date.now()) {
  const text = String(value ?? "").trim();
  if (!text || text.length > 128 || /[\u0000-\u001f\u007f]/u.test(text)) return null;
  if (/^\d{10,13}$/u.test(text)) {
    const timestamp = Number(text) * (text.length <= 10 ? 1000 : 1);
    if (timestamp >= Date.UTC(2000, 0, 1) && timestamp <= now + 300_000) {
      return new Date(timestamp).toISOString();
    }
    return null;
  }
  if (/^\d{4}-\d{2}-\d{2}$/u.test(text)) {
    const date = new Date(`${text}T00:00:00Z`);
    return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === text ? text : null;
  }
  // Do not invent a timezone, year, or absolute date for a platform's relative label.
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/u.test(text)) return null;
  const timestamp = Date.parse(text);
  return Number.isFinite(timestamp) && timestamp >= Date.UTC(2000, 0, 1) && timestamp <= now + 300_000
    ? new Date(timestamp).toISOString() : null;
}

export async function capturePublication(scope, platform, itemId) {
  const page = typeof scope.page === "function" ? scope.page() : scope;
  const selector = [...(DATE_SELECTORS[platform] || []),
    'time[datetime]:not([itemprop="dateModified"])', '[itemprop="datePublished"]'].join(",");
  const id = String(itemId).split(":").at(-1);
  const nodes = await scope.locator(selector).evaluateAll((elements, id) => elements.slice(0, 16).flatMap(node => {
    if (node.closest('[data-comment-id], [data-e2e*="comment"], .comment-item, .comments-container')) return [];
    const card = node.closest('[data-aweme-id], [data-note-id], [data-photo-id]');
    if (card && !["data-aweme-id", "data-note-id", "data-photo-id"].some(key => card.getAttribute(key) === id)) return [];
    const rect = node.getBoundingClientRect();
    const style = getComputedStyle(node);
    const visible = rect.width > 0 && rect.height > 0 && style.display !== "none" && style.visibility !== "hidden";
    const machine = node.getAttribute("datetime") || node.getAttribute("content");
    if (!visible && node.getAttribute("itemprop") !== "datePublished") return [];
    return [{ value: machine || node.textContent?.trim(), source: machine ? "dom_attribute" : "dom_label" }];
  }), id);
  const structured = await page.evaluate(({ platform, id }) => {
    const idKeys = { douyin: ["aweme_id", "awemeId"], xiaohongshu: ["noteId", "note_id"], kuaishou: ["photoId", "photo_id"] }[platform] || [];
    const timeKeys = platform === "xiaohongshu"
      ? ["publishTime", "publish_time", "createTime", "create_time", "time"]
      : ["publishTime", "publish_time", "createTime", "create_time", "timestamp"];
    const roots = [window.__INITIAL_STATE__, window.__NEXT_DATA__, window._SSR_HYDRATED_DATA];
    for (const script of document.querySelectorAll('script[type="application/json"], script[type="application/ld+json"], script#RENDER_DATA')) {
      if (script.textContent.length > 5_000_000) continue;
      try { roots.push(JSON.parse(script.id === "RENDER_DATA" ? decodeURIComponent(script.textContent) : script.textContent)); } catch { /* Non-JSON scripts are not executed. */ }
    }
    const seen = new Set();
    const queue = roots.filter(Boolean);
    for (let cursor = 0; cursor < queue.length && cursor < 20_000; cursor += 1) {
      const node = queue[cursor];
      if (!node || typeof node !== "object" || seen.has(node)) continue;
      seen.add(node);
      if (idKeys.some(key => String(node[key] ?? "") === id)) {
        for (const key of timeKeys) {
          if (typeof node[key] === "number" || typeof node[key] === "string") return { value: node[key], source: `post_state:${key}` };
        }
      }
      if (node.datePublished && [node.url, node["@id"], node.mainEntityOfPage?.["@id"]].some(url => {
        try { return new URL(url, location.href).pathname.split("/").includes(id); } catch { return false; }
      })) return { value: node.datePublished, source: "json_ld:datePublished" };
      for (const value of Object.values(node).slice(0, 500)) {
        if (queue.length < 20_000 && value && typeof value === "object") queue.push(value);
      }
    }
    return null;
  }, { platform, id });
  const candidates = [...nodes.filter(node => node.source === "dom_attribute"), structured, ...nodes].filter(Boolean);
  for (const candidate of candidates) {
    const published = normalizePublication(candidate.value);
    if (published) return { published_at: published, publication_text: null, publication_source: candidate.source };
  }
  const label = nodes.find(node => node.source === "dom_label" && typeof node.value === "string"
    && node.value.length <= 128 && !/[\u0000-\u001f\u007f]/u.test(node.value));
  return { published_at: null, publication_text: label?.value || null, publication_source: label?.source || null };
}
