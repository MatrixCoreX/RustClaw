import { Loader2, RefreshCw } from "lucide-react";

import type { ChannelName } from "../types/api";
import { TaskIdCopyButton } from "./TaskIdCopyButton";

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
  interactionIndependentWorkspace: boolean;
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
  onInteractionIndependentWorkspaceChange: (value: boolean) => void;
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
  interactionIndependentWorkspace,
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
  onInteractionIndependentWorkspaceChange,
  onInteractionSkillNameChange,
  onInteractionSkillArgsChange,
  onSubmitInteractionTask,
}: ManualTaskSubmitPanelProps) {
  return (
    <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
      <h3 className="text-lg font-semibold">{t("手动提交一条任务", "Submit a task manually")}</h3>
      <p className="mt-2 max-w-3xl text-sm leading-6 text-white/55">
        {t(
          "这里会直接创建一条后端任务，适合测试指定技能、复现问题或排查执行过程。日常对话建议使用 Agent 页面。",
          "This creates a backend task directly. Use it to test a specific skill, reproduce an issue, or inspect execution. Use the Agent page for everyday conversations.",
        )}
      </p>
      <div className="mt-3 flex flex-wrap gap-x-5 gap-y-1 text-xs text-white/50">
        <span>{t("1. 选择任务类型并填写内容", "1. Choose a task type and enter its content")}</span>
        <span>{t("2. 提交后在上方查看进度", "2. Track progress above after submitting")}</span>
        <span>{t("3. 用任务 ID 查询完整结果", "3. Use the task ID to query the full result")}</span>
      </div>
      <div className="mt-5 grid gap-4 md:grid-cols-2">
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
          <label className="flex items-start gap-3 rounded-xl border border-white/15 bg-black/10 px-3 py-3">
            <input
              type="checkbox"
              checked={interactionIndependentWorkspace}
              onChange={(event) => onInteractionIndependentWorkspaceChange(event.target.checked)}
              className="mt-0.5 h-4 w-4"
            />
            <span>
              <span className="block text-sm font-medium">{t("使用独立工作区", "Use an independent workspace")}</span>
              <span className="mt-1 block text-xs leading-5 text-white/55">
                {t(
                  "适合修改代码：不会直接覆盖当前目录，完成后可先检查改动再决定是否合并。",
                  "Recommended for code changes: it keeps the current folder untouched so changes can be reviewed before applying.",
                )}
              </span>
            </span>
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

      {interactionSubmittedTaskId ? (
        <div className="mt-3 rounded-lg border border-emerald-400/20 bg-emerald-500/10 px-3 py-3">
          <p className="text-xs text-emerald-100/70">
            {t("任务 ID 是这次执行的查询编号，不是任务内容或访问密钥。", "The task ID is the lookup reference for this run, not task content or an access key.")}
          </p>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <code className="min-w-0 flex-1 select-all break-all text-xs text-emerald-50">{interactionSubmittedTaskId}</code>
            <TaskIdCopyButton taskId={interactionSubmittedTaskId} t={t} />
          </div>
        </div>
      ) : null}

      {interactionError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {tSlash("提交失败 / Submit failed")}: {interactionError}
        </p>
      ) : null}
    </section>
  );
}
