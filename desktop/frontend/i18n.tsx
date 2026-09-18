import { useCallback, useEffect, useSyncExternalStore } from "react";
import { appStorageKey, productCopy } from "../../UI/src/lib/product-identity";
import english from "./i18n/en.json";

export type Language = "zh" | "en";
const key = appStorageKey("monitor.lang");
const changed = "desktop-language-change";
const translations: Record<string, string> = english;
const original = new Map(Object.entries(translations).map(([zh, en]) => [en, zh]));
export const getLanguage = (): Language => localStorage.getItem(key) === "en" ? "en" : "zh";
function translate(language: Language, text: string, en?: string): string {
  const zh = original.get(text) ?? text;
  return productCopy(language === "zh" ? zh : en ?? translations[zh] ?? text);
}
/** Desktop-owned presentation only. Account names, keys and protocol data are not translated. */
export const copy = (zh: string, en?: string) => translate(getLanguage(), zh, en);
export function setLanguage(next: Language | ((current: Language) => Language)) {
  const language = typeof next === "function" ? next(getLanguage()) : next;
  localStorage.setItem(key, language);
  document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
  window.dispatchEvent(new Event(changed));
}
function subscribe(notify: () => void) {
  const storage = (event: StorageEvent) => { if (event.key === key || event.key === null) notify(); };
  window.addEventListener(changed, notify);
  window.addEventListener("storage", storage);
  return () => { window.removeEventListener(changed, notify); window.removeEventListener("storage", storage); };
}
export function useLanguage() {
  const lang = useSyncExternalStore(subscribe, getLanguage, () => "zh" as const);
  useEffect(() => { document.documentElement.lang = lang === "zh" ? "zh-CN" : "en"; }, [lang]);
  const t = useCallback((zh: string, en?: string) => translate(lang, zh, en), [lang]);
  return { lang, t, setLang: setLanguage };
}
export function LanguageToggle({ className = "" }: { className?: string }) {
  const { lang, t, setLang } = useLanguage();
  return <button type="button" className={className} data-desktop-language-toggle
    aria-label={t("切换语言", "Switch language")} title={t("切换语言", "Switch language")}
    onClick={() => setLang(lang === "zh" ? "en" : "zh")}>{lang === "zh" ? "中文" : "English"}</button>;
}
