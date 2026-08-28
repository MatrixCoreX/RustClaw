import { useState } from "react";

import type { ApiResponse, NniAssetTransferResponse } from "../types/api";
import type { AssetTransferAsset } from "../lib/asset-transfer";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface AssetTransferInput {
  asset: AssetTransferAsset;
  amount: string;
  recipientPublicKey: string;
  authorizationMode: "delegated_hardware" | "asset_owner";
  ownerPrivateKey?: string;
}

interface AssetTransferFailureData {
  attempts?: Array<{ error_code?: string }>;
}

function transferErrorCode(
  body: ApiResponse<NniAssetTransferResponse | AssetTransferFailureData>,
): string | null {
  const attempts = body.data && "attempts" in body.data ? body.data.attempts : undefined;
  return attempts?.at(-1)?.error_code ?? body.error ?? null;
}

function transferErrorMessage(code: string | null, t: Translate): string {
  switch (code) {
    case "nni_asset_transfer_insufficient_aic_balance":
      return t("AIC 余额不足。", "The AIC balance is insufficient.");
    case "nni_asset_transfer_insufficient_usd_balance":
      return t("USD 余额不足。", "The USD balance is insufficient.");
    case "nni_owner_pubkey_invalid":
      return t("收款账户公钥不合规。", "The recipient public key is not valid.");
    case "nni_asset_transfer_same_account":
      return t("不能向当前账户转账。", "You cannot transfer to the current account.");
    case "nni_owner_private_key_mismatch":
      return t("私钥与当前资产账户不匹配。", "The private key does not match the current asset account.");
    case "nni_asset_owner_required":
    case "nni_asset_device_not_authorized":
    case "nni_asset_authorization_changed":
      return t("当前资产账户授权已失效，请先在 NNI 页面重新绑定。", "This asset authorization is no longer valid. Rebind it on the NNI page first.");
    case "nni_signature_verify_failed":
    case "nni_owner_signature_verify_failed":
      return t("签名验证失败，资产没有转出。", "Signature verification failed. No assets were transferred.");
    case "nni_asset_transfer_expired":
      return t("本次签名请求已过期，请重新提交。", "This signing request expired. Submit it again.");
    case "nni_asset_transfer_rate_limited":
    case "nni_rate_limit_asset_transfer":
      return t("操作过于频繁，请稍后再试。", "Too many requests. Try again shortly.");
    case "nni_asset_transfer_outcome_unknown":
      return t("暂时无法确认转账结果，请先刷新余额和资产浏览器，避免重复提交。", "The transfer outcome is temporarily unknown. Refresh the balance and asset explorer before trying again.");
    default:
      return t("转账未完成，请稍后重试。", "The transfer was not completed. Try again later.");
  }
}

export function useAssetTransferRuntime({
  apiFetch,
  t,
  onCompleted,
}: {
  apiFetch: ApiFetch;
  t: Translate;
  onCompleted?: () => void | Promise<unknown>;
}) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [lastTransfer, setLastTransfer] = useState<NniAssetTransferResponse | null>(null);

  const transfer = async (input: AssetTransferInput) => {
    setLoading(true);
    setError(null);
    setMessage(null);
    try {
      const response = await apiFetch("/v1/nni/assets/transfer", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          asset: input.asset,
          amount: input.amount,
          to_asset_owner_pubkey: input.recipientPublicKey,
          authorization_mode: input.authorizationMode,
          ...(input.authorizationMode === "asset_owner"
            ? { owner_private_key: input.ownerPrivateKey ?? "" }
            : {}),
        }),
      });
      const body = (await response.json()) as ApiResponse<
        NniAssetTransferResponse | AssetTransferFailureData
      >;
      if (!response.ok || !body.ok || !body.data || !("transfer" in body.data)) {
        throw new Error(transferErrorMessage(transferErrorCode(body), t));
      }
      const result = body.data as NniAssetTransferResponse;
      setLastTransfer(result);
      setMessage(t("转账已完成，余额和资产浏览器记录已经更新。", "Transfer completed. The balance and asset explorer are updated."));
      try {
        await onCompleted?.();
      } catch {
        // The signed transfer is final even when the follow-up balance refresh is unavailable.
      }
      return result;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : t("转账未完成。", "The transfer was not completed."));
      return null;
    } finally {
      setLoading(false);
    }
  };

  return {
    loading,
    error,
    message,
    lastTransfer,
    transfer,
    clearFeedback: () => {
      setError(null);
      setMessage(null);
    },
  };
}
