export type BancorAssetSymbol = "POINT" | "USD";

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
  return asset === "POINT" ? formatBancorIntegerAmount(value) : value;
}
