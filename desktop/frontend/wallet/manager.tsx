import { copy, LanguageToggle, useLanguage } from "../i18n";
import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { PRODUCT_DISPLAY_NAME } from "../../../UI/src/lib/product-identity";
import { displayUnits } from "./amounts";
import { walletError } from "./errors";
import type { Confirmation, Outcome, WalletStatus } from "./types";
import "./wallet.css";
import { initialTheme, ThemeToggle } from "../ThemeToggle";

document.title = `${PRODUCT_DISPLAY_NAME} · ${copy("本地资产安全窗口", "Local asset security")}`;
document.documentElement.dataset.theme = initialTheme();
function Manager() {
  const { lang } = useLanguage();
  useEffect(() => { document.title = `${PRODUCT_DISPLAY_NAME} · ${copy("本地资产安全窗口", "Local asset security")}`; }, [lang]);
  const [status, setStatus] = useState<WalletStatus>({
    initialized: false,
    unlocked: false,
    retry_after_seconds: 0,
    accounts: [],
    storage_version: 2,
    backup_upgrade_accounts: [],
  });
  const [pending, setPending] = useState<Confirmation | null>(null);
  const [password, setPassword] = useState("");
  const [signingPassword, setSigningPassword] = useState("");
  const [repeat, setRepeat] = useState("");
  const [name, setName] = useState("");
  const [backupId, setBackupId] = useState("");
  const [backupPassword, setBackupPassword] = useState("");
  const [backupVaultPassword, setBackupVaultPassword] = useState("");
  const [backupRepeat, setBackupRepeat] = useState("");
  const [restore, setRestore] = useState(false);
  const [restorePassword, setRestorePassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [theme, setTheme] = useState<string>(initialTheme);
  const refresh = async () => {
    const [s, p] = await Promise.all([
      invoke<WalletStatus>("wallet_status"),
      invoke<Confirmation | null>("wallet_pending"),
    ]);
    setStatus(s);
    setPending(p);
  };
  useEffect(() => {
    void refresh().catch((e) => setError(walletError(e)));
    const timer = setInterval(() => {
      void refresh().catch(() => {});
    }, 1000);
    return () => clearInterval(timer);
  }, []);
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("agent-runtime.monitor.themeMode", theme);
  }, [theme]);
  useEffect(() => {
    const clear = () => {
      setPassword(""); setRepeat(""); setSigningPassword("");
      setBackupVaultPassword(""); setBackupPassword(""); setBackupRepeat("");
      setRestorePassword("");
    };
    window.addEventListener("blur", clear);
    return () => window.removeEventListener("blur", clear);
  }, []);
  useEffect(() => {
    if (!status.unlocked) {
      setBackupVaultPassword(""); setBackupPassword(""); setBackupRepeat("");
      setRestorePassword(""); setBackupId("");
    }
  }, [status.unlocked]);
  const retryAfter = status.retry_after_seconds;
  const run = async (fn: () => Promise<void>) => {
    if (busy) return;
    setBusy(true);
    setError("");
    setMessage("");
    try {
      await fn();
      await refresh();
    } catch (e) {
      setError(walletError(e));
      await refresh().catch(() => {});
    } finally {
      setBusy(false);
    }
  };
  const unlock = () =>
    run(async () => {
      if (!status.initialized && password !== repeat) {
        setError(copy("两次输入的密码不一致。"));
        return;
      }
      const value = password;
      setPassword("");
      setRepeat("");
      await invoke(status.initialized ? "wallet_unlock" : "wallet_initialize", {
        password: value,
      });
    });
  const terms = pending?.payload.terms;
  useEffect(() => setSigningPassword(""), [pending?.payload.operation_id]);
  return (
    <main className="wallet-ui wallet-manager">
      <header className="wallet-bar-head">
        <div>
          <span className="wallet-eyebrow">
            {PRODUCT_DISPLAY_NAME}  {copy("· 本机安全窗口")} </span>
          <h1>{copy("本地资产账户")}</h1>
        </div>
        <LanguageToggle />
        <ThemeToggle theme={theme} onToggle={() => setTheme(theme === "light" ? "dark" : "light")} />
      </header>
      <p>{copy("私钥在这台电脑加密保存。查看资产无需解锁；每笔交易须输入密码确认。")}</p>
      {!status.unlocked && !pending ? (
        <form
          className="wallet-card"
          onSubmit={(e) => {
            e.preventDefault();
            void unlock();
          }}
        >
          <h2>{status.initialized ? copy("解锁密钥库") : copy("设置密钥库密码")}</h2>
          <p>
            {status.initialized
              ? copy("解锁后可创建、备份或恢复账号。查看资产不需要解锁。")
              : copy("设置至少 12 个字符的密码。密码不会发送到设备或资产服务。")}
          </p>
          {status.initialized && status.storage_version === 1 && <p className="wallet-note" data-wallet-migration>
            {copy("本次解锁会升级账号保护，账号和地址保持不变。升级后请使用新版桌面端；已有加密备份仍可恢复。")}
          </p>}
          <label className="wallet-label">
             {copy("密钥库密码")} <input
              aria-label={copy("密钥库密码")}
              type="password"
              autoComplete="off"
              minLength={12}
              maxLength={1024}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
            />
          </label>
          {!status.initialized && (
            <label className="wallet-label">
               {copy("再次输入密码")} <input
                type="password"
                autoComplete="off"
                minLength={12}
                value={repeat}
                onChange={(e) => setRepeat(e.target.value)}
                required
              />
            </label>
          )}
          <button className="wallet-primary" disabled={busy || retryAfter > 0}>
            {busy ? copy("正在处理…") : retryAfter > 0 ? copy(`${retryAfter} 秒后重试`, `Retry in ${retryAfter} seconds`) : status.initialized ? copy("解锁") : copy("创建密钥库")}
          </button>
        </form>
      ) : (
        <>
          <div className="wallet-bar-head">
            <span className="wallet-tag">{status.unlocked ? copy("已解锁") : copy("已锁定")}</span>
            <button
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  await invoke("wallet_lock");
                })
              }
            >
               {copy("立即锁定")} </button>
          </div>
          {pending && terms && (
            <section
              className="wallet-card wallet-confirm"
              aria-label={copy("确认资产操作")}
            >
              <h2>
                {terms.kind === "transfer" ? copy("确认转账") : copy("确认 Bancor 交易")}
              </h2>
              <p>{copy("请核对以下实际签名内容。确认后会提交到所示节点。")}</p>
              <dl>
                <dt>{copy("账户")}</dt>
                <dd>{pending.account_name}</dd>
                <dt>{copy("完整公钥")}</dt>
                <dd className="wallet-public">{pending.public_key}</dd>
                <dt>{copy("连接目标")}</dt>
                <dd>
                  {pending.device_label === pending.origin ? pending.origin : `${pending.device_label} · ${pending.origin}`}
                </dd>
                <dt>{copy("资产服务节点")}</dt>
                <dd className="wallet-public">{pending.payload.node_url}</dd>
                <dt>{copy("账本")}</dt>
                <dd className="wallet-public">{pending.payload.ledger_id}</dd>
                {terms.kind === "transfer" ? (
                  <>
                    <dt>{copy("收款账户")}</dt>
                    <dd className="wallet-public">{terms.recipient}</dd>
                    <dt>{copy("转出金额")}</dt>
                    <dd>
                      {displayUnits(terms.amount_units)} {terms.asset}
                    </dd>
                    <dt>Memo</dt>
                    <dd>{terms.memo || copy("无")}</dd>
                  </>
                ) : (
                  <>
                    <dt>{copy("方向")}</dt>
                    <dd>{terms.side === "buy" ? copy("买入 AIC") : copy("卖出 AIC")}</dd>
                    <dt>{copy("支付金额")}</dt>
                    <dd>
                      {displayUnits(terms.input_units)}{" "}
                      {terms.side === "buy" ? "USD" : "AIC"}
                    </dd>
                    <dt>{copy("预计收到")}</dt>
                    <dd>
                      {displayUnits(terms.quoted_output_units)}{" "}
                      {terms.side === "buy" ? "AIC" : "USD"}
                    </dd>
                    <dt>{copy("最低收到")}</dt>
                    <dd>
                      {displayUnits(terms.min_output_units)}{" "}
                      {terms.side === "buy" ? "AIC" : "USD"}
                    </dd>
                    <dt>{copy("滑点上限")}</dt>
                    <dd>{terms.slippage_bps / 100}%</dd>
                  </>
                )}
                <dt>{copy("手续费")}</dt>
                <dd>
                  {displayUnits(terms.fee_units)}{" "}
                  {terms.kind === "transfer"
                    ? terms.asset
                    : terms.side === "buy"
                      ? "USD"
                      : "AIC"}
                </dd>
                <dt>{copy("费用上限")}</dt>
                <dd>{terms.max_fee_bps / 100}%</dd>
                <dt>{copy("本次最多扣除")}</dt>
                <dd>
                  {displayUnits(
                    terms.kind === "transfer"
                      ? (
                          BigInt(terms.amount_units) + BigInt(terms.fee_units)
                        ).toString()
                      : terms.input_units,
                  )}{" "}
                  {terms.kind === "transfer"
                    ? terms.asset
                    : terms.side === "buy"
                      ? "USD"
                      : "AIC"}
                </dd>
                <dt>{copy("有效期至")}</dt>
                <dd>
                  {new Date(
                    pending.payload.expires_at_unix * 1000,
                  ).toLocaleString(lang === "zh" ? "zh-CN" : "en-US")}
                </dd>
              </dl>
              <label className="wallet-label">
                 {copy("交易确认密码")} <input
                  aria-label={copy("交易确认密码")}
                  type="password"
                  autoComplete="off"
                  maxLength={1024}
                  value={signingPassword}
                  disabled={busy}
                  onChange={(e) => setSigningPassword(e.target.value)}
                />
              </label>
              <div className="wallet-actions">
                <button
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      await invoke("wallet_cancel_operation");
                    })
                  }
                >
                   {copy("取消")} </button>
                <button
                  data-wallet-confirm
                  className="wallet-primary"
                  disabled={busy || !signingPassword || retryAfter > 0}
                  onClick={() =>
                    void run(async () => {
                      const value = signingPassword;
                      setSigningPassword("");
                      const result = await invoke<Outcome>("wallet_confirm", {
                        operationId: pending.payload.operation_id,
                        password: value,
                      });
                      setPending(null);
                      setMessage(
                        result.status === "succeeded"
                          ? copy("操作已确认完成。返回资产页面可刷新余额。")
                          : result.status === "pending"
                            ? copy("操作已提交，结果待核实。请返回资产页面核实结果。")
                            : copy("本次操作未执行成功，请返回资产页面查看结果。"),
                      );
                    })
                  }
                >
                  {retryAfter > 0 ? copy(`${retryAfter} 秒后重试`, `Retry in ${retryAfter} seconds`) : copy("确认并签名提交")}
                </button>
              </div>
            </section>
          )}
          {retryAfter > 0 && <p role="status" className="wallet-note">{copy("密码验证暂时受限，请在")} {retryAfter}  {copy("秒后重试。")}</p>}
          {status.unlocked && !pending && (<>
          <form
            className="wallet-card"
            onSubmit={(e) => {
              e.preventDefault();
              void run(async () => {
                await invoke("wallet_create", { name });
                setName("");
                setMessage(copy("账户已生成并安全保存。请立即完成加密备份。"));
              });
            }}
          >
            <h2>{copy("新建资产账号")}</h2>
            <label className="wallet-label">
               {copy("账户名称")} <input
                aria-label={copy("账户名称")}
                required
                maxLength={50}
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={copy("例如：日常交易")}
              />
            </label>
            <button
              data-wallet-create
              disabled={busy || Boolean(pending)}
              className="wallet-primary"
            >
               {copy("新建密钥对并安全保存")} </button>
          </form>
          <section className="wallet-card">
            <h2>{copy("已保存的账号")}</h2>
            {status.accounts.length === 0 ? (
              <p>{copy("尚未创建账号。")}</p>
            ) : (
              status.accounts.map((account) => (
                <article className="wallet-account" key={account.id}>
                  <div className="wallet-bar-head">
                    <strong>{account.name}</strong>
                    <span className="wallet-tag">
                      {account.backed_up ? copy("备份已验证") : copy("尚未备份")}
                    </span>
                  </div>
                  {status.backup_upgrade_accounts.includes(account.id) && <p className="wallet-note" data-wallet-backup-upgrade>
                    {copy("建议更新备份：此账号使用旧版备份。请保存一份新版加密备份，原备份仍可恢复。")}
                  </p>}
                  <p className="wallet-public">{account.public_key}</p>
                  <button
                    disabled={busy}
                    onClick={() => {
                      setBackupId(account.id);
                      setBackupVaultPassword("");
                      setBackupPassword("");
                      setBackupRepeat("");
                    }}
                  >
                     {copy("保存加密备份")} </button>
                  {backupId === account.id && (
                    <form
                      onSubmit={(e) => {
                        e.preventDefault();
                        void run(async () => {
                          if (backupPassword !== backupRepeat) {
                            setError(copy("两次备份密码不一致。"));
                            return;
                          }
                          const value = backupPassword;
                          const vaultPassword = backupVaultPassword;
                          setBackupVaultPassword("");
                          setBackupPassword("");
                          setBackupRepeat("");
                          const saved = await invoke<boolean>("wallet_backup", {
                            accountId: account.id,
                            password: value,
                            vaultPassword,
                          });
                          if (saved) {
                            setBackupId("");
                            setMessage(
                              copy("加密备份已保存，并已读回验证。请另存到安全位置，记住备份密码。"),
                            );
                          }
                        });
                      }}
                    >
                      <p>
                         {copy("导出前需再次验证密钥库密码，完成后自动锁定。备份密码用于换电脑恢复，请妥善保管。")} </p>
                      <p>{copy("请使用 16–128 个字符的独立备份密码，例如多个无关词组成的长密码；避免常用、重复或连续字符。")}</p>
                      <label className="wallet-label">
                         {copy("当前密钥库密码")} <input aria-label={copy("备份确认密码")} type="password" required maxLength={1024}
                          autoComplete="off" value={backupVaultPassword}
                          onChange={(e) => setBackupVaultPassword(e.target.value)} />
                      </label>
                      <label className="wallet-label">
                         {copy("备份密码")} <input
                          type="password"
                          minLength={16}
                          maxLength={128}
                          required
                          autoComplete="off"
                          value={backupPassword}
                          onChange={(e) => setBackupPassword(e.target.value)}
                        />
                      </label>
                      <label className="wallet-label">
                         {copy("再次输入备份密码")} <input
                          type="password"
                          minLength={16}
                          maxLength={128}
                          required
                          autoComplete="off"
                          value={backupRepeat}
                          onChange={(e) => setBackupRepeat(e.target.value)}
                        />
                      </label>
                      <button disabled={busy}>{busy ? copy("正在加密并验证备份…") : copy("选择保存位置并验证备份")}</button>
                    </form>
                  )}
                </article>
              ))
            )}
          </section>
          <section className="wallet-card">
            <button onClick={() => setRestore(!restore)}>
               {copy("从加密备份恢复账号")} </button>
            {restore && (
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  void run(async () => {
                    const value = restorePassword;
                    setRestorePassword("");
                    const result = await invoke("wallet_restore", {
                      password: value,
                      name: name || copy("恢复的账户"),
                    });
                    if (result) {
                      setRestore(false);
                      setMessage(copy("账号已恢复，公钥校验通过。"));
                    }
                  });
                }}
              >
                <label className="wallet-label">
                   {copy("恢复后的账户名称")} <input
                    maxLength={50}
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder={copy("恢复的账户")}
                  />
                </label>
                <label className="wallet-label">
                   {copy("备份密码")} <input
                    type="password"
                    minLength={12}
                    required
                    autoComplete="off"
                    value={restorePassword}
                    onChange={(e) => setRestorePassword(e.target.value)}
                  />
                </label>
                <button disabled={busy}>{copy("选择加密备份文件并恢复")}</button>
              </form>
            )}
          </section>
          </>)}
        </>
      )}
      {error && (
        <p className="wallet-error" role="alert">
          {copy(error)}
        </p>
      )}
      {message && (
        <p className="wallet-notice" role="status">
          {copy(message)}
        </p>
      )}
      <p className="wallet-footnote">
         {copy("解锁后最多保留 5 分钟，操作不会延长时限；关闭安全窗口、离开应用或系统锁屏会提前锁定。丢失备份及密码可能无法恢复资产账户。账户与硬件绑定、奖励配置相互独立。")} </p>
    </main>
  );
}
createRoot(document.getElementById("root")!).render(<Manager />);
