import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { PRODUCT_DISPLAY_NAME } from "../../../UI/src/lib/product-identity";
import { displayUnits } from "./amounts";
import { walletError } from "./errors";
import type { Confirmation, Outcome, WalletStatus } from "./types";
import "./wallet.css";

document.title = `${PRODUCT_DISPLAY_NAME} · 本地资产安全窗口`;
function Manager() {
  const [status, setStatus] = useState<WalletStatus>({
    initialized: false,
    unlocked: false,
    accounts: [],
  });
  const [pending, setPending] = useState<Confirmation | null>(null);
  const [password, setPassword] = useState("");
  const [signingPassword, setSigningPassword] = useState("");
  const [repeat, setRepeat] = useState("");
  const [name, setName] = useState("");
  const [backupId, setBackupId] = useState("");
  const [backupPassword, setBackupPassword] = useState("");
  const [backupRepeat, setBackupRepeat] = useState("");
  const [restore, setRestore] = useState(false);
  const [restorePassword, setRestorePassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [theme, setTheme] = useState(
    () => localStorage.getItem("agent-runtime.monitor.themeMode") ?? "light",
  );
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
    } finally {
      setBusy(false);
    }
  };
  const unlock = () =>
    run(async () => {
      if (!status.initialized && password !== repeat) {
        setError("两次输入的密码不一致。");
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
            {PRODUCT_DISPLAY_NAME} · 本机安全窗口
          </span>
          <h1>本地资产账户</h1>
        </div>
        <button onClick={() => setTheme(theme === "light" ? "dark" : "light")}>
          {theme === "light" ? "深色" : "浅色"}外观
        </button>
      </header>
      <p>私钥在这台电脑加密保存。查看资产无需解锁；每笔交易须输入密码确认。</p>
      {!status.unlocked && !pending ? (
        <form
          className="wallet-card"
          onSubmit={(e) => {
            e.preventDefault();
            void unlock();
          }}
        >
          <h2>{status.initialized ? "解锁密钥库" : "设置密钥库密码"}</h2>
          <p>
            {status.initialized
              ? "解锁后可创建、备份或恢复账号。查看资产不需要解锁。"
              : "设置至少 12 个字符的密码。密码不会发送到设备或资产服务。"}
          </p>
          <label className="wallet-label">
            密钥库密码
            <input
              aria-label="密钥库密码"
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
              再次输入密码
              <input
                type="password"
                autoComplete="off"
                minLength={12}
                value={repeat}
                onChange={(e) => setRepeat(e.target.value)}
                required
              />
            </label>
          )}
          <button className="wallet-primary" disabled={busy}>
            {busy ? "正在处理…" : status.initialized ? "解锁" : "创建密钥库"}
          </button>
        </form>
      ) : (
        <>
          <div className="wallet-bar-head">
            <span className="wallet-tag">{status.unlocked ? "已解锁" : "已锁定"}</span>
            <button
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  await invoke("wallet_lock");
                })
              }
            >
              立即锁定
            </button>
          </div>
          {pending && terms && (
            <section
              className="wallet-card wallet-confirm"
              aria-label="确认资产操作"
            >
              <h2>
                {terms.kind === "transfer" ? "确认转账" : "确认 Bancor 交易"}
              </h2>
              <p>请核对以下实际签名内容。确认后会提交到所示节点。</p>
              <dl>
                <dt>账户</dt>
                <dd>{pending.account_name}</dd>
                <dt>完整公钥</dt>
                <dd className="wallet-public">{pending.public_key}</dd>
                <dt>连接设备</dt>
                <dd>
                  {pending.device_label} · {pending.origin}
                </dd>
                <dt>资产服务节点</dt>
                <dd className="wallet-public">{pending.payload.node_url}</dd>
                <dt>账本</dt>
                <dd className="wallet-public">{pending.payload.ledger_id}</dd>
                {terms.kind === "transfer" ? (
                  <>
                    <dt>收款账户</dt>
                    <dd className="wallet-public">{terms.recipient}</dd>
                    <dt>转出金额</dt>
                    <dd>
                      {displayUnits(terms.amount_units)} {terms.asset}
                    </dd>
                    <dt>Memo</dt>
                    <dd>{terms.memo || "无"}</dd>
                  </>
                ) : (
                  <>
                    <dt>方向</dt>
                    <dd>{terms.side === "buy" ? "买入 AIC" : "卖出 AIC"}</dd>
                    <dt>支付金额</dt>
                    <dd>
                      {displayUnits(terms.input_units)}{" "}
                      {terms.side === "buy" ? "USD" : "AIC"}
                    </dd>
                    <dt>预计收到</dt>
                    <dd>
                      {displayUnits(terms.quoted_output_units)}{" "}
                      {terms.side === "buy" ? "AIC" : "USD"}
                    </dd>
                    <dt>最低收到</dt>
                    <dd>
                      {displayUnits(terms.min_output_units)}{" "}
                      {terms.side === "buy" ? "AIC" : "USD"}
                    </dd>
                    <dt>滑点上限</dt>
                    <dd>{terms.slippage_bps / 100}%</dd>
                  </>
                )}
                <dt>手续费</dt>
                <dd>
                  {displayUnits(terms.fee_units)}{" "}
                  {terms.kind === "transfer"
                    ? terms.asset
                    : terms.side === "buy"
                      ? "USD"
                      : "AIC"}
                </dd>
                <dt>费用上限</dt>
                <dd>{terms.max_fee_bps / 100}%</dd>
                <dt>本次最多扣除</dt>
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
                <dt>有效期至</dt>
                <dd>
                  {new Date(
                    pending.payload.expires_at_unix * 1000,
                  ).toLocaleString()}
                </dd>
              </dl>
              <label className="wallet-label">
                交易确认密码
                <input
                  aria-label="交易确认密码"
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
                  取消
                </button>
                <button
                  data-wallet-confirm
                  className="wallet-primary"
                  disabled={busy || !signingPassword}
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
                          ? "操作已确认完成。返回资产页面可刷新余额。"
                          : result.status === "pending"
                            ? "操作已提交，结果待核实。请返回资产页面核实结果。"
                            : "本次操作未执行成功，请返回资产页面查看结果。",
                      );
                    })
                  }
                >
                  确认并签名提交
                </button>
              </div>
            </section>
          )}
          {status.unlocked && !pending && (<>
          <form
            className="wallet-card"
            onSubmit={(e) => {
              e.preventDefault();
              void run(async () => {
                await invoke("wallet_create", { name });
                setName("");
                setMessage("账户已生成并安全保存。请立即完成加密备份。");
              });
            }}
          >
            <h2>新建资产账号</h2>
            <label className="wallet-label">
              账户名称
              <input
                aria-label="账户名称"
                required
                maxLength={50}
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="例如：日常交易"
              />
            </label>
            <button
              data-wallet-create
              disabled={busy || Boolean(pending)}
              className="wallet-primary"
            >
              新建密钥对并安全保存
            </button>
          </form>
          <section className="wallet-card">
            <h2>已保存的账号</h2>
            {status.accounts.length === 0 ? (
              <p>尚未创建账号。</p>
            ) : (
              status.accounts.map((account) => (
                <article className="wallet-account" key={account.id}>
                  <div className="wallet-bar-head">
                    <strong>{account.name}</strong>
                    <span className="wallet-tag">
                      {account.backed_up ? "备份已验证" : "尚未备份"}
                    </span>
                  </div>
                  <p className="wallet-public">{account.public_key}</p>
                  <button
                    disabled={busy}
                    onClick={() => {
                      setBackupId(account.id);
                      setBackupPassword("");
                      setBackupRepeat("");
                    }}
                  >
                    保存加密备份
                  </button>
                  {backupId === account.id && (
                    <form
                      onSubmit={(e) => {
                        e.preventDefault();
                        void run(async () => {
                          if (backupPassword !== backupRepeat) {
                            setError("两次备份密码不一致。");
                            return;
                          }
                          const value = backupPassword;
                          setBackupPassword("");
                          setBackupRepeat("");
                          const saved = await invoke<boolean>("wallet_backup", {
                            accountId: account.id,
                            password: value,
                          });
                          if (saved) {
                            setBackupId("");
                            setMessage(
                              "加密备份已保存，并已读回验证。请另存到安全位置，记住备份密码。",
                            );
                          }
                        });
                      }}
                    >
                      <p>
                        备份密码可以与密钥库密码不同。换电脑恢复时需要此密码。
                      </p>
                      <label className="wallet-label">
                        备份密码
                        <input
                          type="password"
                          minLength={12}
                          required
                          autoComplete="off"
                          value={backupPassword}
                          onChange={(e) => setBackupPassword(e.target.value)}
                        />
                      </label>
                      <label className="wallet-label">
                        再次输入备份密码
                        <input
                          type="password"
                          minLength={12}
                          required
                          autoComplete="off"
                          value={backupRepeat}
                          onChange={(e) => setBackupRepeat(e.target.value)}
                        />
                      </label>
                      <button disabled={busy}>选择保存位置并验证备份</button>
                    </form>
                  )}
                </article>
              ))
            )}
          </section>
          <section className="wallet-card">
            <button onClick={() => setRestore(!restore)}>
              从加密备份恢复账号
            </button>
            {restore && (
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  void run(async () => {
                    const value = restorePassword;
                    setRestorePassword("");
                    const result = await invoke("wallet_restore", {
                      password: value,
                      name: name || "恢复的账户",
                    });
                    if (result) {
                      setRestore(false);
                      setMessage("账号已恢复，公钥校验通过。");
                    }
                  });
                }}
              >
                <label className="wallet-label">
                  恢复后的账户名称
                  <input
                    maxLength={50}
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder="恢复的账户"
                  />
                </label>
                <label className="wallet-label">
                  备份密码
                  <input
                    type="password"
                    minLength={12}
                    required
                    autoComplete="off"
                    value={restorePassword}
                    onChange={(e) => setRestorePassword(e.target.value)}
                  />
                </label>
                <button disabled={busy}>选择加密备份文件并恢复</button>
              </form>
            )}
          </section>
          </>)}
        </>
      )}
      {error && (
        <p className="wallet-error" role="alert">
          {error}
        </p>
      )}
      {message && (
        <p className="wallet-notice" role="status">
          {message}
        </p>
      )}
      <p className="wallet-footnote">
        解锁后最多保留 5
        分钟；离开应用或系统锁屏会提前锁定。丢失备份及密码可能无法恢复资产账户。账户与硬件绑定、奖励配置相互独立。
      </p>
    </main>
  );
}
createRoot(document.getElementById("root")!).render(<Manager />);
