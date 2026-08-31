import {
  Check,
  Copy,
  KeyRound,
  Loader2,
  ShieldAlert,
  ShieldCheck,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import type { NniOwnerAuthorizationChallenge } from "../hooks/useNniRuntime";
import {
  nniPrivateKeyOperationsAllowed,
  normalizeNniOwnerSignature,
  validateNniOwnerPrivateKey,
  validateNniOwnerPublicKey,
  type NniOwnerKeyPair,
} from "../lib/nni-owner-public-key";
import { useUiDialog } from "./UiDialogProvider";

type Translate = (zh: string, en: string) => string;
export type NniAssetAccountDialogMode = "create" | "bind" | "replace" | "recover";
type AuthorizationMode = "private_key" | "external_signature";

export interface NniAssetAccountDialogProps {
  mode: NniAssetAccountDialogMode;
  t: Translate;
  chipReady: boolean;
  remoteNodeReady: boolean;
  loading: boolean;
  actionError: string | null;
  generatedKeyPair: NniOwnerKeyPair | null;
  authorizationChallenge: NniOwnerAuthorizationChallenge | null;
  privateKeyCopied: boolean;
  onClose: () => void;
  onGenerate: () => unknown | Promise<unknown>;
  onDiscardGenerated: () => void;
  onCopyGeneratedPrivateKey: () => unknown | Promise<unknown>;
  onAuthorizeWithPrivateKey: (privateKey: string) => unknown | Promise<unknown>;
  onRecover: (privateKey: string) => unknown | Promise<unknown>;
  onStartExternalAuthorization: (publicKey: string) => unknown | Promise<unknown>;
  onCompleteExternalAuthorization: (signature: string) => unknown | Promise<unknown>;
  onCancelExternalAuthorization: () => void;
  onCopyText: (value: string) => void;
}

export function NniAssetAccountDialog({
  mode,
  t,
  chipReady,
  remoteNodeReady,
  loading,
  actionError,
  generatedKeyPair,
  authorizationChallenge,
  privateKeyCopied,
  onClose,
  onGenerate,
  onDiscardGenerated,
  onCopyGeneratedPrivateKey,
  onAuthorizeWithPrivateKey,
  onRecover,
  onStartExternalAuthorization,
  onCompleteExternalAuthorization,
  onCancelExternalAuthorization,
  onCopyText,
}: NniAssetAccountDialogProps) {
  const { confirm } = useUiDialog();
  const privateKeyOperationsAllowed = nniPrivateKeyOperationsAllowed();
  const [authorizationMode, setAuthorizationMode] = useState<AuthorizationMode>(
    privateKeyOperationsAllowed ? "private_key" : "external_signature",
  );
  const [privateKey, setPrivateKey] = useState("");
  const [publicKey, setPublicKey] = useState("");
  const [externalSignature, setExternalSignature] = useState("");
  const closeButtonRef = useRef<HTMLButtonElement | null>(null);
  const privateKeyValidation = useMemo(() => validateNniOwnerPrivateKey(privateKey), [privateKey]);
  const publicKeyValidation = useMemo(() => validateNniOwnerPublicKey(publicKey), [publicKey]);
  const externalSignatureValid = Boolean(normalizeNniOwnerSignature(externalSignature));

  const title = mode === "create"
    ? t("创建资产账户", "Create asset account")
    : mode === "replace"
      ? t("更换资产账户", "Replace asset account")
      : mode === "recover"
        ? t("换机恢复", "Recover on this device")
        : t("重新绑定资产账户", "Rebind asset account");

  const close = () => {
    if (loading) return;
    if (authorizationChallenge) onCancelExternalAuthorization();
    setPrivateKey("");
    setPublicKey("");
    setExternalSignature("");
    onClose();
  };

  useEffect(() => {
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const frame = window.requestAnimationFrame(() => closeButtonRef.current?.focus());
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !loading) close();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("keydown", onKeyDown);
      document.body.style.overflow = previousOverflow;
    };
    // The dialog owns its close behavior for the lifetime of one opened flow.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loading]);

  useEffect(() => {
    setExternalSignature("");
  }, [authorizationChallenge?.taskId]);

  useEffect(() => {
    if (privateKeyOperationsAllowed || authorizationMode !== "private_key") return;
    if (authorizationChallenge) onCancelExternalAuthorization();
    setAuthorizationMode("external_signature");
    setPrivateKey("");
  }, [authorizationChallenge, authorizationMode, onCancelExternalAuthorization, privateKeyOperationsAllowed]);

  const selectAuthorizationMode = (nextMode: AuthorizationMode) => {
    if (nextMode === "private_key" && !privateKeyOperationsAllowed) return;
    if (nextMode === authorizationMode) return;
    if (authorizationChallenge) onCancelExternalAuthorization();
    setAuthorizationMode(nextMode);
    setExternalSignature("");
  };

  const submitPrivateKeyAuthorization = async () => {
    if (!privateKeyValidation.ok) return;
    const result = await Promise.resolve(onAuthorizeWithPrivateKey(privateKeyValidation.normalized));
    setPrivateKey("");
    if (result) onClose();
  };

  const submitRecovery = async () => {
    if (!privateKey.trim()) return;
    const result = await Promise.resolve(onRecover(privateKey.trim()));
    setPrivateKey("");
    if (result) onClose();
  };

  const joinCreatedAccount = async () => {
    if (!generatedKeyPair) return;
    const saved = await confirm({
      title: t("确认已保存私钥", "Confirm private-key backup"),
      message: t(
        "请确认你已经把私钥安全地离线保存。加入后页面将不再显示这把私钥；私钥丢失会影响换机恢复和资产控制。",
        "Confirm that you saved the private key securely offline. It will no longer be shown after joining. Losing it affects device recovery and asset control.",
      ),
      confirmLabel: t("已保存，加入", "Saved, join now"),
    });
    if (!saved) return;
    const result = await Promise.resolve(onAuthorizeWithPrivateKey(generatedKeyPair.private_key));
    if (result) onClose();
  };

  const startExternalAuthorization = async () => {
    if (!publicKeyValidation.ok) return;
    await Promise.resolve(onStartExternalAuthorization(publicKeyValidation.normalized));
  };

  const completeExternalAuthorization = async () => {
    if (!externalSignatureValid) return;
    const result = await Promise.resolve(onCompleteExternalAuthorization(externalSignature));
    if (result) onClose();
  };

  return (
    <div
      className="fixed inset-0 z-[110] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) close();
      }}
      data-nni-asset-account-dialog={mode}
    >
      <section
        role="dialog"
        aria-modal="true"
        aria-labelledby="nni-asset-account-dialog-title"
        className="theme-card max-h-[min(88vh,760px)] w-full max-w-xl overflow-y-auto border p-0 shadow-2xl"
      >
        <header className="sticky top-0 z-10 flex items-start gap-3 border-b border-white/10 bg-[var(--theme-card)] px-5 py-4">
          <span className="mt-0.5 text-sky-300"><KeyRound className="h-5 w-5" /></span>
          <div className="min-w-0 flex-1">
            <h2 id="nni-asset-account-dialog-title" className="text-base font-semibold text-white">{title}</h2>
            <p className="mt-1 text-sm leading-6 text-white/55">
              {mode === "create"
                ? t("这组密钥代表你的资产账户，私钥只显示一次。", "This key pair represents your asset account. The private key is shown once.")
                : mode === "recover"
                  ? t("使用原资产私钥把账户恢复到当前硬件设备。", "Use the original asset private key to recover the account on this hardware device.")
                  : t("当前硬件和目标资产账户都要完成签名，避免误绑他人的账户。", "Both this hardware device and the target asset account must sign, preventing accidental binding to someone else's account.")}
            </p>
          </div>
          <button
            ref={closeButtonRef}
            type="button"
            className="theme-icon-btn h-8 w-8 shrink-0"
            disabled={loading}
            onClick={close}
            title={t("关闭", "Close")}
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        <div className="grid gap-4 px-5 py-5">
          {actionError ? (
            <p role="alert" className="rounded-md border border-rose-400/30 bg-rose-500/10 px-3 py-2 text-sm leading-6 text-rose-100">
              {actionError}
            </p>
          ) : null}

          {!privateKeyOperationsAllowed ? (
            <div role="alert" className="flex items-start gap-2 rounded-md border border-amber-400/30 bg-amber-500/10 px-3 py-2.5 text-sm leading-6 text-amber-100">
              <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
              <span>{t(
                "当前页面使用非本机 HTTP 连接，私钥输入、显示和签名已禁用。请改用 HTTPS，或在设备本机通过 localhost 操作。",
                "Private-key input, display, and signing are disabled over non-loopback HTTP. Use HTTPS or operate through localhost on this device.",
              )}</span>
            </div>
          ) : null}

          {!privateKeyOperationsAllowed && (mode === "create" || mode === "recover") ? (
            <div className="flex justify-end pt-1">
              <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={loading} onClick={close}>{t("关闭", "Close")}</button>
            </div>
          ) : mode === "create" ? (
            generatedKeyPair ? (
              <>
                <div className="flex items-start gap-2 rounded-md border border-amber-400/30 bg-amber-500/10 px-3 py-2.5 text-sm leading-6 text-amber-100">
                  <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
                  <span>{t("私钥不会保存。请先离线备份，再点击加入。", "The private key is not saved. Back it up offline before joining.")}</span>
                </div>
                <label className="grid gap-1.5 text-xs text-white/60">
                  <span>{t("资产公钥", "Asset public key")}</span>
                  <code className="break-all rounded-md bg-black/20 p-3 text-sm text-white/85">{generatedKeyPair.public_key}</code>
                </label>
                <div className="grid gap-1.5">
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-xs text-white/60">{t("一次性显示的私钥", "One-time private key")}</span>
                    <button type="button" className="theme-secondary-btn px-2 py-1.5 text-xs" data-nni-copy-owner-private-key="true" onClick={() => void onCopyGeneratedPrivateKey()}>
                      {privateKeyCopied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                      {privateKeyCopied ? t("已复制", "Copied") : t("复制私钥", "Copy private key")}
                    </button>
                  </div>
                  <code className="break-all rounded-md bg-black/25 p-3 text-sm text-amber-100">{generatedKeyPair.private_key}</code>
                </div>
                <div className="flex flex-col-reverse gap-2 pt-1 sm:flex-row sm:justify-between">
                  <button type="button" className="theme-secondary-btn px-3 py-2 text-sm text-red-100" data-nni-discard-owner-key-pair="true" disabled={loading} onClick={() => { onDiscardGenerated(); onClose(); }}>
                    {t("放弃这组密钥", "Discard this key pair")}
                  </button>
                  <button
                    type="button"
                    className="theme-accent-btn px-4 py-2 text-sm"
                    disabled={!chipReady || !remoteNodeReady || loading}
                    onClick={() => void joinCreatedAccount()}
                    data-nni-created-owner-join="true"
                  >
                    {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <ShieldCheck className="h-4 w-4" />}
                    {t("加入", "Join")}
                  </button>
                </div>
              </>
            ) : (
              <div className="grid justify-items-center gap-3 py-8 text-center">
                {loading ? <Loader2 className="h-6 w-6 animate-spin text-sky-300" /> : <KeyRound className="h-6 w-6 text-sky-300" />}
                <p className="text-sm text-white/60">
                  {loading ? t("正在创建资产密钥...", "Creating asset keys...") : t("资产密钥尚未创建。", "Asset keys have not been created yet.")}
                </p>
                {!loading ? <button type="button" className="theme-accent-btn px-4 py-2 text-sm" onClick={() => void onGenerate()}>{t("创建密钥", "Create keys")}</button> : null}
              </div>
            )
          ) : mode === "recover" ? (
            <>
              <div
                role="note"
                data-nni-recovery-warning="true"
                className="flex items-start gap-2 rounded-md border border-amber-400/30 bg-amber-500/10 px-3 py-2.5 text-sm leading-6 text-amber-100"
              >
                <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
                <span>
                  {t(
                    "仅当原设备已损坏或无法再使用时，才使用换机恢复。此操作会把资产账户迁移到新设备，并撤销该资产账户当前关联的其他设备授权；如果原设备仍可正常使用，执行后它将无法继续参与 NNI。请确认原设备确实不可恢复后再继续。",
                    "Use device recovery only when the original device is damaged or no longer usable. This moves the asset account to the new device and revokes every other device currently authorized for that account. If the original device still works, it will no longer be able to participate in NNI after recovery. Continue only after confirming that the original device cannot be recovered.",
                  )}
                </span>
              </div>
              <p className="text-sm leading-6 text-white/65">
                {t("新设备必须已经获准加入网络。私钥只用于这次恢复签名，不会保存在浏览器中。", "The new device must already be admitted to the network. The private key is used only for this recovery signature and is not stored in the browser.")}
              </p>
              <label className="grid gap-1.5 text-xs text-white/65">
                <span>{t("原资产私钥", "Original asset private key")}</span>
                <input
                  type="password"
                  autoComplete="one-time-code"
                  autoCapitalize="none"
                  data-1p-ignore="true"
                  data-bwignore="true"
                  data-form-type="other"
                  data-lpignore="true"
                  data-protonpass-ignore="true"
                  spellCheck={false}
                  value={privateKey}
                  onChange={(event) => setPrivateKey(event.target.value)}
                  className="theme-input w-full"
                  placeholder={t("粘贴资产私钥", "Paste the asset private key")}
                />
              </label>
              <div className="flex justify-end gap-2">
                <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={loading} onClick={close}>{t("取消", "Cancel")}</button>
                <button type="button" className="theme-accent-btn px-4 py-2 text-sm" disabled={!privateKey.trim() || loading} onClick={() => void submitRecovery()}>
                  {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <KeyRound className="h-4 w-4" />}
                  {t("签名并恢复", "Sign and recover")}
                </button>
              </div>
            </>
          ) : (
            <>
              <div className="grid grid-cols-2 rounded-md border border-white/10 bg-black/10 p-1" role="tablist" aria-label={t("签名方式", "Signing method")}>
                <button type="button" role="tab" aria-selected={authorizationMode === "private_key"} disabled={!privateKeyOperationsAllowed} className={authorizationMode === "private_key" ? "theme-accent-btn justify-center px-3 py-2 text-sm" : "theme-secondary-btn justify-center border-transparent px-3 py-2 text-sm disabled:opacity-45"} onClick={() => selectAuthorizationMode("private_key")}>
                  {t("输入私钥", "Use private key")}
                </button>
                <button type="button" role="tab" aria-selected={authorizationMode === "external_signature"} className={authorizationMode === "external_signature" ? "theme-accent-btn justify-center px-3 py-2 text-sm" : "theme-secondary-btn justify-center border-transparent px-3 py-2 text-sm"} onClick={() => selectAuthorizationMode("external_signature")}>
                  {t("外部签名", "External signature")}
                </button>
              </div>

              {authorizationMode === "private_key" ? (
                <>
                  <div className="rounded-md border border-sky-400/20 bg-sky-500/10 px-3 py-2 text-xs leading-5 text-sky-100">
                    {t("私钥只在当前浏览器内用于签名，不会发送给本机服务、远程节点或写入存储。", "The private key signs only inside this browser. It is not sent to the local service or remote node and is not stored.")}
                  </div>
                  <label className="grid gap-1.5 text-xs text-white/65">
                    <span>{t("目标资产私钥", "Target asset private key")}</span>
                    <input
                      type="password"
                      autoComplete="one-time-code"
                      autoCapitalize="none"
                      data-1p-ignore="true"
                      data-bwignore="true"
                      data-form-type="other"
                      data-lpignore="true"
                      data-protonpass-ignore="true"
                      spellCheck={false}
                      value={privateKey}
                      onChange={(event) => setPrivateKey(event.target.value)}
                      className="theme-input w-full"
                      placeholder={t("粘贴 K1 资产私钥", "Paste a K1 asset private key")}
                    />
                  </label>
                  {privateKey && !privateKeyValidation.ok ? <p className="text-xs text-rose-200">{t("私钥格式无效，请检查 K1 Base58 密钥和校验和。", "Invalid private key. Check the K1 Base58 key and checksum.")}</p> : null}
                  {privateKeyValidation.ok ? (
                    <div className="rounded-md border border-white/10 bg-black/10 p-3">
                      <p className="text-xs text-white/50">{t("将绑定的资产公钥", "Asset public key to bind")}</p>
                      <code className="mt-1 block break-all text-xs text-white/75">{privateKeyValidation.publicKey}</code>
                    </div>
                  ) : null}
                  <div className="flex justify-end gap-2">
                    <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={loading} onClick={close}>{t("取消", "Cancel")}</button>
                    <button type="button" className="theme-accent-btn px-4 py-2 text-sm" disabled={!privateKeyValidation.ok || !chipReady || !remoteNodeReady || loading} onClick={() => void submitPrivateKeyAuthorization()}>
                      {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <ShieldCheck className="h-4 w-4" />}
                      {mode === "replace" ? t("确认更换", "Replace") : t("重新绑定", "Rebind")}
                    </button>
                  </div>
                </>
              ) : authorizationChallenge ? (
                <>
                  <p className="text-sm leading-6 text-white/65">{t("硬件签名已完成。请在外部钱包签名下面的原始数据，再粘贴签名。", "The hardware signature is ready. Sign the raw payload in an external wallet, then paste the signature.")}</p>
                  <div className="relative">
                    <code className="block max-h-40 overflow-auto break-all rounded-md bg-black/25 p-3 pr-10 text-xs leading-5 text-white/80">{authorizationChallenge.signingPayload}</code>
                    <button type="button" className="theme-icon-btn absolute right-2 top-2 h-7 w-7" onClick={() => onCopyText(authorizationChallenge.signingPayload)} title={t("复制签名数据", "Copy payload")}><Copy className="h-3.5 w-3.5" /></button>
                  </div>
                  <label className="grid gap-1.5 text-xs text-white/65">
                    <span>{t("目标资产密钥签名", "Target asset-key signature")}</span>
                    <input type="text" autoComplete="off" spellCheck={false} value={externalSignature} onChange={(event) => setExternalSignature(event.target.value)} className="theme-input w-full font-mono" placeholder={t("粘贴 128 位十六进制签名", "Paste a 128-character hex signature")} />
                  </label>
                  <div className="flex justify-end gap-2">
                    <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={loading} onClick={() => { onCancelExternalAuthorization(); setExternalSignature(""); }}>{t("返回", "Back")}</button>
                    <button type="button" className="theme-accent-btn px-4 py-2 text-sm" disabled={!externalSignatureValid || loading} onClick={() => void completeExternalAuthorization()}>
                      {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <ShieldCheck className="h-4 w-4" />}
                      {t("提交签名", "Submit signature")}
                    </button>
                  </div>
                </>
              ) : (
                <>
                  <p className="text-sm leading-6 text-white/65">{t("保留原有钱包签名方式：先填写目标公钥，再复制签名数据到外部钱包。", "The existing wallet flow remains available: enter the target public key, then sign the generated payload in an external wallet.")}</p>
                  <label className="grid gap-1.5 text-xs text-white/65">
                    <span>{t("目标资产公钥", "Target asset public key")}</span>
                    <input type="text" autoComplete="off" spellCheck={false} value={publicKey} onChange={(event) => setPublicKey(event.target.value)} className="theme-input w-full" placeholder={t("粘贴 K1 资产公钥", "Paste a K1 asset public key")} />
                  </label>
                  {publicKey && !publicKeyValidation.ok ? <p className="text-xs text-rose-200">{t("公钥格式无效，请检查 K1 Base58 公钥和校验和。", "Invalid public key. Check the K1 Base58 public key and checksum.")}</p> : null}
                  <div className="flex justify-end gap-2">
                    <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={loading} onClick={close}>{t("取消", "Cancel")}</button>
                    <button type="button" className="theme-accent-btn px-4 py-2 text-sm" disabled={!publicKeyValidation.ok || !chipReady || !remoteNodeReady || loading} onClick={() => void startExternalAuthorization()}>
                      {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <KeyRound className="h-4 w-4" />}
                      {t("生成签名请求", "Create signing request")}
                    </button>
                  </div>
                </>
              )}
            </>
          )}
        </div>
      </section>
    </div>
  );
}
