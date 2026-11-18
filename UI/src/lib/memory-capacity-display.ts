import type { ByteMeasurement, CapacityDecision, ManualGrowthDecision } from "./memory-capacity";
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

export function manualGrowthLabel(decision: ManualGrowthDecision | undefined, t: Translate): string {
  if (decision === undefined) return t("新增内容：状态尚不可用，暂不能确认存储预算", "New content: status unavailable; storage budget is unconfirmed");
  const labels: Record<ManualGrowthDecision, [string, string]> = {
    available: ["新增内容：记忆存储预算可用", "New content: memory storage budget is available"],
    storage_state_missing: ["新增内容：缺少存储状态，请联系管理员检查", "New content: storage observation is missing; contact an administrator"],
    storage_state_unknown: ["新增内容：存储状态需要检查", "New content: storage observation needs attention"],
    storage_state_unavailable: ["新增内容：暂时无法读取存储状态", "New content: storage observation is unavailable"],
    storage_policy_missing: ["新增内容：缺少存储配置记录，请联系管理员检查", "New content: storage configuration is missing; contact an administrator"],
    storage_policy_invalid: ["新增内容：存储配置记录需要检查", "New content: storage configuration needs attention"],
    storage_policy_unavailable: ["新增内容：暂时无法读取存储配置", "New content: storage configuration is unavailable"],
    measurement_unavailable: ["新增内容：暂时无法测量记忆占用", "New content: memory usage cannot be measured"],
    domain_budget_reached: ["新增内容：记忆存储预算已用满，可先导出或删除不需要的内容", "New content: memory storage budget reached; export or delete unwanted content"],
    explicit_stop_unbound: ["新增内容：存储保护已暂停写入，请联系管理员检查", "New content: storage protection pauses writes; contact an administrator"],
  };
  return t(...labels[decision]);
}

export function measurementLabel(value: ByteMeasurement, locale: string, t: Translate): string {
  if (value.status === "admin_only") return t("仅管理员可见", "Administrator only");
  if (value.status === "unavailable") return t("暂不可测", "Measurement unavailable");
  return `${BigInt(value.bytes).toLocaleString(locale)} B`;
}
