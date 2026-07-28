import type { RefObject } from "react";
import { ChevronDown, Loader2, Sparkles } from "lucide-react";

import type { ImportedSkillResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;

export interface SkillImportPanelProps {
  t: Translate;
  skillImportSource: string;
  skillImportLoading: boolean;
  skillImportError: string | null;
  skillImportMessage: string | null;
  systemRestartMessage: string | null;
  skillImportPreview: ImportedSkillResponse | null;
  localImportPickerOpen: boolean;
  folderImportInputRef: RefObject<HTMLInputElement | null>;
  fileImportInputRef: RefObject<HTMLInputElement | null>;
  onSkillImportSourceChange: (value: string) => void;
  onImportExternalSkill: () => unknown | Promise<unknown>;
  onLocalImportPickerOpenChange: (value: boolean | ((previous: boolean) => boolean)) => void;
  onUploadImportedSkillFiles: (fileList: FileList | null) => unknown | Promise<unknown>;
  onDismissSkillImportPreview: () => void;
}

export function SkillImportPanel({
  t,
  skillImportSource,
  skillImportLoading,
  skillImportError,
  skillImportMessage,
  systemRestartMessage,
  skillImportPreview,
  localImportPickerOpen,
  folderImportInputRef,
  fileImportInputRef,
  onSkillImportSourceChange,
  onImportExternalSkill,
  onLocalImportPickerOpenChange,
  onUploadImportedSkillFiles,
  onDismissSkillImportPreview,
}: SkillImportPanelProps) {
  return (
    <div className="mb-5">
      <div className="rounded-2xl border border-sky-500/20 bg-sky-500/10 p-4 sm:p-5">
        <div className="flex items-start justify-between gap-3">
          <div>
            <p className="text-[10px] uppercase tracking-[0.28em] text-sky-100/70">{t("第三方技能", "Third-party skills")}</p>
            <h3 className="mt-2 text-base font-semibold text-white">
              {t("把别人做好的技能接入进来，扩展 RustClaw 的能力。", "Bring in ready-made skills to extend what RustClaw can do.")}
            </h3>
            <p className="mt-2 text-sm text-white/65">
              {t(
                "请选择包含 skill.toml 和 INTERFACE.md 的本地技能包。构建、协议冒烟和回执验证全部通过后才会开启。",
                "Choose a local package containing skill.toml and INTERFACE.md. It is enabled only after build, protocol smoke, and receipt verification pass.",
              )}
            </p>
          </div>
          <Sparkles className="mt-1 h-4 w-4 shrink-0 text-sky-200" />
        </div>
        <div className="mt-4 grid gap-3 md:grid-cols-[minmax(0,1fr)_auto]">
          <label className="block space-y-2">
            <span className="text-[10px] uppercase tracking-widest text-sky-100/70">{t("本地技能包目录", "Local package directory")}</span>
            <input
              className="theme-input"
              value={skillImportSource}
              onChange={(event) => onSkillImportSourceChange(event.target.value)}
              placeholder={t(
                "例如 /path/to/external_skills/my_skill",
                "For example, /path/to/external_skills/my_skill",
              )}
            />
          </label>
          <div className="flex items-end">
            <button
              type="button"
              onClick={() => void onImportExternalSkill()}
              disabled={skillImportLoading}
              className="theme-accent-btn px-4 py-2.5 text-sm"
            >
              {skillImportLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <Sparkles className="h-4 w-4" />}
              {t("导入 Skill", "Import Skill")}
            </button>
          </div>
        </div>
        <div className="mt-3">
          <div className="relative inline-flex">
            <button
              type="button"
              onClick={() => onLocalImportPickerOpenChange((previous) => !previous)}
              disabled={skillImportLoading}
              className="inline-flex items-center gap-2 rounded-xl border border-white/20 bg-white/5 px-3 py-2 text-xs text-white/85 hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {t("选择本地技能", "Choose Local Skill")}
              <ChevronDown className={`h-3.5 w-3.5 transition-transform ${localImportPickerOpen ? "rotate-180" : ""}`} />
            </button>
            {localImportPickerOpen ? (
              <div className="absolute left-0 top-full z-20 mt-2 min-w-[12rem] rounded-xl border border-white/10 bg-[#12151f] p-1.5 shadow-2xl">
                <button
                  type="button"
                  onClick={() => {
                    onLocalImportPickerOpenChange(false);
                    folderImportInputRef.current?.click();
                  }}
                  className="flex w-full items-center justify-between rounded-lg px-3 py-2 text-left text-xs text-white/85 hover:bg-white/5"
                >
                  <span>{t("从文件夹导入", "Import Folder")}</span>
                  <span className="text-[10px] text-white/40">{t("适合整个技能包", "Full bundle")}</span>
                </button>
                <button
                  type="button"
                  onClick={() => {
                    onLocalImportPickerOpenChange(false);
                    fileImportInputRef.current?.click();
                  }}
                  className="flex w-full items-center justify-between rounded-lg px-3 py-2 text-left text-xs text-white/85 hover:bg-white/5"
                >
                  <span>{t("选择包内文件", "Select Package Files")}</span>
                  <span className="text-[10px] text-white/40">{t("必须包含 manifest 和接口", "Manifest + interface required")}</span>
                </button>
              </div>
            ) : null}
          </div>
          <input
            ref={folderImportInputRef}
            type="file"
            className="hidden"
            multiple
            onChange={(event) => void onUploadImportedSkillFiles(event.target.files)}
            {...({ webkitdirectory: "", directory: "" } as Record<string, string>)}
          />
          <input
            ref={fileImportInputRef}
            type="file"
            className="hidden"
            multiple
            onChange={(event) => void onUploadImportedSkillFiles(event.target.files)}
          />
        </div>
        {skillImportError ? (
          <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
            {skillImportError}
          </p>
        ) : null}
        {skillImportMessage ? (
          <p className="mt-3 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
            {skillImportMessage}
          </p>
        ) : null}
        {systemRestartMessage ? (
          <p className="mt-3 rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-sm text-white/80">
            {systemRestartMessage}
          </p>
        ) : null}
        {skillImportPreview ? (
          <div className="mt-3 rounded-lg border border-white/10 bg-[#12151f] px-3 py-3 text-xs text-white/75">
            <div className="flex flex-wrap items-start justify-between gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <span className="rounded-md border border-sky-400/30 bg-sky-500/10 px-2 py-1 text-sky-200">{skillImportPreview.skill_name}</span>
                <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-white/70">{skillImportPreview.build_adapter}</span>
                <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-white/70">{skillImportPreview.launcher}</span>
                <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-white/70">v{skillImportPreview.package_version}</span>
              </div>
              <button
                type="button"
                onClick={onDismissSkillImportPreview}
                className="rounded-md border border-white/15 bg-white/5 px-2 py-1 text-[11px] text-white/65 hover:bg-white/10 hover:text-white/85"
              >
                {t("收起", "Dismiss")}
              </button>
            </div>
            <p className="mt-2 text-sm text-white/85">{skillImportPreview.description}</p>
            <p className="mt-2 text-sm text-emerald-200">
              {t(
                "技能已经安装并加入 Skill Store；你可以在工具/技能页关闭它，或在这里删除后再重新安装。",
                "The skill is installed and listed in Skill Store. Disable it in Tools/Skills, or remove and reinstall it here.",
              )}
            </p>
            <p className="mt-2 text-white/55">
              {t("支持平台", "Supported platforms")}: {skillImportPreview.supported_os.join(", ")} / {skillImportPreview.supported_arch.join(", ")}
            </p>
            <p className="mt-1 font-mono text-[10px] text-white/40">
              {t("验收回执", "Verified receipt")}: {skillImportPreview.receipt_digest.slice(0, 16)}…
            </p>
          </div>
        ) : null}
      </div>
    </div>
  );
}
