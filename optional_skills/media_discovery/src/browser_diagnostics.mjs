import fs from "node:fs/promises";
import path from "node:path";

export function browserStageError(code, stage) {
  return Object.assign(new Error(code), { discovery_stage: stage });
}

export async function recordBrowserFailure(page, { root, runId, platform, stage, error }) {
  const document = await page.evaluate(() => ({
    origin: location.origin,
    pathname: location.pathname.slice(0, 256),
    ready_state: document.readyState,
    recommendation_cards: window.document.querySelectorAll("[data-aweme-id]").length,
    video_elements: window.document.querySelectorAll("video").length,
    visible_videos: Array.from(window.document.querySelectorAll("video")).filter((node) => {
      const rect = node.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    }).length,
    detail_surfaces: window.document.querySelectorAll('[data-e2e="video-detail"]').length,
    next_controls: window.document.querySelectorAll('[data-e2e="video-switch-next-arrow"]').length,
    login_inputs: window.document.querySelectorAll('input[type="password"],input[type="tel"]').length,
    iframe_count: window.document.querySelectorAll("iframe").length,
  })).catch(() => null);
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
