import fs from "node:fs/promises";
import path from "node:path";

export function browserStageError(code, stage) {
  return Object.assign(new Error(code), { discovery_stage: stage });
}

export function assertDocumentResponse(response, stage) {
  const contentType = response?.headers?.()["content-type"]?.split(";")[0].trim().toLowerCase();
  if (contentType === "application/json" || contentType?.endsWith("+json")) {
    throw browserStageError("unexpected_page_response", stage);
  }
}

export async function recordBrowserFailure(page, { root, runId, platform, stage, error, timeoutMs = 5000 }) {
  let timer;
  const capture = page.evaluate((platform) => ({
    origin: location.origin,
    pathname: location.pathname.slice(0, 256),
    ready_state: document.readyState,
    platform_error_code: platform === "xiaohongshu" && location.pathname === "/website-login/error"
      ? (/^\d{1,12}$/u.test(new URL(location.href).searchParams.get("error_code") || "")
        ? new URL(location.href).searchParams.get("error_code") : null) : null,
    recommendation_cards: window.document.querySelectorAll({
      douyin: "[data-aweme-id]", xiaohongshu: "section.note-item[data-note-id]", kuaishou: ".video-card",
    }[platform]).length,
    video_elements: window.document.querySelectorAll("video").length,
    visible_videos: Array.from(window.document.querySelectorAll("video")).filter((node) => {
      const rect = node.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    }).length,
    detail_surfaces: window.document.querySelectorAll('[data-e2e="video-detail"]').length,
    next_controls: window.document.querySelectorAll('[data-e2e="video-switch-next-arrow"]').length,
    login_inputs: window.document.querySelectorAll('input[type="password"],input[type="tel"]').length,
    iframe_count: window.document.querySelectorAll("iframe").length,
  }), platform).catch(() => null);
  let document;
  try {
    document = await Promise.race([capture, new Promise(resolve => { timer = setTimeout(() => resolve(null), timeoutMs); })]);
  } finally {
    clearTimeout(timer);
  }
  const diagnostic = {
    schema_version: 1,
    platform,
    stage: error.discovery_stage || stage,
    document,
    captured_at: new Date().toISOString(),
  };
  error.discovery_diagnostic = diagnostic;
  const directory = path.join(root, "diagnostics", runId);
  await fs.mkdir(directory, { recursive: true });
  await fs.writeFile(path.join(directory, `${platform}.json`), `${JSON.stringify(diagnostic, null, 2)}\n`, {
    mode: 0o600,
  });
  return diagnostic;
}
