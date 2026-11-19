import type { SkillPolicyReview } from "../types/api";
type Translate = (zh: string, en: string) => string;
export function SkillPolicyReviewPanel({ t, reviews, loading, onDecision }: {
  t: Translate; reviews: SkillPolicyReview[]; loading: boolean;
  onDecision?: (review: SkillPolicyReview, decision: "approve" | "revoke") => unknown;
}) {
  const labels: Array<[keyof SkillPolicyReview["permissions"], string, string]> = [
    ["filesystem_read", "读取文件", "Read files"], ["filesystem_write", "修改文件", "Modify files"],
    ["network", "访问网络", "Access the network"], ["subprocess", "启动程序", "Start programs"],
    ["package_install", "安装依赖", "Install dependencies"], ["privilege_escalation", "申请系统权限", "Request system privileges"],
    ["external_publish", "向外部发布", "Publish externally"], ["llm_gateway", "使用模型服务", "Use model services"],
  ];
  return <div className="mt-4 space-y-3">{reviews.map((review) => {
    const enabled = review.admission_state === "enabled";
    const permissions = labels.filter(([key]) => review.permissions[key] === true).map(([, zh, en]) => t(zh, en));
    if (review.permissions.credential_refs?.length) permissions.push(t(`使用已保存的凭据（${review.permissions.credential_refs.length} 项）`, `Use saved credentials (${review.permissions.credential_refs.length})`));
    return <section key={review.skill_name} className="rounded-xl theme-panel-soft p-4">
      <div className="flex flex-wrap justify-between gap-2"><h4 className="font-medium">{review.skill_name} <span className="text-xs opacity-60">v{review.package_version}</span></h4>
        <span className="text-xs opacity-70">{enabled ? t("已授权并开启", "Approved and enabled") : review.admission_state === "installed_disabled" ? t("已授权，未开启", "Approved, disabled") : t("已安装，等待授权", "Installed, awaiting approval")}</span></div>
      <p className="mt-2 text-sm opacity-75">{t("该技能申请：", "This skill requests:")} {permissions.length ? permissions.join(t("、", ", ")) : t("无需额外权限", "No additional permissions")}</p>
      {review.risk_level === "high" && <p className="mt-2 text-sm text-amber-600 dark:text-amber-300">{t("包含修改或外部操作，执行时仍需要确认。", "Includes changes or external actions. Confirmation is still required at execution.")}</p>}
      <details className="mt-2 text-xs opacity-60"><summary>{t("查看能力与版本依据", "Capabilities and version details")}</summary><ul className="mt-2">{review.capabilities.map((capability) => <li key={`${capability.name}:${capability.action}`}>{capability.name}{capability.action ? ` / ${capability.action}` : ""}</li>)}</ul><p className="mt-2 break-all">{review.manifest_digest}</p></details>
      <div className="mt-3 flex flex-wrap gap-2">{!enabled && <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={loading || !onDecision} onClick={() => onDecision?.(review, "approve")}>{t("批准权限并开启", "Approve permissions and enable")}</button>}
      {review.admission_state !== "awaiting_policy_approval" && <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={loading || !onDecision} onClick={() => onDecision?.(review, "revoke")}>{t("撤销授权", "Revoke approval")}</button>}</div>
    </section>;
  })}</div>;
}
