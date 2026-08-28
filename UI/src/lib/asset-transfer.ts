import { validateNniOwnerPublicKey } from "./nni-owner-public-key";

const ASSET_SCALE = 100_000_000n;
const MAX_ASSET_UNITS = 9_223_372_036_854_775_807n;

export type AssetTransferAsset = "AIC" | "USD";

export type AssetTransferDraftError =
  | "source_required"
  | "recipient_required"
  | "recipient_invalid"
  | "same_account"
  | "amount_invalid"
  | "amount_too_large"
  | "insufficient_balance";

export type AssetTransferDraftValidation =
  | {
    ok: true;
    sourcePublicKey: string;
    recipientPublicKey: string;
    asset: AssetTransferAsset;
    amount: string;
    amountUnits: bigint;
  }
  | { ok: false; error: AssetTransferDraftError };

function decimalUnits(value: string): { amount: string; units: bigint } | null {
  const normalized = value.trim();
  const match = /^(\d+)(?:\.(\d{1,8}))?$/.exec(normalized);
  if (!match) return null;
  const whole = BigInt(match[1]);
  const fraction = (match[2] ?? "").padEnd(8, "0");
  const units = whole * ASSET_SCALE + BigInt(fraction || "0");
  if (units <= 0n) return null;
  return {
    amount: `${whole}.${fraction}`,
    units,
  };
}

export function validateAssetTransferDraft({
  sourcePublicKey,
  recipientPublicKey,
  asset,
  amount,
  availableBalance,
}: {
  sourcePublicKey: string;
  recipientPublicKey: string;
  asset: AssetTransferAsset;
  amount: string;
  availableBalance: string;
}): AssetTransferDraftValidation {
  const source = validateNniOwnerPublicKey(sourcePublicKey);
  if (!source.ok) return { ok: false, error: "source_required" };
  if (!recipientPublicKey.trim()) return { ok: false, error: "recipient_required" };
  const recipient = validateNniOwnerPublicKey(recipientPublicKey);
  if (!recipient.ok) return { ok: false, error: "recipient_invalid" };
  if (source.normalized === recipient.normalized) {
    return { ok: false, error: "same_account" };
  }
  const parsedAmount = decimalUnits(amount);
  if (!parsedAmount) return { ok: false, error: "amount_invalid" };
  if (parsedAmount.units > MAX_ASSET_UNITS) {
    return { ok: false, error: "amount_too_large" };
  }
  const parsedBalance = decimalUnits(availableBalance);
  const balanceUnits = parsedBalance?.units ?? 0n;
  if (parsedAmount.units > balanceUnits) {
    return { ok: false, error: "insufficient_balance" };
  }
  return {
    ok: true,
    sourcePublicKey: source.normalized,
    recipientPublicKey: recipient.normalized,
    asset,
    amount: parsedAmount.amount,
    amountUnits: parsedAmount.units,
  };
}
