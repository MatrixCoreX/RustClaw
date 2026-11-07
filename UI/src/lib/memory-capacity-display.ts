import type { ByteMeasurement, CapacityDecision } from "./memory-capacity";
type Translate = (zh: string, en: string) => string;

export function capacityLabel(decision: CapacityDecision, t: Translate) {
  const labels: Record<CapacityDecision, [string, string]> = {
    available: ["容量检查未阻止自动生成", "Capacity checks do not block automatic generation"],
    storage_paused: ["存储压力使自动生成暂停", "Storage pressure pauses automatic generation"],
    storage_state_missing: ["缺少存储状态，暂不能确认容量", "Storage observation is missing"],
    storage_state_unknown: ["存储状态无法识别", "Storage observation is not recognized"],
    principal_quota_missing: ["当前账号缺少配额设置", "Account quota policy is missing"],
    principal_quota_invalid: ["当前账号配额数据需要检查", "Account quota data needs attention"],
    principal_rows_exhausted: ["近期原文条数已达配额", "Source transcript row quota reached"],
    principal_bytes_exhausted: ["近期原文字节数已达配额", "Source transcript byte quota reached"],
    principal_background_cost_exhausted: ["后台逻辑工作量已达配额", "Background logical-work quota reached"],
  };
  return t(...labels[decision]);
}

export function measurementLabel(value: ByteMeasurement, locale: string, t: Translate): string {
  if (value.status === "admin_only") return t("仅管理员可见", "Administrator only");
  if (value.status === "unavailable") return t("暂不可测", "Measurement unavailable");
  return `${BigInt(value.bytes).toLocaleString(locale)} B`;
}
