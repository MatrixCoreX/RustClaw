import {
  ArrowLeft,
  CheckCircle2,
  CircleDollarSign,
  Coins,
  KeyRound,
  SendHorizontal,
  ShieldCheck,
  X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import {
  ASSET_TRANSFER_MEMO_MAX_BYTES,
  assetTransferMemoByteLength,
  validateAssetTransferDraft,
  type AssetTransferAsset,
  type AssetTransferDraftError,
} from "../lib/asset-transfer";
import {
  nniPrivateKeyOperationsAllowed,
  validateNniOwnerPrivateKey,
  validateNniOwnerPublicKey,
} from "../lib/nni-owner-public-key";
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
    case "memo_too_long":
      return t("Memo 不能超过 256 字节。", "The memo cannot exceed 256 bytes.");
  }
}

export function AssetTransferDialog({
  open,
  asset,
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
  asset: AssetTransferAsset;
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
  const privateKeyOperationsAllowed = nniPrivateKeyOperationsAllowed();
  const [amount, setAmount] = useState("");
  const [recipientPublicKey, setRecipientPublicKey] = useState("");
  const [memo, setMemo] = useState("");
  const [authorizationMode, setAuthorizationMode] = useState<AuthorizationMode>(
    signingDeviceReady || !privateKeyOperationsAllowed ? "delegated_hardware" : "asset_owner",
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
  const memoByteLength = useMemo(() => assetTransferMemoByteLength(memo), [memo]);
  const AssetIcon = asset === "AIC" ? Coins : CircleDollarSign;
  const assetPanelClass = asset === "AIC"
    ? "border-emerald-400/40 bg-emerald-500/10"
    : "border-amber-400/40 bg-amber-500/10";
  const assetIconClass = asset === "AIC"
    ? "border-emerald-400/30 bg-emerald-500/15 text-emerald-300"
    : "border-amber-400/30 bg-amber-500/15 text-amber-300";
  const assetTextClass = asset === "AIC" ? "text-emerald-300" : "text-amber-300";

  useEffect(() => {
    if (!open) {
      setAmount("");
      setRecipientPublicKey("");
      setMemo("");
      setOwnerPrivateKey("");
      setLocalError(null);
      setReviewing(false);
      setCompleted(false);
      setAuthorizationMode(signingDeviceReady || !privateKeyOperationsAllowed ? "delegated_hardware" : "asset_owner");
    }
  }, [open, privateKeyOperationsAllowed, signingDeviceReady]);

  useEffect(() => {
    if (privateKeyOperationsAllowed || authorizationMode !== "asset_owner") return;
    setAuthorizationMode("delegated_hardware");
    setOwnerPrivateKey("");
  }, [authorizationMode, privateKeyOperationsAllowed]);

  if (!open) return null;

  const validateForReview = () => {
    const draft = validateAssetTransferDraft({
      sourcePublicKey,
      recipientPublicKey,
      asset,
      amount,
      availableBalance,
      memo,
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
      if (!privateKeyOperationsAllowed) {
        setLocalError(t(
          "当前页面使用非本机 HTTP 连接，已禁用资产私钥签名。请改用 HTTPS 或设备本机 localhost。",
          "Asset private-key signing is disabled over non-loopback HTTP. Use HTTPS or localhost on this device.",
        ));
        return null;
      }
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
      memo: draft.memo,
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
      <section className="theme-dialog-panel max-h-[92vh] w-full max-w-xl overflow-y-auto p-5 sm:p-6" role="dialog" aria-modal="true" aria-labelledby="asset-transfer-title">
        <header className="flex items-start justify-between gap-4">
          <div>
            <h2 id="asset-transfer-title" className="text-lg font-semibold text-[var(--theme-text-strong)]">
              {t(`${asset} 转账`, `${asset} transfer`)}
            </h2>
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
            <div className={`space-y-3 rounded-md border p-4 ${assetPanelClass}`}>
              <div className="flex items-center gap-3" data-transfer-review-asset={asset}>
                <span className={`flex h-11 w-11 shrink-0 items-center justify-center rounded-full border ${assetIconClass}`}>
                  <AssetIcon className="h-5 w-5" />
                </span>
                <div>
                  <p className="text-xs text-[var(--theme-text-muted)]">{t("转出资产与金额", "Asset and amount")}</p>
                  <p className={`mt-0.5 text-2xl font-bold ${assetTextClass}`}>{amount} {asset}</p>
                </div>
              </div>
              <div>
                <p className="text-xs text-[var(--theme-text-muted)]">{t("付款账户", "From")}</p>
                <p className="mt-1 break-all font-mono text-xs text-[var(--theme-text-body)]">{sourcePublicKey}</p>
              </div>
              <div>
                <p className="text-xs text-[var(--theme-text-muted)]">{t("收款账户", "To")}</p>
                <p className="mt-1 break-all font-mono text-xs text-[var(--theme-text-body)]">{recipientPublicKey.trim()}</p>
              </div>
              {memo ? (
                <div>
                  <p className="text-xs text-[var(--theme-text-muted)]">Memo</p>
                  <p className="mt-1 whitespace-pre-wrap break-words text-sm text-[var(--theme-text-body)]">{memo}</p>
                </div>
              ) : null}
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
            <div
              className={`flex items-center gap-3 rounded-md border px-4 py-3 ${assetPanelClass}`}
              data-transfer-asset={asset}
              aria-label={t(`本次转账资产：${asset}`, `Transfer asset: ${asset}`)}
            >
              <span className={`flex h-11 w-11 shrink-0 items-center justify-center rounded-full border ${assetIconClass}`}>
                <AssetIcon className="h-5 w-5" />
              </span>
              <div>
                <p className="text-xs font-medium text-[var(--theme-text-muted)]">{t("本次转账资产", "Transfer asset")}</p>
                <p className={`mt-0.5 text-2xl font-bold ${assetTextClass}`} data-transfer-asset-symbol={asset}>{asset}</p>
              </div>
            </div>
            <label className="grid gap-1.5">
              <span className="text-sm font-medium text-[var(--theme-text-strong)]">{t("转账金额", "Amount")}</span>
              <input className="theme-input" inputMode="decimal" value={amount} placeholder="0.00000000" onChange={(event) => {
                setAmount(event.target.value);
                setLocalError(null);
              }} />
              <span className="text-xs text-[var(--theme-text-muted)]">{t("可用余额", "Available")}: {availableBalance} {asset}</span>
            </label>
            <label className="grid max-w-md gap-1.5">
              <span className="text-sm font-medium text-[var(--theme-text-strong)]">{t("收款账户公钥", "Recipient public key")}</span>
              <input className="theme-input font-mono text-xs" type="text" aria-label={t("收款账户公钥", "Recipient public key")} value={recipientPublicKey} spellCheck={false} autoComplete="off" onChange={(event) => {
                setRecipientPublicKey(event.target.value);
                setLocalError(null);
              }} />
              {recipientValidation ? (
                <span className={`text-xs ${recipientValidation.ok ? "text-emerald-400" : "text-rose-400"}`}>
                  {recipientValidation.ok ? t("公钥格式有效", "Valid public key") : t("公钥格式无效", "Invalid public key")}
                </span>
              ) : null}
            </label>
            <label className="grid max-w-md gap-1.5">
              <span className="text-sm font-medium text-[var(--theme-text-strong)]">Memo <span className="font-normal text-[var(--theme-text-muted)]">{t("（可选）", "(optional)")}</span></span>
              <input className="theme-input text-sm" type="text" aria-label="Memo" value={memo} onChange={(event) => {
                setMemo(event.target.value);
                setLocalError(null);
              }} />
              <span className={`text-xs ${memoByteLength > ASSET_TRANSFER_MEMO_MAX_BYTES ? "text-rose-400" : "text-[var(--theme-text-muted)]"}`}>
                {memoByteLength} / {ASSET_TRANSFER_MEMO_MAX_BYTES} {t("字节", "bytes")}
              </span>
            </label>
            <fieldset>
              <legend className="text-sm font-medium text-[var(--theme-text-strong)]">{t("签名方式", "Signing method")}</legend>
              <div className="mt-2 grid gap-2 sm:grid-cols-2">
                <button type="button" aria-pressed={authorizationMode === "delegated_hardware"} disabled={!signingDeviceReady} className={authorizationMode === "delegated_hardware" ? "theme-accent-btn justify-start px-3 py-3" : "theme-secondary-btn justify-start px-3 py-3 disabled:opacity-45"} onClick={() => {
                  setAuthorizationMode("delegated_hardware");
                  setOwnerPrivateKey("");
                  setLocalError(null);
                }}>
                  <ShieldCheck className="h-4 w-4" />
                  {t("硬件设备代签", "Hardware signing")}
                </button>
                <button type="button" aria-pressed={authorizationMode === "asset_owner"} disabled={!privateKeyOperationsAllowed} className={authorizationMode === "asset_owner" ? "theme-accent-btn justify-start px-3 py-3" : "theme-secondary-btn justify-start px-3 py-3 disabled:opacity-45"} onClick={() => {
                  setAuthorizationMode("asset_owner");
                  setLocalError(null);
                }}>
                  <KeyRound className="h-4 w-4" />
                  {t("资产私钥签名", "Asset private key")}
                </button>
              </div>
            </fieldset>
            {!privateKeyOperationsAllowed ? (
              <p role="note" className="rounded-md border border-amber-400/30 bg-amber-500/10 px-3 py-2 text-xs leading-5 text-amber-100">
                {t(
                  "非本机 HTTP 页面不接收资产私钥。请使用硬件签名，或改用 HTTPS / 设备本机 localhost。",
                  "Asset private keys are not accepted over non-loopback HTTP. Use hardware signing, HTTPS, or localhost on this device.",
                )}
              </p>
            ) : null}
            {authorizationMode === "asset_owner" ? (
              <label className="grid gap-1.5">
                <span className="text-sm font-medium text-[var(--theme-text-strong)]">{t("资产私钥", "Asset private key")}</span>
                <input
                  className="theme-input font-mono text-xs"
                  type="password"
                  autoComplete="one-time-code"
                  autoCapitalize="none"
                  data-1p-ignore="true"
                  data-bwignore="true"
                  data-form-type="other"
                  data-lpignore="true"
                  data-protonpass-ignore="true"
                  spellCheck={false}
                  value={ownerPrivateKey}
                  onChange={(event) => {
                  setOwnerPrivateKey(event.target.value);
                  setLocalError(null);
                  }}
                />
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
