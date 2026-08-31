import { useCallback, useEffect, useMemo, useState } from "react";
import { Download, ExternalLink, Loader2, LockKeyhole, RotateCcw, ShieldCheck } from "lucide-react";

import type { NginxUiStatus, WorkspaceUpdateMode } from "../types/api";
import type { GitRemoteSetupPanelProps } from "./GitRemoteSetupPanel";
import { useUiDialog } from "./UiDialogProvider";

type Translate = (zh: string, en: string) => string;
type ClientPlatform = "windows" | "macos" | "linux" | "ios" | "android" | "chromeos";
type ClientBrowser = "chrome" | "edge" | "firefox" | "safari";

interface LocalHttpsSetupPanelProps {
  t: Translate;
  apiFetch: GitRemoteSetupPanelProps["apiFetch"];
  status: NginxUiStatus | null;
  busy: boolean;
  activeMode?: WorkspaceUpdateMode | string;
  onStart: (mode: WorkspaceUpdateMode) => unknown | Promise<unknown>;
}

const PLATFORM_OPTIONS: Array<{ value: ClientPlatform; zh: string; en: string }> = [
  { value: "windows", zh: "Windows", en: "Windows" },
  { value: "macos", zh: "macOS", en: "macOS" },
  { value: "linux", zh: "Ubuntu / Linux", en: "Ubuntu / Linux" },
  { value: "ios", zh: "iPhone / iPad", en: "iPhone / iPad" },
  { value: "android", zh: "Android", en: "Android" },
  { value: "chromeos", zh: "ChromeOS", en: "ChromeOS" },
];

const BROWSER_OPTIONS: Array<{ value: ClientBrowser; label: string }> = [
  { value: "chrome", label: "Chrome" },
  { value: "edge", label: "Edge" },
  { value: "firefox", label: "Firefox" },
  { value: "safari", label: "Safari" },
];

function browserOptionsForPlatform(platform: ClientPlatform) {
  const allowed: Record<ClientPlatform, ClientBrowser[]> = {
    windows: ["chrome", "edge", "firefox"],
    macos: ["safari", "chrome", "edge", "firefox"],
    linux: ["chrome", "edge", "firefox"],
    ios: ["safari", "chrome", "edge", "firefox"],
    android: ["chrome", "edge", "firefox"],
    chromeos: ["chrome"],
  };
  return BROWSER_OPTIONS.filter((option) => allowed[platform].includes(option.value));
}

export function resolveClientPlatform(userAgent: string, platform: string, maxTouchPoints = 0): ClientPlatform {
  if (/Android/i.test(userAgent)) return "android";
  if (/CrOS/i.test(userAgent)) return "chromeos";
  if (/iPhone|iPad|iPod/i.test(userAgent) || (platform === "MacIntel" && maxTouchPoints > 1)) return "ios";
  if (/Win/i.test(platform)) return "windows";
  if (/Mac/i.test(platform)) return "macos";
  return "linux";
}

