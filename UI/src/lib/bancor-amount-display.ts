export type BancorAssetSymbol = "AIC" | "USD";

export function formatBancorIntegerAmount(value: string): string {
  const match = /^([+-]?)(\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!match) return value;
  const [, sign, whole, fraction = ""] = match;
  const rounded = BigInt(whole) + (fraction[0] >= "5" ? 1n : 0n);
  return `${sign}${rounded}`;
}

export function formatBancorAssetAmountForDisplay(
  value: string,
  asset: BancorAssetSymbol,
): string {
  return asset === "AIC" ? formatBancorIntegerAmount(value) : value;
}

export function formatBancorTradeHistoryAmount(value: string): string {
  const match = /^([+-]?\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!match) return value;
  const fraction = (match[2] ?? "").replace(/0+$/, "");
  return fraction ? `${match[1]}.${fraction}` : match[1];
}
