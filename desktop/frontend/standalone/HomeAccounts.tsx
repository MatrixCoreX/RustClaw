import { copy, useLanguage } from "../i18n";
import { useRef, useState } from "react";
import { NniPublicKeyDisplay } from "../../../UI/src/components/NniPublicKeyDisplay";
import { openWallet, selectAccount, useWallet } from "../wallet/store";
import { walletError } from "../wallet/errors";
import { shortPublic } from "../wallet/amounts";
import "./standalone.css";

export function HomeAccounts({ onOpen, busy = false }: {
  onOpen: (accountId: string) => void;
  busy?: boolean;
}) {
  const { t } = useLanguage();
  const wallet = useWallet();
  const [error, setError] = useState("");
  const [selecting, setSelecting] = useState(false);
  const selectingRef = useRef(false);
  const account = wallet.accounts.find(a => a.id === wallet.selectedId) ?? wallet.accounts[0];
  const disabled = busy || selecting;
  const choose = async (id: string) => {
    if (selectingRef.current || busy) return;
    selectingRef.current = true; setSelecting(true); setError("");
    try { await selectAccount(id); }
    catch (e) { setError(walletError(e)); }
    finally { selectingRef.current = false; setSelecting(false); }
  };
  const manage = () => {
    setError("");
    void openWallet().catch(e => setError(walletError(e)));
  };
  return <section className="desktop-home-wallet" aria-labelledby="home-accounts-title">
    <div className="desktop-list-head">
      <div><h2 id="home-accounts-title">{copy("本地资产账户")} <span>{t(`共 ${wallet.accounts.length} 个`, `${wallet.accounts.length} accounts`)}</span></h2>
        <p>{copy("直接查看资产、转账或进行 Bancor 交易，无需登录设备。")}</p></div>
      <button type="button" disabled={disabled} onClick={manage}>{copy("管理本地账号")}</button>
    </div>
    {account ? <article className="desktop-card desktop-home-account" data-home-account={account.id}>
        <div className="desktop-home-account-heading"><label htmlFor="home-asset-account">{copy("选择本地账户")}</label>
          <span className="desktop-badge">{account.backed_up ? copy("备份已验证") : copy("尚未备份")}</span></div>
        <select id="home-asset-account" className="desktop-home-account-select" aria-label={copy("首页本地资产账户")}
          value={account.id} disabled={disabled} onChange={e => void choose(e.target.value)}>
          {wallet.accounts.map(a => <option key={a.id} value={a.id}>{t(`桌面本地账号（共 ${wallet.accounts.length} 个）`, `Local account (${wallet.accounts.length} total)`)} · {a.name} · {shortPublic(a.public_key)}</option>)}
        </select>
        <span className="desktop-home-key-label">{copy("账户公钥 · 收款地址")}</span>
        <NniPublicKeyDisplay key={account.id} value={account.public_key} t={t} allowFormatSwitch={false} copyButton="compact"
          className="desktop-home-public-key" valueClassName="text-xs" />
        <div className="desktop-home-account-footer"><small>{copy("私钥已在本机加密保存")}{!account.backed_up ? copy("，交易前请先备份。") : t("。", ".")}</small>
          <div className="desktop-actions"><button type="button" className="primary" disabled={disabled} data-home-open="assets" onClick={() => onOpen(account.id)}>{copy("进入资产")}</button></div></div>
      </article> : <div className="desktop-card desktop-home-wallet-empty">
      <div><h3>{copy("创建你的本地资产账户")}</h3><p>{copy("生成密钥对或从加密备份恢复，之后可独立使用资产和交易页面。")}</p></div>
      <button type="button" className="primary" disabled={disabled} onClick={manage}>{copy("创建 / 恢复账户")}</button>
    </div>}
    {(error || wallet.error) && <p className="desktop-error" role="alert">{copy(error || wallet.error)}</p>}
  </section>;
}
