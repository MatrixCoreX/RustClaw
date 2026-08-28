import { ArrowLeft, CheckCircle2, KeyRound, SendHorizontal, ShieldCheck, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import {
  validateAssetTransferDraft,
  type AssetTransferAsset,
  type AssetTransferDraftError,
} from "../lib/asset-transfer";
import { validateNniOwnerPrivateKey, validateNniOwnerPublicKey } from "../lib/nni-owner-public-key";
import type { AssetTransferInput } from "../hooks/useAssetTransferRuntime";

type Translate = (zh: string, en: string) => string;
type AuthorizationMode = "delegated_hardware" | "asset_owner";

function draftErrorMessage(error: AssetTransferDraftError, asset: AssetTransferAsset, t: Translate) {
  switch (error) {
    case "source_required":
      return t("当前资产账户不可用。", "The current asset account is unavailable.");
    case "recipient_required":
      return t("请输入收款账户公钥。", "Enter the recipient public key.");
    case "recipient_invalid":
      return t("收款账户公钥不合规。", "The recipient public key is not valid.");
    case "same_account":
      return t("收款账户不能与当前账户相同。", "The recipient cannot be the current account.");
    case "amount_invalid":
      return t("请输入大于 0 且最多 8 位小数的金额。", "Enter an amount above 0 with no more than 8 decimal places.");
    case "amount_too_large":
      return t("金额超出系统可处理范围。", "The amount exceeds the supported range.");
    case "insufficient_balance":
      return t(`${asset} 余额不足。`, `The ${asset} balance is insufficient.`);
  }
}

export function AssetTransferDialog({
  open,
  t,
  sourcePublicKey,
  aicBalance,
  usdBalance,
  signingDeviceReady,
  loading,
  remoteError,
  onClose,
  onSubmit,
}: {
  open: boolean;
  t: Translate;
  sourcePublicKey: string;
  aicBalance: string;
  usdBalance: string;
  signingDeviceReady: boolean;
  loading: boolean;
  remoteError: string | null;
  onClose: () => void;
  onSubmit: (input: AssetTransferInput) => Promise<unknown>;
}) {
  const [asset, setAsset] = useState<AssetTransferAsset>("AIC");
  const [amount, setAmount] = useState("");
  const [recipientPublicKey, setRecipientPublicKey] = useState("");
  const [authorizationMode, setAuthorizationMode] = useState<AuthorizationMode>(
    signingDeviceReady ? "delegated_hardware" : "asset_owner",
  );
  const [ownerPrivateKey, setOwnerPrivateKey] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);
  const [reviewing, setReviewing] = useState(false);
  const [completed, setCompleted] = useState(false);
  const availableBalance = asset === "AIC" ? aicBalance : usdBalance;
  const recipientValidation = useMemo(
    () => recipientPublicKey.trim() ? validateNniOwnerPublicKey(recipientPublicKey) : null,
    [recipientPublicKey],
  );

  useEffect(() => {
    if (!open) {
      setAmount("");
      setRecipientPublicKey("");
      setOwnerPrivateKey("");
      setLocalError(null);
      setReviewing(false);
      setCompleted(false);
    }
  }, [open]);

  if (!open) return null;

  const validateForReview = () => {
    const draft = validateAssetTransferDraft({
      sourcePublicKey,
      recipientPublicKey,
      asset,
      amount,
      availableBalance,
    });
    if (draft.ok === false) {
      setLocalError(draftErrorMessage(draft.error, asset, t));
      return null;
    }
    if (authorizationMode === "delegated_hardware" && !signingDeviceReady) {
      setLocalError(t("当前设备无法使用硬件代签。", "Hardware signing is not available on this device."));
      return null;
    }
    if (authorizationMode === "asset_owner") {
      const privateKey = validateNniOwnerPrivateKey(ownerPrivateKey);
      if (!privateKey.ok) {
        setLocalError(t("资产私钥不合规。", "The asset private key is not valid."));
        return null;
      }
      if (privateKey.publicKey !== draft.sourcePublicKey) {
        setLocalError(t("私钥与当前资产账户不匹配。", "The private key does not match the current asset account."));
        return null;
      }
    }
    setLocalError(null);
    return draft;
  };

  const submit = async () => {
    const draft = validateForReview();
    if (!draft) return;
    const result = await onSubmit({
      asset: draft.asset,
      amount: draft.amount,
      recipientPublicKey: draft.recipientPublicKey,
      authorizationMode,
      ...(authorizationMode === "asset_owner" ? { ownerPrivateKey } : {}),
    });
    setOwnerPrivateKey("");
    if (result) setCompleted(true);
  };

  return (
    <div className="fixed inset-0 z-[90] flex items-center justify-center bg-black/55 p-4" role="presentation" onMouseDown={(event) => {
      if (event.currentTarget === event.target && !loading) onClose();
    }}>
      <section className="theme-panel max-h-[92vh] w-full max-w-xl overflow-y-auto p-5 sm:p-6" role="dialog" aria-modal="true" aria-labelledby="asset-transfer-title">
        <header className="flex items-start justify-between gap-4">
          <div>
            <h2 id="asset-transfer-title" className="text-lg font-semibold text-[var(--theme-text-strong)]">
              {t("资产转账", "Asset transfer")}
            </h2>
            <p className="mt-1 text-sm leading-5 text-[var(--theme-text-muted)]">
              {t("向合规的资产公钥转出 AIC 或 USD。", "Send AIC or USD to a valid asset public key.")}
            </p>
          </div>
          <button type="button" className="theme-icon-btn" aria-label={t("关闭", "Close")} disabled={loading} onClick={onClose}>
            <X className="h-4 w-4" />
          </button>
        </header>

        {completed ? (
          <div className="py-10 text-center" data-asset-transfer-completed="true">
            <CheckCircle2 className="mx-auto h-10 w-10 text-emerald-400" />
            <p className="mt-4 font-semibold text-[var(--theme-text-strong)]">{t("转账已完成", "Transfer completed")}</p>
            <p className="mt-2 text-sm text-[var(--theme-text-muted)]">{t("余额和资产浏览器记录已经更新。", "The balance and asset explorer record are updated.")}</p>
            <button type="button" className="theme-accent-btn mt-6 px-4 py-2 text-sm" onClick={onClose}>{t("完成", "Done")}</button>
          </div>
        ) : reviewing ? (
          <div className="mt-6 space-y-5" data-asset-transfer-review="true">
            <div className="space-y-3 rounded-md border border-[var(--theme-border)] bg-[var(--theme-card-strong)] p-4">
              <div>
                <p className="text-xs text-[var(--theme-text-muted)]">{t("转出", "Amount")}</p>
                <p className="mt-1 text-xl font-semibold text-[var(--theme-text-strong)]">{amount} {asset}</p>
              </div>
              <div>
                <p className="text-xs text-[var(--theme-text-muted)]">{t("付款账户", "From")}</p>
                <p className="mt-1 break-all font-mono text-xs text-[var(--theme-text-body)]">{sourcePublicKey}</p>
              </div>
              <div>
                <p className="text-xs text-[var(--theme-text-muted)]">{t("收款账户", "To")}</p>
                <p className="mt-1 break-all font-mono text-xs text-[var(--theme-text-body)]">{recipientPublicKey.trim()}</p>
              </div>
              <div>
                <p className="text-xs text-[var(--theme-text-muted)]">{t("签名方式", "Signing method")}</p>
                <p className="mt-1 text-sm text-[var(--theme-text-body)]">
                  {authorizationMode === "delegated_hardware" ? t("硬件设备代签", "Hardware signing") : t("资产私钥签名", "Asset private key")}
                </p>
              </div>
            </div>
            {(localError || remoteError) ? <p role="alert" className="rounded-md border border-rose-400/30 bg-rose-500/10 px-3 py-2 text-sm leading-6 text-rose-100">{localError || remoteError}</p> : null}
            <div className="flex flex-wrap justify-end gap-2">
              <button type="button" className="theme-secondary-btn px-4 py-2 text-sm" disabled={loading} onClick={() => setReviewing(false)}>
                <ArrowLeft className="h-4 w-4" />
                {t("返回修改", "Back")}
              </button>
              <button type="button" className="theme-accent-btn px-4 py-2 text-sm" disabled={loading} onClick={() => void submit()}>
                <SendHorizontal className="h-4 w-4" />
                {loading ? t("签名并提交中", "Signing and submitting") : t("确认转账", "Confirm transfer")}
              </button>
            </div>
          </div>
        ) : (
          <div className="mt-6 space-y-5">
            <fieldset>
              <legend className="text-sm font-medium text-[var(--theme-text-strong)]">{t("选择资产", "Choose asset")}</legend>
              <div className="mt-2 grid grid-cols-2 gap-2" role="tablist">
                {(["AIC", "USD"] as const).map((option) => (
                  <button key={option} type="button" role="tab" aria-selected={asset === option} className={asset === option ? "theme-accent-btn justify-center px-3 py-2" : "theme-secondary-btn justify-center px-3 py-2"} onClick={() => {
                    setAsset(option);
                    setLocalError(null);
                  }}>{option}</button>
                ))}
              </div>
            </fieldset>
            <label className="grid gap-1.5">
              <span className="text-sm font-medium text-[var(--theme-text-strong)]">{t("转账金额", "Amount")}</span>
              <input className="theme-input" inputMode="decimal" value={amount} placeholder="0.00000000" onChange={(event) => {
                setAmount(event.target.value);
                setLocalError(null);
              }} />
              <span className="text-xs text-[var(--theme-text-muted)]">{t("可用余额", "Available")}: {availableBalance} {asset}</span>
            </label>
            <label className="grid gap-1.5">
              <span className="text-sm font-medium text-[var(--theme-text-strong)]">{t("收款账户公钥", "Recipient public key")}</span>
              <textarea className="theme-input min-h-20 resize-y font-mono text-xs" value={recipientPublicKey} spellCheck={false} autoComplete="off" onChange={(event) => {
                setRecipientPublicKey(event.target.value);
                setLocalError(null);
              }} />
              {recipientValidation ? (
                <span className={`text-xs ${recipientValidation.ok ? "text-emerald-400" : "text-rose-400"}`}>
                  {recipientValidation.ok ? t("公钥格式有效", "Valid public key") : t("公钥格式无效", "Invalid public key")}
                </span>
              ) : null}
            </label>
            <fieldset>
              <legend className="text-sm font-medium text-[var(--theme-text-strong)]">{t("签名方式", "Signing method")}</legend>
              <div className="mt-2 grid gap-2 sm:grid-cols-2">
                <button type="button" disabled={!signingDeviceReady} className={authorizationMode === "delegated_hardware" ? "theme-accent-btn justify-start px-3 py-3" : "theme-secondary-btn justify-start px-3 py-3 disabled:opacity-45"} onClick={() => {
                  setAuthorizationMode("delegated_hardware");
                  setOwnerPrivateKey("");
                  setLocalError(null);
                }}>
                  <ShieldCheck className="h-4 w-4" />
                  {t("硬件设备代签", "Hardware signing")}
                </button>
                <button type="button" className={authorizationMode === "asset_owner" ? "theme-accent-btn justify-start px-3 py-3" : "theme-secondary-btn justify-start px-3 py-3"} onClick={() => {
                  setAuthorizationMode("asset_owner");
                  setLocalError(null);
                }}>
                  <KeyRound className="h-4 w-4" />
                  {t("资产私钥签名", "Asset private key")}
                </button>
              </div>
            </fieldset>
            {authorizationMode === "asset_owner" ? (
              <label className="grid gap-1.5">
                <span className="text-sm font-medium text-[var(--theme-text-strong)]">{t("资产私钥", "Asset private key")}</span>
                <input className="theme-input font-mono text-xs" type="password" autoComplete="new-password" value={ownerPrivateKey} onChange={(event) => {
                  setOwnerPrivateKey(event.target.value);
                  setLocalError(null);
                }} />
                <span className="text-xs leading-5 text-[var(--theme-text-muted)]">{t("私钥只用于本次签名，不会保存。", "The private key is used only for this signature and is not saved.")}</span>
              </label>
            ) : null}
            {(localError || remoteError) ? <p role="alert" className="rounded-md border border-rose-400/30 bg-rose-500/10 px-3 py-2 text-sm leading-6 text-rose-100">{localError || remoteError}</p> : null}
            <div className="flex justify-end">
              <button type="button" className="theme-accent-btn px-4 py-2 text-sm" onClick={() => {
                if (validateForReview()) setReviewing(true);
              }}>
                {t("查看并确认", "Review transfer")}
              </button>
            </div>
          </div>
        )}
      </section>
    </div>
  );
}
