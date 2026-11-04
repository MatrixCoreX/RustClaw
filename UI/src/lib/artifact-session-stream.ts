import { safeArtifactUrl } from "./task-artifacts";
import type { BrowserAuthMode } from "./webd-csrf";

export type ArtifactFetch = ((path: string, init?: RequestInit) => Promise<Response>) & {
  nativeMediaUrl?: (path: string) => string | null;
};

// Native media cannot carry an API key. Reuse cookies only on the exact UI origin;
// remote connections, key login and transformed previews retain authenticated fetch.
export function withSessionArtifactStreaming(fetcher: ArtifactFetch, options: {
  authMode: BrowserAuthMode;
  apiBase: string;
  pageUrl: string;
  isCurrent: () => boolean;
}): ArtifactFetch {
  return Object.assign(fetcher, {
    nativeMediaUrl(path: string): string | null {
      if (options.authMode !== "webd" || !options.isCurrent() || !safeArtifactUrl(path)) return null;
      try {
        const page = new URL(options.pageUrl);
        const base = new URL(options.apiBase || "/", page);
        if (!/^https?:$/.test(page.protocol) || base.origin !== page.origin
          || base.username || base.password || base.search || base.hash) return null;
        const artifact = new URL(path, page);
        if (artifact.hash || [...artifact.searchParams].some(([key, value]) => key !== "disposition" || value !== "inline")) return null;
        return new URL(`${base.href.replace(/\/$/, "")}${path}`).href;
      } catch {
        return null;
      }
    },
  });
}
