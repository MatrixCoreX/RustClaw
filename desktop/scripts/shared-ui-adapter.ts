import { adaptWalletUi } from "./wallet-ui-adapter";
import path from 'node:path';
import { normalizePath, type Plugin } from '../../UI/node_modules/vite/dist/node/index.js';

/** Build-time adaptation only. Repository UI sources remain untouched and are never copied. */
export function sharedUiAdapter(root: string): Plugin {
  const uiRoot = normalizePath(path.resolve(root, '../UI/src')) + '/';
  const runtime = normalizePath(path.resolve(root, 'frontend/runtime.ts'));
  const components = normalizePath(path.resolve(root, 'frontend/SharedAdapters.tsx'));
  const once = (source: string, before: string, after: string, id: string) => {
    if (source.split(before).length !== 2) throw new Error(`Shared UI contract changed: ${id}: ${before}`);
    return source.replace(before, after);
  };
  return {
    name: 'desktop-shared-ui-contract', enforce: 'pre',
    transform(source, id) {
      id = normalizePath(id);
      source = source.replaceAll('\r\n', '\n');
      if (id === uiRoot + 'index.css') {
        return {code: source.replace(/^@import url\('https:\/\/fonts\.googleapis\.com[^\n]+\n/m, ''), map: null};
      }
      if (!id.startsWith(uiRoot) || !/\.[jt]sx?$/.test(id)) return;
      let text = source;
      const imports = new Set<string>();
      for (const [original, replacement] of [['window.localStorage', 'desktopStorage'], ['window.sessionStorage', 'desktopSessionStorage']]) {
        if (text.includes(original)) { text = text.replaceAll(original, replacement); imports.add(replacement); }
      }
      if (id === uiRoot + 'App.tsx') {
        const walletPages = normalizePath(path.resolve(root, 'frontend/wallet/pages.tsx'));
        text = once(text, 'from "./components/AssetsPage"', `from ${JSON.stringify(walletPages)}`, id);
        text = once(text, 'from "./components/BancorPage"', `from ${JSON.stringify(walletPages)}`, id);
        text = once(text, 'onRefresh={() => Promise.allSettled([\n                assetOverviewRuntime.fetchMarket(),', 'onRefreshMarket={() => assetOverviewRuntime.fetchMarket()}\n              onRefresh={() => Promise.allSettled([\n                assetOverviewRuntime.fetchMarket(),', id);
        if ((text.match(/\bfetch\(/g) ?? []).length !== 6) throw new Error('Shared UI fetch inventory changed');
        imports.add('desktopFetch as fetch'); imports.add('desktopOrigin'); imports.add('desktopIdentity'); imports.add('desktopLogout');
        text = once(text, 'preferredBrowserApiBaseUrl(saved, window.location)', 'desktopOrigin', id);
        text = once(text, 'preferredWebdBaseUrl(saved, window.location)', 'desktopOrigin', id);
        text = once(text, 'useState<"key" | "webd" | null>(readPersistedAuthMode)', 'useState<"key" | "webd" | null>(desktopIdentity ? "webd" : null)', id);
        text = once(text, 'const [uiAuthReady, setUiAuthReady] = useState(false)', 'const [uiAuthReady, setUiAuthReady] = useState(Boolean(desktopIdentity))', id);
        text = once(text, 'useState<AuthIdentityResponse | null>(null)', 'useState<AuthIdentityResponse | null>(desktopIdentity)', id);
        text = once(text, 'const logout = async () => {', 'const logout = async () => { await desktopLogout(); return;', id);
        text = once(text, 'const [interactionUserId, setInteractionUserId] = useState<number | null>(null)', 'const [interactionUserId, setInteractionUserId] = useState<number | null>(desktopIdentity?.user_id ?? null)', id);
        text = once(text, 'const [interactionChatId, setInteractionChatId] = useState<number | null>(null)', 'const [interactionChatId, setInteractionChatId] = useState<number | null>(desktopIdentity?.chat_id ?? null)', id);
        text = once(text, 'const [interactionRole, setInteractionRole] = useState<string>("-")', 'const [interactionRole, setInteractionRole] = useState<string>(desktopIdentity?.role ?? "-")', id);
        // Expired sessions return to native login rather than the browser's direct-key form.
        text = once(text, 'setUiAuthError(\n          t("登录状态已失效，请重新登录。"', 'void desktopLogout();\n        setUiAuthError(\n          t("登录状态已失效，请重新登录。"', id);
      }
      text = adaptWalletUi(text, id, root, uiRoot);
      if (id === uiRoot + 'lib/nni-owner-public-key.ts') {
        imports.add('desktopSigningLocation');
        text = once(text, '{ protocol: window.location.protocol, hostname: window.location.hostname }', 'desktopSigningLocation()', id);
      }
      if (id === uiRoot + 'components/UiDialogProvider.tsx') {
        imports.add('desktopTargetLabel');
        text = once(text, '{active.message}', '{desktopTargetLabel() && <><strong>{browserCopy("当前设备：", "Current device: ")}{desktopTargetLabel()}</strong><br /></>}{active.message}', id);
      }
      if (id === uiRoot + 'components/ConsoleLayout.tsx') {
        text = `import { useDesktopConsoleLayout } from ${JSON.stringify(normalizePath(path.resolve(root, 'frontend/console-layout.ts')))};\n` + text;
        text = once(text, '}: ConsoleLayoutProps) {', '}: ConsoleLayoutProps) {\n  const desktopLayoutRef = useDesktopConsoleLayout();', id);
        text = once(text, '<div\n      className={', '<div\n      ref={desktopLayoutRef}\n      data-desktop-layout={currentPage === "chat" ? "chat" : "page"}\n      className={', id);
        text = once(text, '<div\n        className={`px-3', '<div\n        data-desktop-content\n        className={`px-3', id);
        text = once(text, '<aside\n', '<aside\n          data-desktop-sidebar\n', id);
        text = once(text, 'data-nav-active={active ? "true" : undefined}', 'data-desktop-page={item.id} data-nav-active={active ? "true" : undefined}', id);
      }
      if (id === uiRoot + 'components/AippPage.tsx') {
        text = once(text, '<SandboxedAipp app={selectedApp} lang={lang} apiFetch={apiFetch} />', '<DesktopAipp app={selectedApp} lang={lang} />', id);
        text = `import { DesktopAipp } from ${JSON.stringify(components)};\n` + text;
        imports.add('desktopDownload');
        text = once(text, 'const endpoint = open && artifact.preview_url ? artifact.preview_url : artifact.download_url;', 'if (!open) { await desktopDownload(artifact.download_url, artifact.filename); return; }\n      const endpoint = open && artifact.preview_url ? artifact.preview_url : artifact.download_url;', id);
      }
      if (id === uiRoot + 'components/AippImageViewer.tsx') {
        imports.add('desktopDownload');
        text = once(text, 'const response = await apiFetchRef.current(image.downloadUrl, { signal: controller.signal });\n      if (!response.ok) throw new Error(`aipp_image_download_http_${response.status}`);\n      const blob = await response.blob();\n      if (controller.signal.aborted) return;\n      saveTaskArtifactBlob(blob, image.filename);', 'await desktopDownload(image.downloadUrl, image.filename);', id);
      }
      if (id === uiRoot + 'components/ChatPage.tsx') {
        imports.add('desktopDownload'); imports.add('desktopMediaUrl'); imports.add('desktopCloseMedia');
        text = once(text, 'const blob = await fetchTaskArtifactBlob(artifactFetchRef.current, artifact.download_url);\n      saveTaskArtifactBlob(blob, artifact.filename);', 'await desktopDownload(artifact.download_url, artifact.filename);', id);
        text = once(text, 'void fetchTaskArtifactBlob(artifactFetchRef.current, mediaPreviewUrl, controller.signal)', `if (previewKind === "video" || previewKind === "audio") {
      let mediaUrl: string | undefined;
      void desktopMediaUrl(mediaPreviewUrl).then(url => {
        mediaUrl = url;
        if (controller.signal.aborted) { void desktopCloseMedia(url); return; }
        setPreviewObjectUrl(url); setPreviewState("ready");
      }).catch(() => { if (!controller.signal.aborted) setPreviewState("error"); });
      return () => { controller.abort(); if (mediaUrl) void desktopCloseMedia(mediaUrl); };
    }
    void fetchTaskArtifactBlob(artifactFetchRef.current, mediaPreviewUrl, controller.signal)`, id);
      }
      if (imports.size) text = `import { ${[...imports].join(', ')} } from ${JSON.stringify(runtime)};\n` + text;
      return text === source ? undefined : {code: text, map: null};
    },
  };
}
