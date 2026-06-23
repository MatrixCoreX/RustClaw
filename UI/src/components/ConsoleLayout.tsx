import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { ChevronDown, ShieldAlert } from "lucide-react";

import type { AuthIdentityResponse, ConsolePage } from "../types/api";

type UiLanguage = "zh" | "en";
type AuthMode = "key" | "webd" | null;
type Translate = (zh: string, en: string) => string;

export interface ConsoleNavItem {
  id: ConsolePage;
  label: string;
  icon: ReactNode;
}

export interface ConsoleLayoutProps {
  t: Translate;
  lang: UiLanguage;
  authMode: AuthMode;
  authIdentity: AuthIdentityResponse | null;
  isAdminIdentity: boolean;
  currentPage: ConsolePage;
  navItems: ConsoleNavItem[];
  maskedIdentityKey: string;
  maskedSavedUiKey: string;
  factoryResetModal: ReactNode;
  children: ReactNode;
  onCurrentPageChange: (page: ConsolePage) => void;
  onToggleLanguage: () => void;
  onLogout: () => unknown | Promise<unknown>;
  onOpenFactoryReset: () => void;
}

export function ConsoleLayout({
  t,
  lang,
  authMode,
  authIdentity,
  isAdminIdentity,
  currentPage,
  navItems,
  maskedIdentityKey,
  maskedSavedUiKey,
  factoryResetModal,
  children,
  onCurrentPageChange,
  onToggleLanguage,
  onLogout,
  onOpenFactoryReset,
}: ConsoleLayoutProps) {
  const [navDropdownOpen, setNavDropdownOpen] = useState(false);
  const navDropdownRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!navDropdownOpen) return;
    const onMouseDown = (event: MouseEvent) => {
      if (navDropdownRef.current?.contains(event.target as Node)) return;
      setNavDropdownOpen(false);
    };
    document.addEventListener("mousedown", onMouseDown);
    return () => document.removeEventListener("mousedown", onMouseDown);
  }, [navDropdownOpen]);

  return (
    <div className="theme-shell min-h-screen">
      {factoryResetModal}
      <header className="theme-header sticky top-0 z-40 border-b border-white/10 px-3 sm:px-6">
        <div className="theme-header-inner mx-auto flex min-h-16 w-full max-w-7xl items-center justify-between gap-3 py-2">
          <div className="min-w-0">
            <button
              type="button"
              onClick={() => onCurrentPageChange("dashboard")}
              className="theme-brand-link inline-flex items-center gap-2 truncate text-left text-lg font-bold tracking-tight transition hover:text-white/85 sm:text-2xl"
            >
              <img className="rustclaw-logo rustclaw-logo-header" src="/rustclaw-logo.svg" alt="" />
              <span>RustClaw</span>
            </button>
          </div>

          <div className="theme-header-actions flex flex-wrap items-center justify-end gap-2">
            <div ref={navDropdownRef} className="relative flex items-center lg:hidden">
              <button
                type="button"
                onClick={() => setNavDropdownOpen((value) => !value)}
                className="theme-topbar-nav-btn"
                aria-expanded={navDropdownOpen}
                aria-haspopup="true"
              >
                <span>{t("导航", "Nav")}</span>
                <ChevronDown className={`h-4 w-4 shrink-0 transition-transform ${navDropdownOpen ? "rotate-180" : ""}`} />
              </button>
              {navDropdownOpen ? (
                <div className="absolute right-0 top-full z-50 mt-1 min-w-[200px] rounded-xl border border-white/10 bg-[var(--theme-header-bg)] py-1 shadow-lg backdrop-blur-sm">
                  {navItems.map((item) => {
                    const active = currentPage === item.id;
                    return (
                      <button
                        key={item.id}
                        type="button"
                        onClick={() => {
                          onCurrentPageChange(item.id);
                          setNavDropdownOpen(false);
                        }}
                        className={`flex w-full items-center gap-2 px-3 py-2.5 text-left text-sm transition ${
                          active ? "theme-nav-active" : "theme-nav-idle"
                        }`}
                      >
                        <span className={active ? "theme-icon-soft" : "text-white/70"}>{item.icon}</span>
                        <span>{item.label}</span>
                      </button>
                    );
                  })}
                  {isAdminIdentity ? (
                    <div className="mt-1 border-t border-white/10 pt-1">
                      <button
                        type="button"
                        onClick={() => {
                          setNavDropdownOpen(false);
                          onOpenFactoryReset();
                        }}
                        className="flex w-full items-center gap-2 px-3 py-2.5 text-left text-sm text-red-100 transition hover:bg-red-500/10"
                      >
                        <ShieldAlert className="h-4 w-4" />
                        <span>{t("恢复出厂设置", "Factory Reset")}</span>
                      </button>
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>

            <div className="theme-toolbar-shell">
              <button
                type="button"
                onClick={onToggleLanguage}
                className="theme-toolbar-segment"
                title={t("切换界面语言", "Switch interface language")}
              >
                {lang === "zh" ? "中文" : "English"}
              </button>
              <span className="theme-toolbar-divider" aria-hidden="true" />
              <button
                type="button"
                onClick={() => void onLogout()}
                className="theme-toolbar-segment theme-toolbar-segment-danger"
                title={
                  authMode === "webd"
                    ? t("退出登录并清除 Web 会话", "Log out and clear web session")
                    : t("退出登录，需重新输入 key", "Log out; key required to sign in again")
                }
              >
                {t("退出", "Log out")}
              </button>
            </div>
          </div>
        </div>
      </header>

      <div className="px-3 py-4 sm:px-6 sm:py-6 lg:pl-[236px]">
        <aside className="fixed left-0 top-16 z-30 hidden h-[calc(100vh-4rem)] w-[220px] overflow-y-auto lg:block">
          <div className="theme-sidebar-shell mx-3 mt-0 sm:mx-4">
            <div className="mb-3 px-1">
              <p className="theme-kicker text-[10px] uppercase tracking-[0.3em]">{t("导航", "Navigation")}</p>
            </div>
            <nav className="flex gap-2 overflow-x-auto pb-1 lg:block lg:space-y-2 lg:overflow-visible">
              {navItems.map((item) => {
                const active = currentPage === item.id;
                return (
                  <button
                    key={item.id}
                    type="button"
                    data-nav-active={active ? "true" : undefined}
                    onClick={(event) => {
                      onCurrentPageChange(item.id);
                      (event.currentTarget as HTMLButtonElement).blur();
                    }}
                    className={`theme-nav-item min-w-[148px] rounded-2xl border px-3 py-2.5 text-left transition lg:block lg:w-full ${
                      active ? "theme-nav-active" : "theme-nav-idle"
                    }`}
                  >
                    <div className="flex items-center gap-2">
                      <span className={active ? "theme-icon-soft" : "text-white/70"}>{item.icon}</span>
                      <span className="text-sm font-medium leading-5">{item.label}</span>
                    </div>
                  </button>
                );
              })}
            </nav>

            {isAdminIdentity ? (
              <button
                type="button"
                onClick={onOpenFactoryReset}
                className="mt-3 flex w-full items-center gap-2 rounded-2xl border border-red-500/25 bg-red-500/10 px-3 py-2.5 text-left text-sm font-medium text-red-100 transition hover:bg-red-500/15"
              >
                <ShieldAlert className="h-4 w-4" />
                <span>{t("恢复出厂设置", "Factory Reset")}</span>
              </button>
            ) : null}

            <div className="theme-panel-soft mt-3 p-3.5 text-sm text-white/70">
              <p className="font-medium text-white">{t("当前登录身份", "Current identity")}</p>
              {authMode === "webd" ? (
                <div className="mt-2 space-y-1 text-xs text-white/55">
                  <p>{t("Web 会话（由 webd 注入访问凭证，浏览器不保存明文 key）", "Web session (webd injects credentials; no plaintext key in browser)")}</p>
                  <p>
                    {t("角色", "Role")}: <span className="text-white/75">{authIdentity?.role || "--"}</span>
                  </p>
                  <p className="break-all font-mono">
                    {t("Key", "Key")}: <span className="text-white/75">{maskedIdentityKey || "--"}</span>
                  </p>
                </div>
              ) : (
                <p className="mt-2 break-all font-mono text-xs text-white/55">{maskedSavedUiKey || "--"}</p>
              )}
            </div>
          </div>
        </aside>

        <main className="mx-auto min-w-0 max-w-7xl space-y-4">{children}</main>
      </div>
    </div>
  );
}
