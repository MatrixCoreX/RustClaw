import type {
  NniBancorMarketResponse,
  NniRewardRecord,
  NniRewardsResponse,
} from "../types/api";

const SECONDS_PER_YEAR = 365 * 24 * 60 * 60;
export const NNI_APR_AUTO_REFRESH_SECONDS = 10 * 60;

export interface NniAprEstimate {
  record: NniRewardRecord;
  periodSeconds: number;
  rewardAic: number;
  aicPriceUsd: number;
  periodValueUsd: number;
  annualRewardUsd: number;
  aprPercent: number;
}

export function parsePositiveNniDevicePrice(value: string): number | null {
  const normalized = value.trim();
  if (!/^(?:\d+\.?\d*|\.\d+)$/.test(normalized)) return null;
  const parsed = Number(normalized);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
}

export function latestNniRewardRecord(
  records: readonly NniRewardRecord[],
): NniRewardRecord | null {
  return records.reduce<NniRewardRecord | null>((latest, record) => {
    if (!latest) return record;
    if (record.period_end_unix !== latest.period_end_unix) {
      return record.period_end_unix > latest.period_end_unix ? record : latest;
    }
    return record.awarded_at_unix > latest.awarded_at_unix ? record : latest;
  }, null);
}

export function calculateNniAprEstimate({
  devicePriceUsd,
  rewards,
  market,
}: {
  devicePriceUsd: string;
  rewards: NniRewardsResponse | null;
  market: NniBancorMarketResponse | null;
}): NniAprEstimate | null {
  const devicePrice = parsePositiveNniDevicePrice(devicePriceUsd);
  const record = latestNniRewardRecord(rewards?.records ?? []);
  if (devicePrice === null || !record || !market) return null;

  const rewardAic = Number(record.reward_aic);
  const aicPriceUsd = Number(market.marginal_price_usd_per_aic);
  const recordPeriodSeconds = record.period_end_unix - record.period_start_unix;
  const periodSeconds = recordPeriodSeconds > 0
    ? recordPeriodSeconds
    : rewards?.reward_policy?.interval_seconds ?? 0;
  if (
    !Number.isFinite(rewardAic)
    || rewardAic < 0
    || !Number.isFinite(aicPriceUsd)
    || aicPriceUsd < 0
    || !Number.isFinite(periodSeconds)
    || periodSeconds <= 0
  ) {
    return null;
  }

  const periodValueUsd = rewardAic * aicPriceUsd;
  const annualRewardUsd = periodValueUsd * (SECONDS_PER_YEAR / periodSeconds);
  const aprPercent = (annualRewardUsd / devicePrice) * 100;
  if (![periodValueUsd, annualRewardUsd, aprPercent].every(Number.isFinite)) return null;

  return {
    record,
    periodSeconds,
    rewardAic,
    aicPriceUsd,
    periodValueUsd,
    annualRewardUsd,
    aprPercent,
  };
}
