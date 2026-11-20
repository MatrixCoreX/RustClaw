import { formatBytes } from "./display-format";

export interface ResourceWaitDiagnostics {
  schema_version?: number;
  reason_code?: string;
  volumes?: { role?: string; state?: string; available_bytes?: number | null; critical_reserve_bytes?: number | null; shortfall_bytes?: number | null }[];
  memory?: { available_bytes?: number | null; required_bytes?: number | null; safety_reserve_bytes?: number | null };
  cpu?: { available_cores?: number | null; required_cores?: number | null };
}

function known(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

export function resourceWaitDetail(value: ResourceWaitDiagnostics | null | undefined, lang: "zh" | "en"): string | null {
  if (!value || value.schema_version !== 1) return null;
  const t = (zh: string, en: string) => lang === "zh" ? zh : en;
  const volume = value.volumes?.find((item) => item.state === "critical" || item.state === "unavailable");
  if (volume) {
    const roles: Record<string, string> = { system: t("系统盘", "System disk"), workspace: t("工作目录磁盘", "Workspace disk"), temporary: t("临时目录磁盘", "Temporary disk"), database: t("数据库磁盘", "Database disk"), audit_database: t("审计数据库磁盘", "Audit database disk") };
    const label = roles[volume.role ?? ""] ?? t("任务所需磁盘", "Required disk");
    if (!known(volume.available_bytes) || !known(volume.critical_reserve_bytes)) {
      return t(`暂时无法读取${label}状态，状态恢复后会自动继续。`, `${label} status is unavailable. The task will continue automatically when it can be checked.`);
    }
    const available = formatBytes(volume.available_bytes), reserve = formatBytes(volume.critical_reserve_bytes);
    const shortage = known(volume.shortfall_bytes) && volume.shortfall_bytes > 0
      ? t(`请至少释放 ${formatBytes(volume.shortfall_bytes)} 空间。`, `Free at least ${formatBytes(volume.shortfall_bytes)}. `) : "";
    return t(`${label}可用 ${available}，需要高于 ${reserve}。${shortage}空间足够后会自动继续。`, `${label} has ${available} available and needs more than ${reserve}. ${shortage}The task will continue automatically when space is available.`);
  }
  const reason = value.reason_code;
  if (reason === "memory_unavailable") {
    if (known(value.memory?.available_bytes) && known(value.memory?.required_bytes)) {
      return t(`当前系统可用内存为 ${formatBytes(value.memory.available_bytes)}，本步骤需要 ${formatBytes(value.memory.required_bytes)}。内存可用后会自动继续。`, `${formatBytes(value.memory.available_bytes)} host memory is available; this step needs ${formatBytes(value.memory.required_bytes)}. It will continue automatically when memory is available.`);
    }
    return t("正在等待足够的内存，内存可用后会自动继续。", "Waiting for enough memory; the task will continue automatically when it is available.");
  }
  if (reason === "cpu_unavailable") return t("正在等待可用的 CPU 资源，资源可用后会自动继续。", "Waiting for available CPU capacity; the task will continue automatically.");
  if (reason === "browser_slot_unavailable") return t("浏览器任务名额已满，空出名额后会自动继续。", "Browser task slots are full. The task will continue automatically when a slot is available.");
  if (reason === "provider_slot_unavailable") return t("模型请求并发已满，空出名额后会自动继续。", "Model request slots are full. The task will continue automatically when a slot is available.");
  if (reason === "network_slot_unavailable") return t("网络任务名额已满，空出名额后会自动继续。", "Network task slots are full. The task will continue automatically when a slot is available.");
  if (reason === "gpu_unavailable_no_fallback") return t("正在等待可用的 GPU 资源，资源可用后会自动继续。", "Waiting for available GPU capacity; the task will continue automatically.");
  if (reason === "resource_pressure") return t("系统负载较高，正在等待运行资源释放；资源可用后会自动继续。", "System load is high. The task will continue automatically when runtime capacity becomes available.");
  return null;
}
