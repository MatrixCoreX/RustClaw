import type { ReactNode } from "react";
import { KeyRound, Loader2 } from "lucide-react";

type UiLanguage = "zh" | "en";
type LoginTab = "key" | "webd";
type Translate = (zh: string, en: string) => string;

export interface SignInPageProps {
  t: Translate;
  lang: UiLanguage;
  loginTab: LoginTab;
  baseUrl: string;
  uiKey: string;
  uiKeyDraft: string;
  maskedSavedUiKey: string;
  webdBaseUrlDraft: string;
  webdUsername: string;
  webdPassword: string;
  uiAuthLoading: boolean;
  uiAuthError: string | null;
  factoryResetModal: ReactNode;
  onBaseUrlChange: (value: string) => void;
  onUiKeyDraftChange: (value: string) => void;
  onWebdBaseUrlDraftChange: (value: string) => void;
  onWebdUsernameChange: (value: string) => void;
  onWebdPasswordChange: (value: string) => void;
  onVerifyUiKey: (key: string) => unknown | Promise<unknown>;
  onLoginWebd: () => unknown | Promise<unknown>;
  onSwitchLoginTab: (tab: LoginTab) => void;
  onToggleLanguage: () => void;
}

export function SignInPage({
  t,
  lang,
  loginTab,
  baseUrl,
  uiKey,
  uiKeyDraft,
  maskedSavedUiKey,
  webdBaseUrlDraft,
  webdUsername,
  webdPassword,
  uiAuthLoading,
  uiAuthError,
  factoryResetModal,
  onBaseUrlChange,
  onUiKeyDraftChange,
  onWebdBaseUrlDraftChange,
  onWebdUsernameChange,
  onWebdPasswordChange,
  onVerifyUiKey,
  onLoginWebd,
  onSwitchLoginTab,
  onToggleLanguage,
}: SignInPageProps) {
  return (
    <>
      <div className="theme-shell min-h-screen px-4 py-8">
        <div className="mx-auto grid max-w-5xl gap-6 lg:grid-cols-[1.1fr_0.9fr]">
          <div className="theme-panel p-6 sm:p-8">
            <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">{t("欢迎", "Welcome")}</p>
            <h1 className="mt-4 flex items-center gap-2 text-2xl font-bold sm:text-3xl">
              <img className="rustclaw-logo rustclaw-logo-hero" src="/rustclaw-logo.svg" alt="" />
              <span>{t("进入 RustClaw 控制台", "Enter RustClaw Console")}</span>
            </h1>
            <p className="mt-4 max-w-xl text-sm leading-7 text-white/70 sm:text-base">
              {t(
                "这是给普通用户准备的可视化面板。你不需要先懂命令行，只要填好服务地址、用户名和密码，就能查看状态、绑定账号、测试消息。",
                "This is a visual panel designed for everyday users. You do not need the command line first; enter the service address, username, and password to check status, bind accounts, and test messages.",
              )}
            </p>

            <div className="mt-6 rounded-2xl border border-white/10 bg-black/20 p-4">
              <p className="text-sm font-semibold text-white">{t("登录前你需要什么？", "What do you need before signing in?")}</p>
              <ol className="mt-3 list-decimal space-y-2 pl-5 text-sm text-white/65">
                <li>{t("一个已经启动的 RustClaw 服务地址。", "A running RustClaw service address.")}</li>
                <li>{t("你的网页登录用户名和密码。", "Your web login username and password.")}</li>
                <li>{t("如果不知道接下来该做什么，登录后先看首页。", "If you are not sure what to do next, start with Home after signing in.")}</li>
              </ol>
            </div>
          </div>

          <div className="theme-panel p-6">
            <div className="mb-6">
              <h2 className="text-xl font-bold">{t("登录", "Sign in")}</h2>
              <p className="mt-2 text-sm text-white/60">
                {loginTab === "key"
                  ? t("使用 Access Key 验证后进入控制台。", "Verify with an access key to enter the console.")
                  : t("默认使用用户名和密码登录。", "Username and password sign-in is the default.")}
              </p>
            </div>

            <div className="space-y-4">
              {loginTab === "key" ? (
                <>
                  <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-2 text-xs leading-relaxed text-white/55">
                    {t(
                      "Key 登录适合管理员或已经拿到 user_key 的用户。普通用户建议返回用户名密码登录。",
                      "Access key sign-in is for admins or users who already have a user_key. Most users should use username and password sign-in.",
                    )}
                  </div>
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">
                      {t("RustClaw 服务地址", "RustClaw service URL")}
                    </span>
                    <input
                      className="theme-input"
                      value={baseUrl}
                      onChange={(event) => onBaseUrlChange(event.target.value)}
                      placeholder="http://127.0.0.1:8787"
                    />
                    <p className="text-xs text-white/45">
                      {t("直连 clawd 或经 webd 代理时均可；请与浏览器能访问到的 API 地址一致。", "Use the API URL your browser can reach (direct clawd or via webd).")}
                    </p>
                  </label>

                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">{t("访问 Key", "Access Key")}</span>
                    <input
                      className="theme-input"
                      value={uiKeyDraft}
                      onChange={(event) => onUiKeyDraftChange(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          void onVerifyUiKey(uiKeyDraft);
                        }
                      }}
                      placeholder={t("输入已经生成好的 user_key", "Enter an existing user_key")}
                    />
                    <p className="text-xs text-white/45">
                      {t("如果你不知道这个 key，通常需要找部署 RustClaw 的人帮你生成。", "If you do not know this key, it usually needs to be generated by whoever set up RustClaw.")}
                    </p>
                  </label>

                  {maskedSavedUiKey ? (
                    <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-2 text-xs text-white/70">
                      <div>{t("已保存 Key", "Saved key")}: {maskedSavedUiKey}</div>
                      <div className="mt-1 text-white/45">
                        {t("输入新 key 会覆盖已保存的 key。", "Entering a new key will replace the saved key.")}
                      </div>
                    </div>
                  ) : null}
                </>
              ) : (
                <>
                  <p className="text-xs leading-relaxed text-white/55">
                    {t(
                      "可填写 webd 地址端口（例如 http://127.0.0.1:8788）；留空则默认走当前页面地址（常见于 nginx 反代）。",
                      "You can enter a webd URL/port (for example http://127.0.0.1:8788); if left empty, current page origin is used (common with nginx reverse proxy).",
                    )}
                  </p>
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">
                      {t("Webd 地址（可选）", "Webd URL (optional)")}
                    </span>
                    <input
                      className="theme-input"
                      value={webdBaseUrlDraft}
                      onChange={(event) => onWebdBaseUrlDraftChange(event.target.value)}
                      placeholder="http://127.0.0.1:8788"
                    />
                  </label>
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">{t("用户名", "Username")}</span>
                    <input
                      className="theme-input"
                      autoComplete="username"
                      value={webdUsername}
                      onChange={(event) => onWebdUsernameChange(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          void onLoginWebd();
                        }
                      }}
                    />
                  </label>
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-widest text-white/50">{t("密码", "Password")}</span>
                    <input
                      className="theme-input"
                      type="password"
                      autoComplete="current-password"
                      value={webdPassword}
                      onChange={(event) => onWebdPasswordChange(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          void onLoginWebd();
                        }
                      }}
                    />
                  </label>
                </>
              )}

              {uiAuthError ? (
                <p className="rounded-xl border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                  {uiAuthError}
                </p>
              ) : null}

              <div className="flex flex-wrap items-center gap-3">
                {loginTab === "key" ? (
                  <>
                    <button
                      type="button"
                      onClick={() => void onVerifyUiKey(uiKeyDraft)}
                      disabled={uiAuthLoading}
                      className="theme-accent-btn"
                    >
                      {uiAuthLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                      {t("进入控制台", "Enter Console")}
                    </button>
                    {uiKey ? (
                      <button
                        type="button"
                        onClick={() => void onVerifyUiKey(uiKey)}
                        disabled={uiAuthLoading}
                        className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-4 py-2 text-sm font-medium text-white transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-60"
                      >
                        {t("使用已保存 Key", "Use saved key")}
                      </button>
                    ) : null}
                    <button
                      type="button"
                      onClick={() => onSwitchLoginTab("webd")}
                      disabled={uiAuthLoading}
                      className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-4 py-2 text-sm font-medium text-white transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-60"
                    >
                      {t("返回用户名密码登录", "Back to username sign-in")}
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      onClick={() => void onLoginWebd()}
                      disabled={uiAuthLoading}
                      className="theme-accent-btn"
                    >
                      {uiAuthLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                      {t("进入控制台", "Enter Console")}
                    </button>
                    <button
                      type="button"
                      onClick={() => onSwitchLoginTab("key")}
                      disabled={uiAuthLoading}
                      className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-4 py-2 text-sm font-medium text-white transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-60"
                    >
                      <KeyRound className="h-4 w-4" />
                      {t("使用 Key 登录", "Use access key")}
                    </button>
                  </>
                )}
                <button
                  type="button"
                  onClick={onToggleLanguage}
                  className="rounded-xl border border-white/15 bg-white/5 px-3 py-2 text-xs hover:bg-white/10"
                >
                  {lang === "zh" ? "中文" : "EN"}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
      {factoryResetModal}
    </>
  );
}
