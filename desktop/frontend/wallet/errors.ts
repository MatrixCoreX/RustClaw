import { copy } from "../i18n";
const messages: Record<string, string> = {
  wallet_backup_password_length: "备份密码需要 16–128 个字符，最多 512 字节，不能包含控制字符。",
  wallet_backup_password_weak: "备份密码容易被猜到。请改用多个无关词组成的长密码，避免常用词组、重复或连续字符。",
  wallet_backup_password_reused: "备份密码不能与密钥库密码相同。请为备份设置独立密码。",
  wallet_backup_parameters_unsupported: "此备份的加密参数不受支持。请保留原文件，使用兼容的桌面版本恢复。",
  wallet_backup_verify_failed: "备份未通过读回验证，未标记为完成。请检查磁盘并重新导出。",
  wallet_memory_protection_unavailable: "系统无法保护密钥内存，资产操作已停止。请检查系统资源或重新启动桌面端。",
  wallet_process_protection_unavailable: "系统未能建立所需的私钥进程保护，资产操作已停止。请重启桌面端后重试。",
  wallet_worker_unavailable: "密钥进程不可用，资产操作已停止。请重新启动桌面端。",
  wallet_worker_timeout: "密钥操作超时，进程已停止。请重新启动桌面端；如有已提交的交易，请先核实结果。",
  wallet_worker_protocol_invalid: "密钥进程通信异常，资产操作已停止。请重新启动桌面端。",
  wallet_migration_required: "请先解锁密钥库，完成账号保护升级。",
  wallet_migration_failed: "账号保护升级未通过验证，原文件已保留。请保留备份并重新启动桌面端。",

  wallet_node_no_healthy: "暂时没有找到同一账本内可用的资产节点。请检查网络后重新优选，或手动选择节点。",
  wallet_node_ledger_ambiguous: "可用节点属于不同账本，请先手动选择要使用的节点。",
  wallet_node_connection_failed: "无法建立资产节点的安全连接。请检查地址、网络和证书，或选择其他节点。",
  wallet_node_redirect_rejected: "资产节点返回了重定向。请使用节点提供的直接 HTTPS 地址。",
  wallet_node_config_invalid: "本机资产节点配置无法读取。请保留原文件后检查配置。",
  wallet_node_missing: "请选择并连接一个资产节点。",
  wallet_node_limit: "最多保存 32 个资产节点。",
  wallet_backend_unsupported:
    "该设备尚未支持桌面本地账户。账户已安全保存在本机，余额、交易和转账需等待后端升级。",
  wallet_backend_account_restricted:
    "该设备目前只允许硬件绑定账户，尚未开放独立资产账户。",
  wallet_locked: "密钥库已锁定。请打开“管理本地账号”解锁。",
  wallet_backup_required: "请先在管理窗口完成加密备份，再使用这个账号。",
  wallet_unlock_failed:
    "密码不正确，或加密数据已损坏。请检查密码或使用加密备份恢复。",
  wallet_keystore_unavailable:
    "系统凭据库不可用或已锁定。请解锁系统凭据库后重试；私钥不会改存为明文。",
  wallet_password_length: "密码至少需要 12 个字符，最多 1024 字节。",
  wallet_account_duplicate: "该账户已经保存在本机，无需重复恢复。",
  wallet_unlock_rate_limited: "密码尝试过于频繁，请等待倒计时结束后重试。",
  wallet_challenge_invalid: "服务端签名内容不符合安全合同，本次操作已停止。",
  wallet_challenge_mismatch:
    "服务端返回的账户、金额、费用或有效期与本次操作不一致，未签名。",
  wallet_response_invalid:
    "服务端返回的数据无法验证。请刷新或联系服务端维护者。",
  wallet_node_invalid: "资产节点未提供安全连接，已停止本次操作。",
  wallet_node_changed: "资产节点或账本已经变化，请回到原节点核实交易结果。",
  wallet_confirmation_pending: "已有一笔操作等待确认，请先到安全窗口处理。",
  wallet_confirmation_missing: "本次确认已失效，请重新发起操作。",
  wallet_operation_expired: "本次签名请求已过期，请重新获取。",
  wallet_outcome_unknown: "暂时无法确认结果。请使用“核实结果”，不要重复提交。",
  wallet_outcome_rate_limited: "资产服务请求过于频繁，结果尚未确认。请稍后使用“核实结果”，不要重复提交。",
  wallet_outcome_node_changed: "资产节点或登录上下文已变化。请恢复原节点并重新登录，再核实结果，不要重复提交。",
  wallet_rate_limited: "资产服务请求过于频繁，请稍后再试。",
  wallet_quote_changed: "市场价格或费用已变化，请重新获取报价并确认。",
  wallet_same_account: "收款账户不能与付款账户相同。",
  wallet_amount_too_small: "金额低于当前市场可交易的最小额度。",
  wallet_market_unavailable: "市场暂时无法成交，请稍后刷新报价再试。",
  wallet_account_frozen: "此资产账户已被冻结，暂时无法操作。",
  wallet_system_account_reserved: "系统储备和手续费账户不接受普通转账，请核对收款地址。",
  wallet_unresolved_operation:
    "该账户有一笔结果待核实的操作。核实后才能继续提交，避免重复扣款。",
  wallet_selection_changed: "账户或连接已切换，本次操作已撤销，请重新操作。",
  wallet_access_denied: "设备登录或资产授权已失效，请重新登录。",
  wallet_admin_required: "请使用设备管理员账户访问资产功能。",
  wallet_insufficient_balance: "当前账户余额不足。",
  wallet_network_failed: "暂时无法连接资产服务，请检查网络后刷新。",
  wallet_amount_invalid: "金额需为正数，最多 8 位小数，且不能超过支持范围。",
  wallet_public_key_invalid: "收款公钥格式或校验和无效，请核对完整地址。",
  wallet_intent_invalid: "请检查资产、金额、收款账户和费用设置。",
  wallet_name_invalid: "账户名称需要 1–50 个可见字符。",
  wallet_backup_invalid:
    "备份格式或密钥校验不通过，请选择本客户端生成的加密备份。",
  wallet_data_invalid: "本地加密数据无法验证，请保留原文件并使用备份恢复。",
  wallet_storage_invalid: "密钥库文件或目录不符合安全要求。请保留原文件，检查目录是否为链接或改用加密备份恢复。",
  wallet_storage_unavailable: "无法读写本地密钥库，请检查磁盘空间和目录权限。",
  wallet_backup_path_invalid: "请将备份保存到密钥库目录以外的位置。",
  wallet_already_open: "密钥库正在由另一个客户端使用，请先关闭另一窗口。",
  wallet_account_limit: "本机最多保存 100 个资产账户。",
  wallet_history_limit:
    "待核实操作已达到安全上限，请先核实已有操作，再继续提交。",
  wallet_backend_rejected: "资产服务拒绝了请求。请检查服务状态后重试。",
};
export function walletError(error: unknown): string {
  return copy(
    messages[String(error)] ?? "操作未完成。请检查连接和密钥库状态后重试。"
  );
}
