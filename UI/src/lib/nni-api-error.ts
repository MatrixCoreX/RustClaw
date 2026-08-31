export type NniErrorTranslate = (zh: string, en: string) => string;

const MACHINE_ERROR_TOKEN = /^[a-z][a-z0-9]*(?:[._:-][a-z0-9]+)+$/i;

export function formatNniApiError(
  code: string | null | undefined,
  t: NniErrorTranslate,
  fallback?: string,
): string {
  const normalized = code?.trim() ?? "";
  switch (normalized) {
    case "nni_rate_limit_explorer_read":
      return t("资产记录读取过于频繁，请稍后刷新。", "Asset activity is being refreshed too frequently. Try again shortly.");
    case "nni_rate_limit_network_read":
      return t("网络状态刷新过于频繁，请稍后再试。", "Network status is being refreshed too frequently. Try again shortly.");
    case "nni_rate_limit_bancor_public_read":
      return t("市场数据刷新过于频繁，请稍后再试。", "Market data is being refreshed too frequently. Try again shortly.");
    case "nni_rate_limit_bancor_private":
      return t("账户与交易请求过于频繁，请稍后再试。", "Account and trading requests are too frequent. Try again shortly.");
    case "nni_rate_limit_asset_transfer":
    case "nni_asset_transfer_rate_limited":
    case "nni_asset_transfer_account_cooldown":
    case "nni_asset_transfer_account_rate_limited":
      return t("转账请求过于频繁，请稍后再试。", "Transfer requests are too frequent. Try again shortly.");
    case "nni_rate_limit_heartbeat":
      return t("心跳提交过于频繁，系统稍后会继续尝试。", "Heartbeat submissions are too frequent. The system will try again later.");
    case "nni_rate_limit_asset_authorization":
      return t("资产绑定或授权操作过于频繁，请稍后再试。", "Asset binding or authorization requests are too frequent. Try again shortly.");
    case "nni_rate_limit_reward_private":
      return t("奖励记录读取过于频繁，请稍后再试。", "Reward activity is being requested too frequently. Try again shortly.");
    case "nni_rate_limit_admin_read":
      return t("管理数据刷新过于频繁，请稍后再试。", "Administrative data is being refreshed too frequently. Try again shortly.");
    case "nni_rate_limit_admin_write":
      return t("管理操作过于频繁，请稍后再试。", "Administrative changes are being submitted too frequently. Try again shortly.");
    case "nni_rate_limit_general":
    case "nni_bancor_trade_rate_limited":
    case "nni_bancor_ip_rate_limited":
      return t("请求过于频繁，请稍后再试。", "Requests are too frequent. Try again shortly.");
    case "nni_remote_nodes_unavailable":
    case "nni_bancor_account_nodes_unavailable":
    case "nni_asset_transfer_nodes_unavailable":
      return t("当前节点暂时无法连接，请稍后重试或切换节点。", "The selected node is temporarily unavailable. Try again later or switch nodes.");
    case "nni_owner_private_key_invalid":
      return t("资产私钥无效，请检查后重试。", "The asset private key is invalid. Check it and try again.");
    case "nni_owner_private_key_mismatch":
    case "nni_owner_private_key_target_mismatch":
      return t("资产私钥与当前资产账户不匹配。", "The asset private key does not match the current asset account.");
    case "nni_join_challenge_missing":
      return t("服务端没有返回签名挑战，请重新发起操作。", "The server did not return a signing challenge. Start the operation again.");
    case "nni_join_signature_missing":
    case "nni_asset_unbind_device_signature_missing":
      return t("设备签名未完成，请检查芯片状态后重试。", "Device signing did not complete. Check the chip status and try again.");
    case "nni_join_verify_rejected":
      return t("签名验证未通过，请重新发起操作。", "Signature verification was not accepted. Start the operation again.");
    case "nni_target_owner_signature_requirement_missing":
      return t("节点未提供所需的资产签名流程，请切换节点或稍后重试。", "The node did not provide the required asset-signing flow. Switch nodes or try again later.");
    case "nni_owner_recovery_identity_changed":
      return t("恢复过程中资产身份不一致，操作已停止。", "The asset identity changed during recovery, so the operation was stopped.");
    case "nni_public_key_whitelist_empty":
    case "public_key_whitelist_empty":
    case "nni_pubkey_not_allowlisted":
    case "nni_public_key_not_allowlisted":
    case "public_key_not_allowlisted":
      return t("当前设备尚未获得 NNI 网络准入，请使用合法设备。", "This device has not been admitted to the NNI network. Use an authorized device.");
    case "nni_asset_owner_required":
      return t("请先在 NNI 页面绑定资产账户。", "Bind an asset account on the NNI page first.");
    case "nni_asset_device_not_authorized":
    case "nni_asset_authorization_changed":
      return t("当前设备的资产授权已失效，请在 NNI 页面重新绑定。", "This device's asset authorization is no longer valid. Rebind it on the NNI page.");
    default:
      if (MACHINE_ERROR_TOKEN.test(normalized)) {
        return t(
          "NNI 操作未完成，请稍后重试；如果仍然失败，请查看日志或联系管理员。",
          "The NNI operation did not complete. Try again later; if it still fails, review the logs or contact an administrator.",
        );
      }
      return normalized || fallback || t("NNI 操作未完成，请稍后重试。", "The NNI operation did not complete. Try again later.");
  }
}

export function formatNniErrorCause(
  cause: unknown,
  t: NniErrorTranslate,
  fallback?: string,
): string {
  const code = cause instanceof Error
    ? cause.message
    : typeof cause === "string"
      ? cause
      : null;
  return formatNniApiError(code, t, fallback);
}
