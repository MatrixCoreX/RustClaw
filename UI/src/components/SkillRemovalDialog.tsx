import { useState } from "react";
import { Database, Trash2, X } from "lucide-react";

type Translate = (zh: string, en: string) => string;

export interface SkillRemovalDialogProps {
  skillName: string;
  existingConfigFiles?: string[];
  storageKind?: string | null;
  privateDataState?: "present" | "empty" | null;
  t: Translate;
  onCancel: () => void;
  onConfirm: (preserveConfig: boolean, preserveData: boolean) => unknown | Promise<unknown>;
}

export function SkillRemovalDialog({
  skillName,
  existingConfigFiles = [],
  storageKind,
  privateDataState,
  t,
  onCancel,
  onConfirm,
}: SkillRemovalDialogProps) {
  const [preserveConfig, setPreserveConfig] = useState(true);
  const [preserveData, setPreserveData] = useState(true);
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="skill-remove-title"
    >
      <div className="w-full max-w-lg border border-white/15 bg-[#161a24] p-5 shadow-2xl rounded-lg">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h3 id="skill-remove-title" className="text-base font-semibold text-white">
              {t(`删除 ${skillName}`, `Remove ${skillName}`)}
            </h3>
            <p className="mt-2 text-sm leading-6 text-white/60">
              {t(
                "技能运行文件会被删除，并从工具/技能页移除。默认保留配置和私有数据，方便以后重新安装。",
                "The skill runner will be deleted and removed from Tools/Skills. Configuration and private data are preserved by default for reinstallation.",
              )}
            </p>
          </div>
          <button
            type="button"
            onClick={onCancel}
            className="theme-topbar-btn h-9 w-9 shrink-0 justify-center p-0"
            title={t("取消", "Cancel")}
            aria-label={t("取消删除", "Cancel removal")}
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        {existingConfigFiles.length ? (
          <div className="mt-4 border border-white/10 bg-white/5 px-3 py-2 rounded">
            <p className="text-xs font-medium text-white/70">{t("当前配置", "Current configuration")}</p>
            {existingConfigFiles.map((path) => (
              <p key={path} className="mt-1 break-all font-mono text-xs text-white/50">
                {path}
              </p>
            ))}
          </div>
        ) : (
          <p className="mt-4 text-xs text-white/45">
            {t("当前没有检测到独立配置文件。", "No dedicated configuration file is currently present.")}
          </p>
        )}
        <div className="mt-4 space-y-2">
          <label className="flex cursor-pointer items-start gap-3 rounded border border-white/10 bg-white/5 px-3 py-3">
            <input
              type="checkbox"
              checked={preserveConfig}
              onChange={(event) => setPreserveConfig(event.target.checked)}
              className="mt-0.5 h-4 w-4"
            />
            <span>
              <span className="block text-sm font-medium text-white/80">{t("保留独立配置", "Keep configuration")}</span>
              <span className="mt-1 block text-xs leading-5 text-white/50">
                {t("重新安装时继续使用当前设置。", "Reuse the current settings after reinstallation.")}
              </span>
            </span>
          </label>
          {storageKind ? (
            <label className="flex cursor-pointer items-start gap-3 rounded border border-white/10 bg-white/5 px-3 py-3">
              <input
                type="checkbox"
                checked={preserveData}
                onChange={(event) => setPreserveData(event.target.checked)}
                className="mt-0.5 h-4 w-4"
              />
              <Database className="mt-0.5 h-4 w-4 shrink-0 text-white/55" />
              <span>
                <span className="block text-sm font-medium text-white/80">{t("保留技能私有数据", "Keep private skill data")}</span>
                <span className="mt-1 block text-xs leading-5 text-white/50">
                  {privateDataState === "present"
                    ? t("检测到已保存的数据。取消勾选会永久删除，且只影响这个技能。", "Saved data was detected. Clearing this option permanently removes only this skill's data.")
                    : t("当前没有检测到业务数据；保留仍是更安全的默认选择。", "No domain data was detected; keeping it remains the safer default.")}
                </span>
              </span>
            </label>
          ) : null}
        </div>
        <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
          <button type="button" onClick={onCancel} className="theme-topbar-btn justify-center px-4 py-2 text-xs">
            {t("取消", "Cancel")}
          </button>
          <button
            type="button"
            onClick={() => void onConfirm(preserveConfig, storageKind ? preserveData : true)}
            className="inline-flex items-center justify-center gap-2 rounded border border-red-500/30 bg-red-500/10 px-4 py-2 text-xs font-medium text-red-100 hover:bg-red-500/15"
          >
            <Trash2 className="h-4 w-4" />
            {t("确认删除技能", "Remove skill")}
          </button>
        </div>
      </div>
    </div>
  );
}
