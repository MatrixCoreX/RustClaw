export type UiLanguage = "zh" | "en";

function copy(lang: UiLanguage, zh: string, en: string): string {
  return lang === "zh" ? zh : en;
}

export function memoryFactStatusLabel(status: string, lang: UiLanguage): string {
  const normalized = status.toLowerCase();
  if (normalized === "active") return copy(lang, "有效", "Active");
  if (normalized === "expired") return copy(lang, "已过期", "Expired");
  if (normalized === "superseded") return copy(lang, "已替换", "Superseded");
  if (normalized === "deleted") return copy(lang, "已删除", "Deleted");
  return status || "--";
}

export function memorySafetyLabel(flag: string, lang: UiLanguage): string {
  const normalized = flag.toLowerCase();
  if (!normalized || normalized === "safe" || normalized === "normal") return copy(lang, "普通", "Normal");
  return copy(lang, "已标记", "Flagged");
}

export function shouldHideMemoryRecentContent(flag: string): boolean {
  const normalized = flag.toLowerCase();
  return Boolean(normalized && normalized !== "safe" && normalized !== "normal");
}
