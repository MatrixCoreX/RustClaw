import type { ApiResponse, SkillStoreOperation, SkillStoreOperationResponse } from "../types/api";

type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export class AippSkillRemovalError extends Error {
  constructor(readonly code: string, readonly pending = false) {
    super(code);
  }
}

export async function removeAippAndSkill(
  apiFetch: ApiFetch,
  skillName: string,
  { pollAttempts = 80, pollDelayMs = 1_500, signal }: { pollAttempts?: number; pollDelayMs?: number; signal?: AbortSignal } = {},
): Promise<void> {
  signal?.throwIfAborted();
  const readOperation = async (response: Response): Promise<SkillStoreOperation> => {
    const body = await response.json() as ApiResponse<SkillStoreOperationResponse>;
    signal?.throwIfAborted();
    if (!response.ok || !body.ok || !body.data?.operation) {
      throw new AippSkillRemovalError(body.error || "skill_store_operation_state_failed");
    }
    const operation = body.data.operation;
    if (!operation.operation_id || operation.skill_name !== skillName || operation.action !== "remove") {
      throw new AippSkillRemovalError("skill_store_operation_state_failed");
    }
    return operation;
  };
  let operation = await readOperation(await apiFetch("/v1/skills/store/remove", {
    method: "POST",
    signal,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ skill_name: skillName, preserve_config: true, preserve_data: true }),
  }));
  const operationId = operation.operation_id;
  for (let attempt = 0; ; attempt += 1) {
    if (operation.operation_id !== operationId) throw new AippSkillRemovalError("skill_store_operation_state_failed");
    if (operation.status === "success") {
      if (operation.result?.installed !== false) throw new AippSkillRemovalError("skill_store_operation_state_failed");
      return;
    }
    if (operation.status === "failure" || operation.status === "cancelled") {
      throw new AippSkillRemovalError(operation.failure?.error_code || "skill_store_operation_state_failed");
    }
    if (operation.status !== "queued" && operation.status !== "running") {
      throw new AippSkillRemovalError("skill_store_operation_state_failed");
    }
    if (attempt >= pollAttempts) throw new AippSkillRemovalError("skill_store_operation_pending", true);
    await new Promise((resolve) => setTimeout(resolve, pollDelayMs));
    signal?.throwIfAborted();
    operation = await readOperation(await apiFetch(`/v1/skills/store/operations/${encodeURIComponent(operationId)}`, { cache: "no-store", signal }));
  }
}
