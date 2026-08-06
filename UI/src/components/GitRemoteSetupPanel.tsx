import { useCallback, useEffect, useState } from "react";
import { CheckCircle2, Github, KeyRound, Loader2, RefreshCw, ShieldCheck, Trash2 } from "lucide-react";

import { useUiDialog } from "./UiDialogProvider";
import type { ApiResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface GitConnectionProfile {
  id: string;
  forge_kind: string;
  git_host: string;
  api_host: string;
  allowed_owners: string[];
  allowed_repositories: string[];
  git_username: string;
  auth_scheme: string;
  git_credential_ref: string;
  api_credential_ref: string;
}

export interface GitConnectionSettings {
  schema_version: number;
  revision: number;
  editable: boolean;
  profiles: GitConnectionProfile[];
  credentials: Array<{ name: "github_git_token" | "github_api_token"; configured: boolean; managed_by_environment: boolean }>;
}

export interface GitRemoteSetupPanelProps {
  apiFetch: ApiFetch;
  t: Translate;
  canManage: boolean;
}

const credentialCopy = {
  github_git_token: {
    title: ["Git 推送凭据", "Git push credential"],
    purpose: [
      "用于读取私有仓库和推送分支。建议只授予目标仓库的 Contents 读写权限。",
      "Used for private repository reads and branch pushes. Grant Contents read/write only for the target repositories.",
    ],
  },
  github_api_token: {
    title: ["Pull Request 凭据", "Pull request credential"],
    purpose: [
      "用于创建和查看 Pull Request、检查运行结果。建议只授予 Pull requests 读写和 Checks/Commit status 读取权限。",
      "Used to create and inspect pull requests and checks. Grant Pull requests read/write plus Checks/Commit status read access.",
    ],
  },
} as const;

export function gitConnectionErrorMessage(code: string, t: Translate): string {
  const messages: Record<string, [string, string]> = {
    git_connection_revision_conflict: ["设置已被其他页面更新，请刷新后再保存。", "Settings changed in another page. Refresh before saving again."],
    git_connection_allowlist_required: ["请至少填写一个账号/组织和一个仓库。", "Add at least one owner/organization and one repository."],
    git_connection_provider_unsupported: ["当前只支持 github.com。", "Only github.com is supported right now."],
    git_credential_write_failed: ["凭据未保存，请检查内容后重试。", "The credential was not saved. Check it and try again."],
    git_credential_delete_failed: ["凭据未删除，请稍后重试。", "The credential was not removed. Try again shortly."],
    git_credential_managed_by_environment: ["这个凭据由环境变量管理，请在服务环境中更新并重启。", "This credential is managed by an environment variable. Update the service environment and restart."],
    git_admin_required: ["只有管理员可以修改远端连接。", "Only an administrator can change remote connections."],
  };
  const message = messages[code];
  return message ? t(message[0], message[1]) : t("操作没有完成，请刷新后重试。", "The change was not completed. Refresh and try again.");
}

function splitList(value: string): string[] {
  return [...new Set(value.split(/[\s,，]+/).map((item) => item.trim()).filter(Boolean))];
}

export function GitRemoteSetupPanel({ apiFetch, t, canManage }: GitRemoteSetupPanelProps) {
  const { confirm: showConfirm } = useUiDialog();
  const [data, setData] = useState<GitConnectionSettings | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [connectionId, setConnectionId] = useState("github-main");
  const [owners, setOwners] = useState("");
  const [repositories, setRepositories] = useState("");
  const [tokens, setTokens] = useState<Record<string, string>>({});

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const response = await apiFetch("/v1/git/connections");
      const body = (await response.json()) as ApiResponse<GitConnectionSettings>;
      if (!response.ok || !body.ok || !body.data) throw new Error(body.error || "git_connection_store_unavailable");
      setData(body.data);
    } catch (caught) {
      setError(gitConnectionErrorMessage(caught instanceof Error ? caught.message : "unknown", t));
    } finally {
      setLoading(false);
    }
  }, [apiFetch, t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const saveConnection = async () => {
    if (!data) return;
    const allowedOwners = splitList(owners);
    const allowedRepositories = splitList(repositories);
    if (!connectionId.trim() || allowedOwners.length === 0 || allowedRepositories.length === 0) {
      setError(t("请填写连接名称、账号/组织和仓库。", "Enter a connection name, owner/organization, and repository."));
      return;
    }
    setSaving("connection");
    setError(null);
    setMessage(null);
    try {
      const response = await apiFetch("/v1/git/connections", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          expected_revision: data.revision,
          id: connectionId,
          allowed_owners: allowedOwners,
          allowed_repositories: allowedRepositories,
        }),
      });
      const body = (await response.json()) as ApiResponse<GitConnectionSettings>;
      if (!response.ok || !body.ok || !body.data) throw new Error(body.error || "git_connection_write_failed");
      setData(body.data);
      setOwners("");
      setRepositories("");
      setMessage(t("GitHub 连接范围已保存。", "GitHub connection scope saved."));
    } catch (caught) {
      setError(gitConnectionErrorMessage(caught instanceof Error ? caught.message : "unknown", t));
    } finally {
      setSaving(null);
    }
  };

  const removeConnection = async (profile: GitConnectionProfile) => {
    if (!data) return;
    const confirmed = await showConfirm({
      title: t("删除这个连接？", "Remove this connection?"),
      message: t("删除后不会删除凭据；重新添加相同范围即可恢复使用。", "Credentials are kept. Add the same scope again to restore access."),
      confirmLabel: t("删除连接", "Remove connection"),
      cancelLabel: t("保留", "Keep it"),
      tone: "danger",
    });
    if (!confirmed) return;
    setSaving(`profile:${profile.id}`);
    setError(null);
    try {
      const response = await apiFetch(`/v1/git/connections/${encodeURIComponent(profile.id)}?expected_revision=${data.revision}`, { method: "DELETE" });
      const body = (await response.json()) as ApiResponse<GitConnectionSettings>;
      if (!response.ok || !body.ok || !body.data) throw new Error(body.error || "git_connection_delete_failed");
      setData(body.data);
      setMessage(t("连接已删除，凭据仍然保留。", "Connection removed; credentials were kept."));
    } catch (caught) {
      setError(gitConnectionErrorMessage(caught instanceof Error ? caught.message : "unknown", t));
    } finally {
      setSaving(null);
    }
  };

  const saveCredential = async (name: keyof typeof credentialCopy) => {
    const value = tokens[name]?.trim() ?? "";
    if (!value) {
      setError(t("请先粘贴新的凭据。", "Paste the new credential first."));
      return;
    }
    setSaving(name);
    setError(null);
    setMessage(null);
    try {
      const response = await apiFetch(`/v1/git/credentials/${name}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ value }),
      });
      const body = (await response.json()) as ApiResponse<GitConnectionSettings>;
      if (!response.ok || !body.ok || !body.data) throw new Error(body.error || "git_credential_write_failed");
      setData(body.data);
      setTokens((current) => ({ ...current, [name]: "" }));
      setMessage(t("凭据已安全保存，页面不会再显示原值。", "Credential saved securely. Its value will not be shown again."));
    } catch (caught) {
      setError(gitConnectionErrorMessage(caught instanceof Error ? caught.message : "unknown", t));
    } finally {
      setSaving(null);
    }
  };

  const removeCredential = async (name: keyof typeof credentialCopy) => {
    const confirmed = await showConfirm({
      title: t("删除这个凭据？", "Remove this credential?"),
      message: t("使用它的私有仓库读取、推送或 PR 操作会停止；之后可以重新设置。", "Private reads, pushes, or PR actions using it will stop. You can set it again later."),
      confirmLabel: t("删除凭据", "Remove credential"),
      cancelLabel: t("取消", "Cancel"),
      tone: "danger",
    });
    if (!confirmed) return;
    setSaving(name);
    setError(null);
    try {
      const response = await apiFetch(`/v1/git/credentials/${name}`, { method: "DELETE" });
      const body = (await response.json()) as ApiResponse<GitConnectionSettings>;
      if (!response.ok || !body.ok || !body.data) throw new Error(body.error || "git_credential_delete_failed");
      setData(body.data);
      setMessage(t("凭据已删除。", "Credential removed."));
    } catch (caught) {
      setError(gitConnectionErrorMessage(caught instanceof Error ? caught.message : "unknown", t));
    } finally {
      setSaving(null);
    }
  };

  const credentialStatus = (name: string) => data?.credentials.find((item) => item.name === name)?.configured ?? false;
  const credentialEnvironmentManaged = (name: string) => data?.credentials.find((item) => item.name === name)?.managed_by_environment ?? false;

  return (
    <article className="theme-panel overflow-hidden rounded-xl shadow-lg shadow-black/10">
      <div className="flex flex-col gap-3 border-b border-white/10 px-5 py-4 sm:flex-row sm:items-start sm:justify-between">
        <div className="flex gap-3">
          <span className="mt-0.5 rounded-lg bg-white/5 p-2 text-white/75"><Github className="h-5 w-5" /></span>
          <div>
            <h2 className="text-base font-semibold">{t("GitHub 远端交付", "GitHub remote delivery")}</h2>
            <p className="mt-1 max-w-3xl text-sm text-white/55">
              {t("先限定允许访问的账号和仓库，再分别设置推送与 Pull Request 凭据。安装远端 Git 技能后，读取、推送和 PR 仍会按各自开关与确认规则执行。", "First limit allowed owners and repositories, then set separate push and pull-request credentials. Remote Git skills still follow their own switches and confirmation rules after installation.")}
            </p>
          </div>
        </div>
        <button type="button" onClick={() => void refresh()} disabled={loading} className="theme-topbar-btn h-9 shrink-0 px-3 disabled:opacity-50">
          {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
          {t("刷新状态", "Refresh")}
        </button>
      </div>

      <div className="space-y-5 px-5 py-5">
        {error ? <p className="rounded-lg border border-red-500/25 bg-red-500/10 px-3 py-2 text-sm text-red-200">{error}</p> : null}
        {message ? <p className="rounded-lg border border-emerald-500/25 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-100">{message}</p> : null}
        {!canManage ? <p className="rounded-lg border border-white/10 bg-white/[0.03] px-3 py-2 text-sm text-white/60">{t("你可以查看连接状态；只有管理员可以修改。", "You can view connection status; only administrators can change it.")}</p> : null}

        <section>
          <div className="flex items-center gap-2"><ShieldCheck className="h-4 w-4 text-sky-300" /><h3 className="text-sm font-semibold">{t("允许的仓库", "Allowed repositories")}</h3></div>
          <p className="mt-1 text-xs text-white/50">{t("普通任务不能临时填写其他网址。当前版本只会连接 github.com 和 api.github.com。", "Tasks cannot supply another URL at runtime. This version connects only to github.com and api.github.com.")}</p>
          <div className="mt-3 space-y-2">
            {(data?.profiles ?? []).map((profile) => (
              <div key={profile.id} className="flex flex-col gap-2 rounded-lg bg-white/[0.035] px-3 py-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="min-w-0">
                  <p className="font-medium text-white/85">{profile.id}</p>
                  <p className="mt-1 break-words text-xs text-white/50">{profile.allowed_owners.join(", ")} / {profile.allowed_repositories.join(", ")}</p>
                </div>
                {canManage ? <button type="button" onClick={() => void removeConnection(profile)} disabled={saving !== null} className="inline-flex items-center gap-1.5 self-start rounded-lg px-2.5 py-1.5 text-xs text-red-200 hover:bg-red-500/10 disabled:opacity-50"><Trash2 className="h-3.5 w-3.5" />{t("删除", "Remove")}</button> : null}
              </div>
            ))}
            {!loading && data && data.profiles.length === 0 ? <p className="rounded-lg bg-white/[0.025] px-3 py-4 text-sm text-white/45">{t("还没有允许的仓库。先添加一个连接范围。", "No repository is allowed yet. Add a connection scope first.")}</p> : null}
          </div>
          {canManage ? (
            <div className="mt-3 grid gap-3 rounded-lg border border-white/10 p-3 md:grid-cols-3">
              <label className="text-xs text-white/55">{t("连接名称", "Connection name")}<input className="theme-input mt-1 w-full" value={connectionId} onChange={(event) => setConnectionId(event.target.value)} placeholder="github-main" /></label>
              <label className="text-xs text-white/55">{t("账号或组织", "Owner or organization")}<input className="theme-input mt-1 w-full" value={owners} onChange={(event) => setOwners(event.target.value)} placeholder="ExampleOwner" /></label>
              <label className="text-xs text-white/55">{t("仓库名称", "Repository name")}<input className="theme-input mt-1 w-full" value={repositories} onChange={(event) => setRepositories(event.target.value)} placeholder="repository" /></label>
              <div className="md:col-span-3 flex justify-end"><button type="button" onClick={() => void saveConnection()} disabled={!data || saving !== null} className="theme-primary-btn disabled:opacity-50">{saving === "connection" ? t("保存中", "Saving") : t("保存连接范围", "Save connection scope")}</button></div>
            </div>
          ) : null}
        </section>

        <section>
          <div className="flex items-center gap-2"><KeyRound className="h-4 w-4 text-amber-300" /><h3 className="text-sm font-semibold">{t("凭据状态", "Credential status")}</h3></div>
          <p className="mt-1 text-xs text-white/50">{t("凭据只写入本机私有存储，不写入仓库、配置文件或浏览器；保存后无法在页面读回。", "Credentials are write-only in private local storage, never the repository, tracked config, or browser. Their values cannot be read back after saving.")}</p>
          <div className="mt-3 grid gap-3 lg:grid-cols-2">
            {(Object.keys(credentialCopy) as Array<keyof typeof credentialCopy>).map((name) => {
              const copy = credentialCopy[name];
              const configured = credentialStatus(name);
              const environmentManaged = credentialEnvironmentManaged(name);
              return (
                <div key={name} className="rounded-lg border border-white/10 bg-white/[0.025] p-4">
                  <div className="flex items-start justify-between gap-3"><div><h4 className="text-sm font-medium">{t(copy.title[0], copy.title[1])}</h4><p className="mt-1 text-xs leading-5 text-white/50">{t(copy.purpose[0], copy.purpose[1])}</p></div><span className={`inline-flex shrink-0 items-center gap-1 rounded-full px-2 py-1 text-[11px] ${configured ? "bg-emerald-500/10 text-emerald-200" : "bg-amber-500/10 text-amber-200"}`}>{configured ? <CheckCircle2 className="h-3.5 w-3.5" /> : null}{configured ? t("已配置", "Configured") : t("未配置", "Not configured")}</span></div>
                  {environmentManaged ? <p className="mt-3 rounded-lg bg-white/[0.035] px-3 py-2 text-xs text-white/55">{t("由服务环境变量管理；请在部署环境中轮转。", "Managed by the service environment; rotate it in the deployment environment.")}</p> : null}
                  {canManage && !environmentManaged ? <div className="mt-3 flex flex-col gap-2 sm:flex-row"><input type="password" autoComplete="new-password" className="theme-input min-w-0 flex-1" value={tokens[name] ?? ""} onChange={(event) => setTokens((current) => ({ ...current, [name]: event.target.value }))} placeholder={configured ? t("粘贴新值可替换", "Paste a new value to replace") : t("粘贴凭据", "Paste credential")} /><button type="button" onClick={() => void saveCredential(name)} disabled={saving !== null} className="theme-primary-btn shrink-0 disabled:opacity-50">{configured ? t("替换", "Replace") : t("保存", "Save")}</button>{configured ? <button type="button" onClick={() => void removeCredential(name)} disabled={saving !== null} className="rounded-lg border border-red-400/20 px-3 py-2 text-xs text-red-200 hover:bg-red-500/10 disabled:opacity-50">{t("删除", "Delete")}</button> : null}</div> : null}
                </div>
              );
            })}
          </div>
        </section>

        <details className="rounded-lg border border-white/10 bg-white/[0.02] px-3 py-2 text-xs text-white/50"><summary className="cursor-pointer text-white/65">{t("安全工作方式", "How safety works")}</summary><ul className="mt-2 list-disc space-y-1 pl-5"><li>{t("推送固定为审批时的完整提交 SHA，远端状态变化会停止操作。", "Pushes use the full approved commit SHA and stop if remote state changes.")}</li><li>{t("强推、删分支、标签发布和多个 refspec 均不可用。", "Force push, branch deletion, tag publication, and multiple refspecs are unavailable.")}</li><li>{t("推送和创建 PR 每次都需要单独确认；读取和异常对账不会发布内容。", "Every push and PR creation needs separate confirmation; reads and reconciliation do not publish content.")}</li></ul></details>
      </div>
    </article>
  );
}
