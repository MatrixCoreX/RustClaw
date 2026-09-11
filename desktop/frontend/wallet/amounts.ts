const MAX = 9223372036854775807n;
export function amountUnits(value: string): string | null {
  const match = /^(\d{1,11})(?:\.(\d{1,8}))?$/.exec(value.trim());
  if (!match) return null;
  const units =
    BigInt(match[1]) * 100000000n + BigInt((match[2] ?? "").padEnd(8, "0"));
  return units > 0n && units <= MAX ? units.toString() : null;
}
export function displayUnits(value: string | undefined): string {
  if (value === undefined || !/^\d{1,19}$/.test(value)) return "—";
  const n = BigInt(value);
  if (n > MAX) return "—";
  return `${n / 100000000n}.${(n % 100000000n).toString().padStart(8, "0")}`;
}
export function shortPublic(value: string): string {
  return `${value.slice(0, 8)}…${value.slice(-8)}`;
}

export function movementSign(kind: string, asset: string): string {
  if (kind === "transfer_in") return "+";
  if (kind === "transfer_out") return "-";
  if (kind === "bancor_buy") return asset === "USD" ? "-" : "+";
  if (kind === "bancor_sell") return asset === "AIC" ? "-" : "+";
  return "";
}
