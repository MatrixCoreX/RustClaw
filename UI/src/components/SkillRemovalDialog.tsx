import { Trash2, X } from "lucide-react";

type Translate = (zh: string, en: string) => string;

export interface SkillRemovalDialogProps {
  skillName: string;
  existingConfigFiles?: string[];
  t: Translate;
  onCancel: () => void;
  onConfirm: (preserveConfig: boolean) => unknown | Promise<unknown>;
}

export function SkillRemovalDialog({
  skillName,
  existingConfigFiles = [],
  t,
  onCancel,
  onConfirm,
}: SkillRemovalDialogProps) {
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
                "技能二进制会被删除，并从工具/技能页移除。请选择是否保留它的独立配置文件。",
                "The skill binary will be deleted and removed from Tools/Skills. Choose whether to keep its dedicated configuration files.",
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
        <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
          <button type="button" onClick={onCancel} className="theme-topbar-btn justify-center px-4 py-2 text-xs">
            {t("取消", "Cancel")}
          </button>
          <button
            type="button"
            onClick={() => void onConfirm(false)}
            className="inline-flex items-center justify-center gap-2 rounded border border-red-500/30 bg-red-500/10 px-4 py-2 text-xs font-medium text-red-100 hover:bg-red-500/15"
          >
            <Trash2 className="h-4 w-4" />
            {t("删除技能和配置", "Remove skill and config")}
          </button>
          <button
            type="button"
            onClick={() => void onConfirm(true)}
            className="theme-accent-btn justify-center px-4 py-2 text-xs"
          >
            {t("保留配置并删除", "Keep config and remove")}
          </button>
        </div>
      </div>
    </div>
  );
}
