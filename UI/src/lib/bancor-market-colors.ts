export type BancorMarketDirection = "up" | "down";
type TranslateProbe = (zh: string, en: string) => string;

export function resolveBancorMarketDirectionColors(t: TranslateProbe): {
  up: string;
  down: string;
} {
  return t("zh", "en") === "zh"
    ? { up: "#f87171", down: "#34d399" }
    : { up: "#34d399", down: "#f87171" };
}

export function resolveBancorMarketDirectionColor(
  direction: BancorMarketDirection,
  t: TranslateProbe,
): string {
  return resolveBancorMarketDirectionColors(t)[direction];
}