export function resolveClientBrowser(userAgent: string): ClientBrowser {
  if (/Firefox\//i.test(userAgent)) return "firefox";
  if (/Edg\//i.test(userAgent)) return "edge";
  if (/Safari\//i.test(userAgent) && !/Chrome\//i.test(userAgent)) return "safari";
  return "chrome";
}

function detectClientPlatform(): ClientPlatform {
  return resolveClientPlatform(navigator.userAgent, navigator.platform, navigator.maxTouchPoints);
}

function detectClientBrowser(): ClientBrowser {
  return resolveClientBrowser(navigator.userAgent);
}

export function certificateInstallInstructions(
  platform: ClientPlatform,
  browser: ClientBrowser,
  t: Translate,
): string[] {
  const browserName = BROWSER_OPTIONS.find((option) => option.value === browser)?.label ?? "Chrome";
  if (browser === "firefox" && platform !== "ios" && platform !== "android") {
    return [
      t("打开 Firefox 右上角菜单，选择“设置”，再进入“隐私与安全”。", "Open the Firefox menu, choose Settings, then open Privacy & Security."),
      t("向下找到“证书”，点击“查看证书”。", "Scroll to Certificates and select View Certificates."),
      t("切换到“证书颁发机构”页签，点击“导入”，选择刚下载的 CA 文件。", "Open the Authorities tab, select Import, and choose the downloaded CA file."),
      t("勾选“信任此 CA 以标识网站”，确认导入。不要导入到“您的证书”页签。", "Enable “Trust this CA to identify websites” and confirm. Do not import it under Your Certificates."),
      t("关闭所有 Firefox 窗口后重新打开，再访问设备的 HTTPS 地址。", "Close every Firefox window, reopen Firefox, then visit the device HTTPS address."),
    ];
  }
  switch (platform) {
    case "windows":
      return [
        browser === "edge"
          ? t("在 Edge 中打开“设置 → 隐私、搜索和服务 → 安全性 → 管理证书”。", "In Edge, open Settings → Privacy, search, and services → Security → Manage certificates.")
          : t("在 Chrome 中打开“设置 → 隐私和安全 → 安全 → 管理证书”。", "In Chrome, open Settings → Privacy and security → Security → Manage certificates."),
        t("在 Windows 证书窗口中选择“受信任的根证书颁发机构”，点击“导入”。也可以双击下载的 CA 文件后选择“安装证书”。", "In the Windows certificate window, select Trusted Root Certification Authorities and choose Import. You may also double-click the downloaded CA file and choose Install Certificate."),
        t("选择“当前用户”，将证书明确放入“受信任的根证书颁发机构”，不要使用自动选择。", "Choose Current User and explicitly place the certificate in Trusted Root Certification Authorities instead of using automatic selection."),
        t("完成向导并确认安全警告，然后关闭证书窗口。", "Finish the wizard, confirm the security warning, and close the certificate window."),
        t(`关闭所有 ${browserName} 窗口后重新打开，再访问设备的 HTTPS 地址。`, `Close every ${browserName} window, reopen it, then visit the device HTTPS address.`),
      ];
    case "macos":
      return [
        t("打开“钥匙串访问”；在左侧选择“系统”钥匙串，再选择“证书”分类。", "Open Keychain Access. Select the System keychain on the left, then select the Certificates category."),
        t("把下载的 CA 文件拖入“钥匙串访问”，或使用“文件 → 导入项目”，按提示输入管理员密码。", "Drag the downloaded CA file into Keychain Access, or use File → Import Items, then enter an administrator password when prompted."),
        t("找到刚导入的证书并双击，展开“信任”，把“使用此证书时”设为“始终信任”。", "Find and double-click the imported certificate, expand Trust, and set When using this certificate to Always Trust."),
        t("关闭证书窗口并再次确认管理员密码；证书状态应显示为受信任。", "Close the certificate window and confirm the administrator password again. The certificate should now show as trusted."),
        t(`彻底退出 ${browserName} 后重新打开，再访问设备的 HTTPS 地址。`, `Fully quit ${browserName}, reopen it, then visit the device HTTPS address.`),
      ];
    case "linux":
      return [
        browser === "edge"
          ? t("打开 Edge 右上角菜单，选择“设置 → 隐私、搜索和服务 → 安全性 → 管理证书”。", "Open the Edge menu, then Settings → Privacy, search, and services → Security → Manage certificates.")
          : t("打开 Chrome 右上角菜单，选择“设置 → 隐私和安全 → 安全 → 管理证书”。", "Open the Chrome menu, then Settings → Privacy and security → Security → Manage certificates."),
        t("在证书管理器中打开“证书颁发机构”或“授权机构”页签，然后点击“导入”。", "In the certificate manager, open Authorities or Certificate Authorities, then select Import."),
        t("选择刚下载的 CA 文件；出现用途选项时，允许该 CA 标识网站。", "Choose the downloaded CA file. If trust purposes are shown, allow this CA to identify websites."),
        t("确认导入后，在证书颁发机构列表中检查该设备 CA 已出现。", "After importing, verify that the device CA appears in the authorities list."),
        t(`关闭所有 ${browserName} 窗口后重新打开，再访问设备的 HTTPS 地址。`, `Close every ${browserName} window, reopen it, then visit the device HTTPS address.`),
      ];
    case "ios":
      return [
        t(`用 ${browserName} 下载证书后，打开系统“设置”；点击顶部的“已下载描述文件”。如果没有该入口，进入“通用 → VPN 与设备管理”。`, `After downloading the certificate in ${browserName}, open Settings and select Profile Downloaded. If it is not shown, open General → VPN & Device Management.`),
        t("选择设备 CA，点击“安装”，按提示输入设备密码并再次确认安装。", "Select the device CA, tap Install, enter the device passcode, and confirm installation again."),
        t("进入“设置 → 通用 → 关于本机 → 证书信任设置”。", "Go to Settings → General → About → Certificate Trust Settings."),
        t("在“针对根证书启用完全信任”下打开该设备 CA，并确认警告。", "Under Enable Full Trust for Root Certificates, enable the device CA and confirm the warning."),
        t(`彻底关闭并重新打开 ${browserName}，再访问设备的 HTTPS 地址。`, `Fully close and reopen ${browserName}, then visit the device HTTPS address.`),
      ];
    case "android":
      return [
        t(`先用 ${browserName} 下载 CA 文件，再打开系统“设置 → 安全与隐私 → 更多安全设置”。不同品牌的名称可能略有不同。`, `Download the CA file in ${browserName}, then open Settings → Security and privacy → More security settings. Labels vary by device maker.`),
        t("选择“加密与凭据”或“安装证书”，再选择“CA 证书”。", "Choose Encryption & credentials or Install a certificate, then select CA certificate."),
        t("确认系统安全提示，选择下载的 CA 文件，并使用锁屏密码完成安装。", "Confirm the security warning, select the downloaded CA file, and complete installation with the screen-lock credential."),
        t("回到“受信任的凭据 → 用户”检查设备 CA 已出现。", "Open Trusted credentials → User and verify that the device CA appears."),
        t(`强制关闭并重新打开 ${browserName}，再访问设备的 HTTPS 地址。`, `Force-close and reopen ${browserName}, then visit the device HTTPS address.`),
      ];
    case "chromeos":
      return [
        t("打开“设置 → 隐私和安全 → 安全 → 管理证书”。", "Open Settings → Privacy and security → Security → Manage certificates."),
        t("打开“授权机构”，点击“导入”，选择下载的 CA 文件。", "Open Authorities, select Import, and choose the downloaded CA file."),
        t("启用“信任此证书以标识网站”，然后确认导入。", "Enable Trust this certificate for identifying websites, then confirm the import."),
        t("关闭所有 Chrome 窗口后重新打开，再访问设备的 HTTPS 地址。", "Close every Chrome window, reopen Chrome, then visit the device HTTPS address."),
      ];
  }
}

export function buildSecureDeviceUrl(currentUrl: string): string {
  const url = new URL(currentUrl);
  url.protocol = "https:";
  url.port = "";
  url.pathname = "/";
  url.search = "";
  url.hash = "";
  return url.toString();
}

export function LocalHttpsSetupPanel({
  t,
  apiFetch,
  status,
  busy,
  activeMode,
  onStart,
}: LocalHttpsSetupPanelProps) {
  const { confirm } = useUiDialog();
  const [platform, setPlatform] = useState<ClientPlatform>(() => detectClientPlatform());
  const [browser, setBrowser] = useState<ClientBrowser>(() => detectClientBrowser());
  const [trusted, setTrusted] = useState(false);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);
  const [secureFlowActive, setSecureFlowActive] = useState(false);
  const [showInstructions, setShowInstructions] = useState(false);
  const fingerprint = status?.local_https_ca_fingerprint_sha256 || "";
  const browserOptions = useMemo(() => browserOptionsForPlatform(platform), [platform]);
  const instructions = useMemo(
    () => certificateInstallInstructions(platform, browser, t),
    [platform, browser, t],
  );
  const usingHttps = window.location.protocol === "https:";

  useEffect(() => setTrusted(false), [fingerprint]);
  useEffect(() => {
    if (!browserOptions.some((option) => option.value === browser)) {
      setBrowser(browserOptions[0]?.value ?? "chrome");
    }
  }, [browser, browserOptions]);

  const downloadCertificate = useCallback(async () => {
    setDownloading(true);
    setDownloadError(null);
    try {
      const response = await apiFetch("/v1/system/local-https-ca");
      if (!response.ok) throw new Error(`local_https_certificate_http_${response.status}`);
      const blob = await response.blob();
      const href = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = href;
      link.download = "local-device-ca.crt";
      document.body.appendChild(link);
      link.click();
      link.remove();
      window.setTimeout(() => URL.revokeObjectURL(href), 1_000);
    } catch {
      setDownloadError(t("证书下载失败，请检查连接后重试。", "Certificate download failed. Check the connection and try again."));
    } finally {
      setDownloading(false);
    }
  }, [apiFetch, t]);

  const beginSecureAccess = async () => {
    const accepted = await confirm({
      title: t("开启局域网安全访问", "Enable secure LAN access"),
      message: status?.local_https_enabled
        ? t(
            "如果只在本机通过 localhost 或 127.0.0.1 访问，不需要此功能。它用于通过局域网 IP 访问时加密传输。继续后只会展开配置步骤，不会立即下载证书或切换页面；现有 HTTP 地址仍会保留。是否继续？",
            "You do not need this when accessing only from this device through localhost or 127.0.0.1. It encrypts access through a LAN IP. Continuing only opens the setup steps; it will not immediately download a certificate or switch this page. The existing HTTP address remains available. Continue?",
          )
        : t(
            "如果只在本机通过 localhost 或 127.0.0.1 访问，不需要此功能。它用于通过局域网 IP 访问时加密传输。继续后只会展开配置步骤，不会立即启用 HTTPS。请依次准备证书、安装并信任证书，最后再激活 HTTPS；现有 HTTP 地址仍会保留。是否继续？",
            "You do not need this when accessing only from this device through localhost or 127.0.0.1. It encrypts access through a LAN IP. Continuing only opens the setup steps and does not enable HTTPS. Prepare the certificate, install and trust it, then activate HTTPS as the final step. The existing HTTP address remains available. Continue?",
          ),
      confirmLabel: t("查看配置步骤", "Show setup steps"),
    });
    if (!accepted) return;

    setSecureFlowActive(true);
    setShowInstructions(true);
  };

  if (status?.local_https_supported === false) {
    return (
      <section className="rounded-lg border border-emerald-300/20 bg-emerald-300/[0.05] p-4 sm:p-5" aria-labelledby="local-https-heading">
        <div className="flex items-start gap-3">
          <span className="rounded-lg bg-emerald-300/10 p-2 text-emerald-100">
            <LockKeyhole className="h-5 w-5" />
          </span>
          <div>
            <h4 id="local-https-heading" className="text-sm font-semibold text-white">
              {t("局域网 HTTPS（可选）", "LAN HTTPS (optional)")}
            </h4>
            <p className="mt-2 text-sm leading-6 text-white/55">
              {t("当前服务器系统暂不支持自动配置局域网 HTTPS。", "Automatic LAN HTTPS setup is not supported on this server platform.")}
            </p>
          </div>
        </div>
      </section>
    );
  }

  return (
    <section className="rounded-lg border border-emerald-300/20 bg-emerald-300/[0.05] p-4 sm:p-5" aria-labelledby="local-https-heading">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-3">
          <span className="rounded-lg bg-emerald-300/10 p-2 text-emerald-100">
            <LockKeyhole className="h-5 w-5" />
          </span>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h4 id="local-https-heading" className="text-sm font-semibold text-white">
                {t("局域网 HTTPS（可选）", "LAN HTTPS (optional)")}
              </h4>
              <span className={`rounded-full px-2 py-0.5 text-[11px] ${usingHttps ? "bg-emerald-400/10 text-emerald-200" : "bg-white/8 text-white/60"}`}>
                {usingHttps ? t("当前已安全访问", "Secure connection active") : t("当前使用 HTTP", "Using HTTP")}
              </span>
            </div>
            <p className="mt-2 max-w-3xl text-sm leading-6 text-white/60">
              {t(
                "本机通过 localhost 或 127.0.0.1 访问时不需要开启。通过局域网 IP 访问并希望加密传输时，可安装设备 CA 后启用 HTTPS；HTTP 默认保留。",
                "You do not need this for localhost or 127.0.0.1 access. For encrypted access through a LAN IP, install the device CA and enable HTTPS. HTTP remains available by default.",
              )}
            </p>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          {!usingHttps ? (
            <button
              type="button"
              className="theme-accent-btn min-h-10 rounded-md border border-white/35 px-3 py-2 text-sm"
              disabled={busy || downloading}
              onClick={() => void beginSecureAccess()}
            >
              {busy && activeMode === "local_https_enable" || downloading ? <Loader2 className="h-4 w-4 animate-spin" /> : <LockKeyhole className="h-4 w-4" />}
              {status?.local_https_enabled
                ? t("查看安全访问配置", "View secure access setup")
                : t("配置安全访问", "Set up secure access")}
            </button>
          ) : null}
          <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" onClick={() => setShowInstructions((value) => !value)}>
            {showInstructions ? t("收起安装帮助", "Hide setup help") : t("查看安装帮助", "Show setup help")}
          </button>
        </div>
      </div>

      {secureFlowActive && !usingHttps ? (
        <p className="mt-3 text-sm leading-6 text-emerald-100/80">
          {status?.local_https_enabled
            ? t("HTTPS 已激活。完成证书信任后，请在第 3 步点击切换到 HTTPS。", "HTTPS is active. After trusting the certificate, use step 3 to switch to HTTPS.")
            : t("请按下方顺序完成配置；在第 3 步确认前不会启用 HTTPS。", "Complete the steps below in order. HTTPS will not be enabled before your confirmation in step 3.")}
        </p>
      ) : null}

      {showInstructions ? <div className="mt-4 grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(280px,0.8fr)]">
        <div className="space-y-4">
          <div className="flex items-start gap-3">
            <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-white/10 text-xs font-semibold text-white">1</span>
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-white">{t("准备并下载设备 CA", "Prepare and download the device CA")}</p>
              <div className="mt-2 flex flex-wrap gap-2">
                {!status?.local_https_prepared ? (
                  <button
                    type="button"
                    className="theme-secondary-btn px-3 py-2 text-sm"
                    disabled={busy}
                    onClick={() => void onStart("local_https_prepare")}
                  >
                    {busy && activeMode === "local_https_prepare" ? <Loader2 className="h-4 w-4 animate-spin" /> : <ShieldCheck className="h-4 w-4" />}
                    {t("准备证书", "Prepare certificate")}
                  </button>
                ) : (
                  <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={downloading} onClick={() => void downloadCertificate()}>
                    {downloading ? <Loader2 className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
                    {t("下载 CA 证书", "Download CA certificate")}
                  </button>
                )}
              </div>
              {fingerprint ? (
                <p className="mt-2 break-all font-mono text-[11px] leading-5 text-white/45">
                  SHA-256: {fingerprint}
                </p>
              ) : null}
              {downloadError ? <p className="mt-2 text-sm text-red-200">{downloadError}</p> : null}
            </div>
          </div>

          <div className="flex items-start gap-3">
            <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-white/10 text-xs font-semibold text-white">2</span>
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-white">{t("在当前系统中安装并信任", "Install and trust on this client")}</p>
              <div className="mt-2 grid gap-2 sm:grid-cols-2">
                <label className="space-y-1 text-xs text-white/50">
                  <span>{t("系统", "System")}</span>
                  <select className="theme-input" value={platform} onChange={(event) => setPlatform(event.target.value as ClientPlatform)}>
                    {PLATFORM_OPTIONS.map((option) => <option key={option.value} value={option.value}>{t(option.zh, option.en)}</option>)}
                  </select>
                </label>
                <label className="space-y-1 text-xs text-white/50">
                  <span>{t("浏览器", "Browser")}</span>
                  <select className="theme-input" value={browser} onChange={(event) => setBrowser(event.target.value as ClientBrowser)}>
                    {browserOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                  </select>
                </label>
              </div>
              <ol className="mt-3 space-y-2 text-sm leading-6 text-white/65">
                {instructions.map((instruction, index) => <li key={instruction}>{index + 1}. {instruction}</li>)}
              </ol>
            </div>
          </div>

          <div className="flex items-start gap-3">
            <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-white/10 text-xs font-semibold text-white">3</span>
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-white">{t("确认后启用 HTTPS", "Confirm and enable HTTPS")}</p>
              <label className="mt-3 flex cursor-pointer items-start gap-2 text-sm leading-5 text-white/70">
                <input type="checkbox" className="mt-0.5 h-4 w-4" checked={trusted} onChange={(event) => setTrusted(event.target.checked)} />
                <span>{t("我已安装当前指纹对应的 CA，并重新启动浏览器。", "I installed the CA matching this fingerprint and restarted the browser.")}</span>
              </label>
              <div className="mt-3 flex flex-wrap gap-2">
                {!status?.local_https_enabled ? (
                  <button
                    type="button"
                    className="theme-primary-btn min-h-10 rounded-md border border-white/35 px-3 py-2 text-sm"
                    disabled={!status?.local_https_prepared || !trusted || busy}
                    onClick={() => void onStart("local_https_enable")}
                  >
                    {busy && activeMode === "local_https_enable" ? <Loader2 className="h-4 w-4 animate-spin" /> : <LockKeyhole className="h-4 w-4" />}
                    {t("激活 HTTPS", "Activate HTTPS")}
                  </button>
                ) : (
                  <button type="button" className="theme-primary-btn px-3 py-2 text-sm" onClick={() => window.location.assign(buildSecureDeviceUrl(window.location.href))}>
                    <ExternalLink className="h-4 w-4" />
                    {t("切换到 HTTPS", "Switch to HTTPS")}
                  </button>
                )}
                {status?.local_https_enabled ? (
                  <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={busy} onClick={() => void onStart("local_https_restore")}>
                    {busy && activeMode === "local_https_restore" ? <Loader2 className="h-4 w-4 animate-spin" /> : <RotateCcw className="h-4 w-4" />}
                    {t("停用 HTTPS", "Disable HTTPS")}
                  </button>
                ) : null}
              </div>
            </div>
          </div>
        </div>

        <div className="self-start border-l border-white/10 pl-4 text-sm leading-6 text-white/60">
          <p className="font-medium text-white">{t("开始前确认", "Before you begin")}</p>
          <ul className="mt-2 space-y-2">
            <li>{t("只安装 CA 公钥证书；浏览器不会收到设备 CA 私钥。", "Only the public CA certificate is installed; the browser never receives the device CA private key.")}</li>
            <li>{t("每台电脑、手机及独立证书库都需要分别信任。", "Each computer, phone, and separate certificate store must trust it individually.")}</li>
            <li>{t("设备局域网 IP 改变时，服务器证书会更新，但同一设备 CA 可以继续使用。", "When the LAN IP changes, the server certificate is renewed while the same device CA remains valid.")}</li>
          </ul>
        </div>
      </div> : null}
    </section>
  );
}
