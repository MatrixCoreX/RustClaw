import assert from "node:assert/strict";
import test from "node:test";

import { formatSystemActionError } from "./system-actions";

const t = (zh: string, _en: string) => zh;

test("formats system action errors from stable machine fields", () => {
  assert.equal(
    formatSystemActionError(
      { ok: false, data: { error_code: "admin_role_required" } },
      403,
      t,
    ),
    "此操作需要管理员权限。",
  );
  assert.equal(
    formatSystemActionError(
      { ok: false, error: "system_restart_schedule_failed" },
      500,
      t,
    ),
    "系统重启未能启动，请查看服务日志后重试。",
  );
  assert.equal(
    formatSystemActionError(
      { ok: false, error: "future_system_machine_code" },
      500,
      t,
    ),
    "系统操作未完成 (500)，请稍后重试；如果仍然失败，请查看日志。",
  );
});
