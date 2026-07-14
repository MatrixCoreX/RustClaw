import { Loader2, RefreshCw } from "lucide-react";

import type { ChannelName } from "../types/api";

type TaskSubmitKind = "ask" | "run_skill";
type Translate = (zh: string, en: string) => string;
type TranslateSlash = (text: string) => string;

export interface ManualTaskSubmitPanelProps {
  t: Translate;
  tSlash: TranslateSlash;
  interactionKind: TaskSubmitKind;
  interactionChannel: ChannelName;
  interactionAdapter: string;
  interactionExternalUserId: string;
  interactionExternalChatId: string;
  interactionRole: string;
  localContextLoading: boolean;
  localContextError: string | null;
  interactionAskText: string;
  interactionSkillName: string;
  interactionSkillArgs: string;
  interactionLoading: boolean;
  interactionSubmittedTaskId: string | null;
  trackingTaskId: string | null;
  interactionError: string | null;
  onInteractionKindChange: (value: TaskSubmitKind) => void;
  onInteractionChannelChange: (value: ChannelName) => void;
  onInteractionAdapterChange: (value: string) => void;
  onInteractionExternalUserIdChange: (value: string) => void;
  onInteractionExternalChatIdChange: (value: string) => void;
  onInteractionAskTextChange: (value: string) => void;
  onInteractionSkillNameChange: (value: string) => void;
  onInteractionSkillArgsChange: (value: string) => void;
  onSubmitInteractionTask: () => unknown | Promise<unknown>;
}

export function ManualTaskSubmitPanel({
  t,
  tSlash,
  interactionKind,
  interactionChannel,
  interactionAdapter,
  interactionExternalUserId,
  interactionExternalChatId,
  interactionRole,
  localContextLoading,
  localContextError,
  interactionAskText,
  interactionSkillName,
  interactionSkillArgs,
  interactionLoading,
  interactionSubmittedTaskId,
  trackingTaskId,
  interactionError,
  onInteractionKindChange,
  onInteractionChannelChange,
  onInteractionAdapterChange,
  onInteractionExternalUserIdChange,
  onInteractionExternalChatIdChange,
  onInteractionAskTextChange,
  onInteractionSkillNameChange,
  onInteractionSkillArgsChange,
  onSubmitInteractionTask,
}: ManualTaskSubmitPanelProps) {
  return (
    <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
      <h3 className="mb-4 text-lg font-semibold">{t("手动提交一条任务", "Submit a task manually")}</h3>
      <div className="grid gap-4 md:grid-cols-2">
        <label className="space-y-2">
          <span className="text-xs uppercase tracking-widest text-white/50">{t("任务类型", "Task type")}</span>
          <select
            className="theme-input"
            value={interactionKind}
            onChange={(event) => onInteractionKindChange(event.target.value as TaskSubmitKind)}
          >
            <option value="ask">ask</option>
            <option value="run_skill">run_skill</option>
          </select>
        </label>
        <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-2 text-sm">
          <p className="text-white/80">{t("当前本地身份", "Current local identity")}</p>
          <p className="mt-1 text-xs text-white/50">role={interactionRole}</p>
          {localContextLoading ? <p className="mt-1 text-xs text-white/50">{tSlash("加载中... / Loading...")}</p> : null}
          {localContextError ? <p className="mt-1 text-xs text-red-300">{tSlash("上下文错误 / Context error")}: {localContextError}</p> : null}
        </div>
      </div>
      <div className="mt-4 grid gap-4 md:grid-cols-2">
        <label className="space-y-2">
          <span className="text-xs uppercase tracking-widest text-white/50">{t("发送渠道", "Channel")}</span>
          <select
            className="theme-input"
            value={interactionChannel}
            onChange={(event) => onInteractionChannelChange(event.target.value as ChannelName)}
          >
            <option value="ui">ui</option>
            <option value="telegram">telegram</option>
            <option value="whatsapp">whatsapp</option>
            <option value="feishu">feishu</option>
            <option value="lark">lark</option>
          </select>
        </label>
        <label className="space-y-2">
          <span className="text-xs uppercase tracking-widest text-white/50">{t("适配器名（可选）", "Adapter name (optional)")}</span>
          <input
            className="theme-input"
            value={interactionAdapter}
            onChange={(event) => onInteractionAdapterChange(event.target.value)}
            placeholder="telegram_bot / whatsapp_cloud / whatsapp_web / feishu"
          />
        </label>
        <label className="space-y-2">
          <span className="text-xs uppercase tracking-widest text-white/50">{t("外部用户 ID（可选）", "External user ID (optional)")}</span>
          <input
            className="theme-input"
            value={interactionExternalUserId}
            onChange={(event) => onInteractionExternalUserIdChange(event.target.value)}
            placeholder={t("外部用户 ID（跨平台）", "External user id")}
          />
        </label>
        <label className="space-y-2">
          <span className="text-xs uppercase tracking-widest text-white/50">{t("外部会话 ID（可选）", "External chat ID (optional)")}</span>
          <input
            className="theme-input"
            value={interactionExternalChatId}
            onChange={(event) => onInteractionExternalChatIdChange(event.target.value)}
            placeholder={t("外部会话 ID（WhatsApp 建议填写）", "External chat id")}
          />
        </label>
      </div>

      {interactionKind === "ask" ? (
        <div className="mt-4 space-y-4">
          <label className="block space-y-2">
            <span className="text-xs uppercase tracking-widest text-white/50">ask.text</span>
            <textarea
              className="theme-input min-h-28"
              value={interactionAskText}
              onChange={(event) => onInteractionAskTextChange(event.target.value)}
              placeholder={t("例如：请汇报当前系统状态", "For example: Please summarize the current system status")}
            />
          </label>
        </div>
      ) : (
        <div className="mt-4 space-y-4">
          <label className="block space-y-2">
            <span className="text-xs uppercase tracking-widest text-white/50">run_skill.skill_name</span>
            <input
              className="theme-input"
              value={interactionSkillName}
              onChange={(event) => onInteractionSkillNameChange(event.target.value)}
            />
          </label>
          <label className="block space-y-2">
            <span className="text-xs uppercase tracking-widest text-white/50">{tSlash("run_skill.args (JSON 或字符串 / string)")}</span>
            <textarea
              className="theme-input min-h-28"
              value={interactionSkillArgs}
              onChange={(event) => onInteractionSkillArgsChange(event.target.value)}
            />
          </label>
        </div>
      )}

      <div className="mt-4 flex flex-wrap items-center gap-3">
        <button
          type="button"
          onClick={() => void onSubmitInteractionTask()}
          disabled={interactionLoading}
          className="theme-accent-btn"
        >
          {interactionLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
          {t("提交任务", "Submit task")}
        </button>

        {interactionSubmittedTaskId ? (
          <span className="text-xs text-emerald-300">
            {tSlash("已提交 / Submitted")}
            {trackingTaskId ? ` ${tSlash("（自动跟踪中 / auto tracking）")}` : ""}
          </span>
        ) : null}
      </div>

      {interactionError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {tSlash("提交失败 / Submit failed")}: {interactionError}
        </p>
      ) : null}
    </section>
  );
}
