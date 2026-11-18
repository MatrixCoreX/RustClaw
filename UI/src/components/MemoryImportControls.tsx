import { FileUp, Loader2 } from "lucide-react";
import type { MemoryImportProgress } from "../lib/memory-import-upload";

interface Props {
  t: (zh: string, en: string) => string;
  progress: MemoryImportProgress | null;
  onImport: (file: File) => void | Promise<void>;
  onResume: () => void | Promise<void>;
  onCancel: () => void | Promise<void>;
}
export function MemoryImportControls({ t, progress, onImport, onResume, onCancel }: Props) {
  const labels = {
    hashing: t("正在校验文件", "Checking the file"),
    uploading: t("正在上传", "Uploading"),
    parsing: t("文件已上传，正在预检", "File uploaded; checking the import"),
    preview: t("等待确认导入", "Waiting for import confirmation"),
    confirming: t("正在确认整批导入", "Confirming the whole import"),
    paused: t("导入等待继续", "Import paused"),
    cancelling: t("正在停止导入", "Stopping the import"),
  };
  const busy = progress !== null && progress.phase !== "paused";
  const percent = progress && progress.total > 0 && ["hashing", "uploading"].includes(progress.phase)
    ? ` ${Math.floor(progress.received / progress.total * 100)}%` : "";
  return <>
    <label className="theme-secondary-btn cursor-pointer px-3 py-2 text-xs">
      <FileUp className="h-3.5 w-3.5" />{t("导入预检", "Preview import")}
      <input type="file" accept="application/json,.json" className="sr-only" disabled={progress !== null}
        onChange={event => {
          const file = event.target.files?.[0];
          if (file) void onImport(file);
          event.currentTarget.value = "";
        }} />
    </label>
    {progress ? <div className="basis-full space-y-2 text-xs text-white/60">
      <p role="status" className="flex items-center gap-2">
        {busy ? <Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin" /> : null}
        {labels[progress.phase]}{percent}
      </p>
      {progress.phase === "paused" ? <p>{t("可继续同一文件；刷新页面后，重新选择同一文件即可续传。",
        "Continue the same file, or reselect it after refreshing to resume.")}</p> : null}
      <div className="flex flex-wrap gap-2">
        {progress.phase === "paused" ? <button type="button" className="theme-secondary-btn px-3 py-1.5 text-xs"
          onClick={() => void onResume()}>{t("继续导入", "Continue import")}</button> : null}
        <button type="button" className="theme-secondary-btn px-3 py-1.5 text-xs disabled:opacity-50"
          disabled={progress.phase === "confirming" || progress.phase === "cancelling"}
          onClick={() => void onCancel()}>{t("取消这次导入", "Cancel this import")}</button>
      </div>
    </div> : null}
  </>;
}
