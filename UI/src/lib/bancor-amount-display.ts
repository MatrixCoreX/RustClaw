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
  _asset: BancorAssetSymbol,
): string {
  return value;
}

export function formatBancorTradeHistoryAmount(value: string): string {
  const match = /^([+-]?\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!match) return value;
  const fraction = (match[2] ?? "").replace(/0+$/, "");
  return fraction ? `${match[1]}.${fraction}` : match[1];
}

export function formatBancorBalanceAmount(value: string): string {
  const match = /^([+-]?)(\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!match) return value;

  const [, sign, wholeText, fractionText = ""] = match;
  const whole = BigInt(wholeText);
  const hundredths = fractionText.padEnd(2, "0").slice(0, 2);
  return `${sign}${whole}.${hundredths}`;
}

export function formatBancorBalanceHoverAmount(value: string): string {
  const match = /^([+-]?)(\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!match) return value;

  const [, sign, wholeText, fractionText = ""] = match;
  const whole = BigInt(wholeText);
  const fraction = fractionText.slice(0, 8).replace(/0+$/, "");
  return fraction ? `${sign}${whole}.${fraction}` : `${sign}${whole}`;
}
