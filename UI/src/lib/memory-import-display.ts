import { MemoryImportTransportError } from "./memory-import-upload";
import type { UiErrorTranslate } from "./ui-error";

export function memoryImportFailure(error: unknown, t: UiErrorTranslate): string {
  if (error instanceof MemoryImportTransportError) {
    switch (error.code) {
      case "memory_import_upload_file_too_large":
      case "memory_storage_write_blocked":
      case "memory_import_storage_full":
        return t("设备的记忆存储空间不足。请调整存储预算或清理不需要的记忆后，再继续导入。",
          "The device has insufficient memory storage. Adjust its storage budget or remove unwanted memory, then continue the import.");
      case "memory_import_checksum_invalid":
      case "memory_import_upload_file_digest_mismatch":
      case "memory_import_schema_unsupported":
      case "memory_import_file_invalid":
        return t("导入文件无效或校验失败。请重新选择完整的记忆导出文件。",
          "The import file is invalid or failed verification. Select a complete memory export file again.");
      case "memory_import_policy_unavailable":
      case "memory_import_ingress_unavailable":
      case "memory_import_upload_schema_unavailable":
        return t("设备暂时无法接收记忆导入。请检查设备版本和存储设置后重试。",
          "The device cannot currently accept memory imports. Check its version and storage settings, then retry.");
      case "memory_import_upload_state_conflict":
        return t("这次导入的状态已改变。请刷新记忆列表核对，或取消后重新选择文件。",
          "This import's state changed. Refresh the memory list to check it, or cancel and select the file again.");
    }
  }
  return t("暂时无法确认导入结果。网络恢复后可继续同一文件核对，或刷新记忆列表。",
    "The import result could not be confirmed. Continue the same file when the connection recovers, or refresh the memory list.");
}
