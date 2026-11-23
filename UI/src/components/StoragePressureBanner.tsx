import { useEffect, useState } from "react";
import { TriangleAlert } from "lucide-react";
import { appStorageKey } from "../lib/product-identity";
import type { HealthResponse } from "../types/api";

const IGNORE_MINUTES = 30;
const IGNORE_MS = IGNORE_MINUTES * 60_000;

function warningStorageKey(apiBase: string): string | null {
  try {
    const url = new URL(apiBase, window.location.href);
    return appStorageKey(`monitor.storageWarningIgnoredUntil:${url.origin}${url.pathname.replace(/\/+$/, "")}`);
  } catch {
    return null;
  }
}

function storedIgnoreDeadline(key: string | null): number {
  if (!key) return 0;
  try {
    const deadline = Number(window.localStorage.getItem(key));
    const now = Date.now();
    return deadline > now && deadline <= now + IGNORE_MS ? deadline : 0;
  } catch {
    return 0;
  }
}

export function StoragePressureBanner({ apiBase, pressure, t }: {
  apiBase: string;
  pressure: HealthResponse["storage_pressure"];
  t: (zh: string, en: string) => string;
}) {
  const key = warningStorageKey(apiBase);
  const [ignoredUntil, setIgnoredUntil] = useState(() => storedIgnoreDeadline(key));
  useEffect(() => {
    if (!ignoredUntil) return;
    const timer = setTimeout(() => setIgnoredUntil(0), Math.max(0, ignoredUntil - Date.now()));
    return () => clearTimeout(timer);
  }, [ignoredUntil]);

  if (!pressure || pressure.state === "normal") return null;
  const dismissible = pressure.state === "warning" && pressure.write_admission;
  if (dismissible && ignoredUntil) return null;

  const ignore = () => {
    const deadline = Date.now() + IGNORE_MS;
    setIgnoredUntil(deadline);
    try {
      if (key) window.localStorage.setItem(key, String(deadline));
    } catch {
      // The current page still honors the deadline when storage is unavailable.
    }
  };

  return (
    <div className="theme-storage-warning">
      <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-4 gap-y-2">
        <p role="status" className="min-w-0 flex-1 basis-64">
          {pressure.write_admission
            ? t("磁盘可用空间偏低，请清理旧文件或扩容。", "Disk space is low. Remove old files or expand storage.")
            : t("磁盘空间不足或状态暂时无法确认，已暂停新的文件写入。请释放空间后重试。", "New file writes are paused because storage is full or its status is unavailable. Free space and retry.")}
        </p>
        {dismissible && (
          <button type="button" className="theme-storage-warning-dismiss" onClick={ignore}>
            {t(`忽略 ${IGNORE_MINUTES} 分钟`, `Hide for ${IGNORE_MINUTES} minutes`)}
          </button>
        )}
      </div>
    </div>
  );
}
