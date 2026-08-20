import type {
  NniBancorMarketResponse,
  NniRewardRecord,
  NniRewardWindowKey,
  NniRewardWindowSummary,
  NniRewardsResponse,
} from "../types/api";

const APR_REFERENCE_SECONDS = 365 * 24 * 60 * 60;
export const NNI_APR_AUTO_REFRESH_SECONDS = 10 * 60;

export interface NniAprEstimate {
  record: NniRewardRecord;
  periodSeconds: number;
  rewardAic: number;
  aicPriceUsd: number;
  periodValueUsd: number;
  aprBasisRewardUsd: number;
  aprPercent: number;
}

export interface NniPeriodAprEstimate {
  window: NniRewardWindowSummary;
  rewardAic: number;
  aicPriceUsd: number;
  windowValueUsd: number;
  aprBasisRewardUsd: number;
  aprPercent: number;
}

export interface NniLatestRewardCache {
  devicePubkey: string;
  rewardAic: string | null;
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

export function latestVisibleNniRewardAic(
  rewards: NniRewardsResponse | null,
  cache: NniLatestRewardCache | null,
): string | null {
  if (rewards?.page === 1) {
    return latestNniRewardRecord(rewards.records)?.reward_aic ?? null;
  }
  if (!rewards || !cache || cache.devicePubkey !== rewards.device_pubkey) return null;
  return cache.rewardAic;
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
  const aprBasisRewardUsd = periodValueUsd * (APR_REFERENCE_SECONDS / periodSeconds);
  const aprPercent = (aprBasisRewardUsd / devicePrice) * 100;
  if (![periodValueUsd, aprBasisRewardUsd, aprPercent].every(Number.isFinite)) return null;

  return {
    record,
    periodSeconds,
    rewardAic,
    aicPriceUsd,
    periodValueUsd,
    aprBasisRewardUsd,
    aprPercent,
  };
}

export function calculateNniPeriodAprEstimate({
  devicePriceUsd,
  rewards,
  market,
  windowKey,
}: {
  devicePriceUsd: string;
  rewards: NniRewardsResponse | null;
  market: NniBancorMarketResponse | null;
  windowKey: NniRewardWindowKey;
}): NniPeriodAprEstimate | null {
  const devicePrice = parsePositiveNniDevicePrice(devicePriceUsd);
  const window = rewards?.reward_windows?.find((candidate) => candidate.key === windowKey);
  if (devicePrice === null || !window || !market) return null;

  const rewardAic = Number(window.total_reward_aic);
  const aicPriceUsd = Number(market.marginal_price_usd_per_aic);
  const { window_seconds: windowSeconds } = window;
  if (
    !Number.isFinite(rewardAic)
    || rewardAic < 0
    || !Number.isFinite(aicPriceUsd)
    || aicPriceUsd < 0
    || !Number.isSafeInteger(windowSeconds)
    || windowSeconds <= 0
    || window.window_end_unix - window.window_start_unix !== windowSeconds
  ) {
    return null;
  }

  const windowValueUsd = rewardAic * aicPriceUsd;
  const aprBasisRewardUsd = windowValueUsd * (APR_REFERENCE_SECONDS / windowSeconds);
  const aprPercent = (aprBasisRewardUsd / devicePrice) * 100;
  if (![windowValueUsd, aprBasisRewardUsd, aprPercent].every(Number.isFinite)) return null;

  return {
    window,
    rewardAic,
    aicPriceUsd,
    windowValueUsd,
    aprBasisRewardUsd,
    aprPercent,
  };
}
