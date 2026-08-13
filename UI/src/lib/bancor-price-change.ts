import type { NniBancorMarketResponse } from "../types/api";

export type BancorPriceChangeSide = "buy" | "sell";

export interface BancorPriceChangeProjection {
  side: BancorPriceChangeSide;
  inputAsset: "USD" | "POINT";
  outputAsset: "POINT" | "USD";
  inputAmount: string;
  feeAmount: string;
  effectiveInputAmount: string;
  outputAmount: string;
  pointReserveAfter: string;
  usdReserveAfter: string;
  currentMarginalPrice: string;
  marginalPriceAfter: string;
  marginalPriceChangePercent: string;
}

export type BancorPriceChangeResult =
  | { ok: true; projection: BancorPriceChangeProjection }
  | { ok: false; error: "amount_invalid" | "amount_too_small" | "market_capacity_exceeded" | "market_invalid" };

const ASSET_SCALE = 10_000n;
const BPS_SCALE = 10_000n;
const MAX_UNITS = 9_223_372_036_854_775_807n;

export function calculateBancorPriceChange({
  side,
  inputAmount,
  market,
}: {
  side: BancorPriceChangeSide;
  inputAmount: string;
  market: NniBancorMarketResponse | null;
}): BancorPriceChangeResult {
  const inputUnits = parseAssetUnits(inputAmount);
  if (inputUnits === null) return { ok: false, error: "amount_invalid" };
  if (!market || !Number.isSafeInteger(market.fee_bps) || market.fee_bps < 0 || market.fee_bps >= 10_000) {
    return { ok: false, error: "market_invalid" };
  }

  let pointReserve: bigint;
  let usdReserve: bigint;
  try {
    pointReserve = parseReserveUnits(market.point_reserve_units);
    usdReserve = parseReserveUnits(market.usd_reserve_units);
  } catch {
    return { ok: false, error: "market_invalid" };
  }

  const feeUnits = market.fee_bps === 0
    ? 0n
    : (inputUnits * BigInt(market.fee_bps) + BPS_SCALE - 1n) / BPS_SCALE;
  const curveInputUnits = inputUnits - feeUnits;
  if (curveInputUnits <= 0n) return { ok: false, error: "amount_too_small" };

  const inputReserveUnits = side === "buy" ? usdReserve : pointReserve;
  const outputReserveUnits = side === "buy" ? pointReserve : usdReserve;
  const outputUnits = (curveInputUnits * outputReserveUnits) / (inputReserveUnits + curveInputUnits);
  if (outputUnits <= 0n || outputUnits >= outputReserveUnits) {
    return { ok: false, error: "amount_too_small" };
  }

  const pointReserveAfter = side === "buy"
    ? pointReserve - outputUnits
    : pointReserve + curveInputUnits;
  const usdReserveAfter = side === "buy"
    ? usdReserve + curveInputUnits
    : usdReserve - outputUnits;
  if (pointReserveAfter <= 0n || usdReserveAfter <= 0n) {
    return { ok: false, error: "amount_too_small" };
  }
  if (pointReserveAfter > MAX_UNITS || usdReserveAfter > MAX_UNITS) {
    return { ok: false, error: "market_capacity_exceeded" };
  }

  return {
    ok: true,
    projection: {
      side,
      inputAsset: side === "buy" ? "USD" : "POINT",
      outputAsset: side === "buy" ? "POINT" : "USD",
      inputAmount: formatAssetUnits(inputUnits),
      feeAmount: formatAssetUnits(feeUnits),
      effectiveInputAmount: formatAssetUnits(curveInputUnits),
      outputAmount: formatAssetUnits(outputUnits),
      pointReserveAfter: formatAssetUnits(pointReserveAfter),
      usdReserveAfter: formatAssetUnits(usdReserveAfter),
      currentMarginalPrice: formatUnsignedRatio(usdReserve, pointReserve, 8),
      marginalPriceAfter: formatUnsignedRatio(usdReserveAfter, pointReserveAfter, 8),
      marginalPriceChangePercent: formatSignedPercentChange({
        beforeNumerator: usdReserve,
        beforeDenominator: pointReserve,
        afterNumerator: usdReserveAfter,
        afterDenominator: pointReserveAfter,
        decimalPlaces: 4,
      }),
    },
  };
}

function parseAssetUnits(value: string): bigint | null {
  const match = /^(0|[1-9][0-9]*)(?:\.([0-9]{1,4}))?$/.exec(value.trim());
  if (!match) return null;
  const fraction = (match[2] || "").padEnd(4, "0");
  const units = BigInt(match[1]) * ASSET_SCALE + BigInt(fraction || "0");
  return units > 0n && units <= MAX_UNITS ? units : null;
}

function parseReserveUnits(value: string): bigint {
  if (!/^[1-9][0-9]*$/.test(value)) throw new Error("invalid reserve");
  const units = BigInt(value);
  if (units > MAX_UNITS) throw new Error("invalid reserve");
  return units;
}

function formatAssetUnits(units: bigint): string {
  const whole = units / ASSET_SCALE;
  const fraction = String(units % ASSET_SCALE).padStart(4, "0");
  return `${whole}.${fraction}`;
}

function formatUnsignedRatio(numerator: bigint, denominator: bigint, decimalPlaces: number): string {
  const scale = 10n ** BigInt(decimalPlaces);
  const scaled = (numerator * scale) / denominator;
  const whole = scaled / scale;
  const fraction = String(scaled % scale).padStart(decimalPlaces, "0");
  return `${whole}.${fraction}`;
}

function formatSignedPercentChange({
  beforeNumerator,
  beforeDenominator,
  afterNumerator,
  afterDenominator,
  decimalPlaces,
}: {
  beforeNumerator: bigint;
  beforeDenominator: bigint;
  afterNumerator: bigint;
  afterDenominator: bigint;
  decimalPlaces: number;
}): string {
  const numerator = afterNumerator * beforeDenominator - beforeNumerator * afterDenominator;
  const denominator = afterDenominator * beforeNumerator;
  const scale = 100n * 10n ** BigInt(decimalPlaces);
  const absolute = numerator < 0n ? -numerator : numerator;
  const rounded = (absolute * scale + denominator / 2n) / denominator;
  const whole = rounded / 10n ** BigInt(decimalPlaces);
  const fraction = String(rounded % 10n ** BigInt(decimalPlaces)).padStart(decimalPlaces, "0");
  const sign = numerator > 0n ? "+" : numerator < 0n ? "-" : "";
  return `${sign}${whole}.${fraction}%`;
}
