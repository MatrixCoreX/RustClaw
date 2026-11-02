import type { AippAccountSnapshot, AippAccountUse, ApiResponse, TaskQueryResponse } from "../types/api";

type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
export type AccountUse = AippAccountUse;
const record = (value: unknown): Record<string, unknown> | null => value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;

export function accountRows(value: unknown): AippAccountSnapshot[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap(item => {
    const row = record(item);
    if (!row || row.schema_version !== 1 || typeof row.platform !== "string" || !/^[a-z0-9_]{1,64}$/u.test(row.platform)) return [];
    const status = ["verified", "saved_session", "expired", "signed_out", "unknown"].includes(String(row.status)) ? row.status as AippAccountSnapshot["status"] : "unknown";
    return [{ schema_version: 1, platform: row.platform, status,
      display_name: status === "verified" && typeof row.display_name === "string" && row.display_name.trim() && row.display_name.length <= 256 && !/[\u0000-\u001f\u007f]/u.test(row.display_name) ? row.display_name : null,
      checked_at: typeof row.checked_at === "number" && Number.isFinite(row.checked_at) && row.checked_at >= 0 && row.checked_at < 1e12 ? row.checked_at : null,
      session_present: typeof row.session_present === "boolean" ? row.session_present : null }];
  });
}

export function accountsFromTask(result: unknown): AippAccountSnapshot[] {
  const root = record(result);
  const direct = record(root?.extra);
  if (Array.isArray(direct?.accounts)) return accountRows(direct.accounts);
  const trace = record(record(root?.task_journal)?.trace);
  const results = Array.isArray(trace?.capability_results) ? trace.capability_results : [];
  return results.flatMap(item => accountRows(record(record(record(item)?.data)?.extra)?.accounts));
}

export async function invokeAccountControl(apiFetch: ApiFetch, skill: string, operation: "status" | "logout", signal: AbortSignal, platform?: string): Promise<AippAccountSnapshot[]> {
  const response = await apiFetch(`/v1/aipps/${encodeURIComponent(skill)}/account-control`, {
    method: "POST", signal, headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ operation, ...(platform ? { platform } : {}), confirm: operation === "logout" }),
  });
  const body = await response.json() as ApiResponse<{ task_id: string }>;
  if (!response.ok || !body.ok || !body.data?.task_id) throw new Error(body.error || "aipp_account_submit_failed");
  for (;;) {
    signal.throwIfAborted();
    const taskResponse = await apiFetch(`/v1/tasks/${encodeURIComponent(body.data.task_id)}`, { signal });
    const taskBody = await taskResponse.json() as ApiResponse<TaskQueryResponse>;
    signal.throwIfAborted();
    if (!taskResponse.ok || !taskBody.ok || !taskBody.data) throw new Error(taskBody.error || "aipp_account_query_failed");
    if (taskBody.data.status === "succeeded") {
      const rows = accountsFromTask(taskBody.data.result_json);
      if (!rows.length) throw new Error("aipp_account_result_unavailable");
      return rows;
    }
    if (["failed", "canceled", "timeout"].includes(taskBody.data.status)) {
      const result = record(taskBody.data.result_json);
      const trace = record(record(result?.task_journal)?.trace);
      const results = Array.isArray(trace?.capability_results) ? trace.capability_results : [];
      const codes = [record(result?.extra)?.error_code, ...results.flatMap(item => [record(item)?.error_code, record(record(record(item)?.data)?.extra)?.error_code])];
      throw new Error(codes.includes("account_session_busy") ? "account_session_busy" : "aipp_account_operation_failed");
    }
    await new Promise<void>((resolve, reject) => {
      const abort = () => { clearTimeout(timer); reject(signal.reason); };
      const timer = setTimeout(() => { signal.removeEventListener("abort", abort); resolve(); }, 500);
      signal.addEventListener("abort", abort, { once: true });
      if (signal.aborted) abort();
    });
  }
}
