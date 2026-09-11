import { useId, useState } from "react";
import { NniPublicKeyDisplay } from "../../../UI/src/components/NniPublicKeyDisplay";
import { lockWallet, openWallet, selectAccount, useWallet } from "./store";
import { shortPublic } from "./amounts";
import { walletError } from "./errors";
import { useDesktopAssetAccount } from "./context";

export function AccountSelector({
  hardwarePublicKey,
  page,
  t,
}: {
  hardwarePublicKey?: string | null;
  page: "assets" | "bancor";
  t: (zh: string, en: string) => string;
}) {
  const desktopAccount = useDesktopAssetAccount();
  hardwarePublicKey = desktopAccount
    ? desktopAccount.hardwarePublicKey
    : hardwarePublicKey;
  const wallet = useWallet();
  const id = useId();
  const account = wallet.accounts.find((a) => a.id === wallet.selectedId);
  const publicKey = wallet.selectedId ? account?.public_key : hardwarePublicKey;
  const [error, setError] = useState("");
  const run = (fn: () => Promise<unknown>) => {
    setError("");
    void fn().catch((e) => setError(walletError(e)));
  };
  return (
    <div
      className={`desktop-account-selector grid min-w-0 gap-1.5 ${page === "bancor" ? "bancor-trade-account-selector mb-2" : ""}`}
      data-assets-account-selector={page === "assets" ? "true" : undefined}
      data-bancor-account-selector={page === "bancor" ? "true" : undefined}
    >
      <div className="desktop-account-selector-heading">
        <label htmlFor={id}>
          {page === "assets"
            ? t("资产账户", "Asset account")
            : t("交易账户", "Trading account")}
        </label>
        <div className="desktop-account-actions">
          <button type="button" onClick={() => run(openWallet)}>
            {t("管理本地账号", "Manage local accounts")}
          </button>
          {wallet.unlocked && (
            <button type="button" onClick={() => run(lockWallet)}>
              {t("锁定", "Lock")}
            </button>
          )}
        </div>
      </div>
      <select
        id={id}
        className="theme-input w-full font-mono text-xs"
        aria-label="桌面资产账户"
        value={wallet.selectedId}
        onChange={(e) => run(() => selectAccount(e.target.value))}
      >
        <option value="">
          {t("硬件设备绑定账号", "Hardware-bound account")} ·{" "}
          {hardwarePublicKey
            ? shortPublic(hardwarePublicKey)
            : t("未绑定", "Not bound")}
        </option>
        {wallet.accounts.map((a) => (
          <option key={a.id} value={a.id}>
            {t("桌面本地账号", "Local account")} · {a.name} ·{" "}
            {shortPublic(a.public_key)}
          </option>
        ))}
      </select>
      {publicKey && (
        <NniPublicKeyDisplay
          value={publicKey}
          t={t}
          className="mt-0.5"
          valueClassName="text-[10px] leading-4 text-[var(--theme-text-muted)]"
          allowFormatSwitch={false}
          copyButton="compact"
        />
      )}
      {wallet.selectedId && (
        <p className="desktop-account-status">
          {t(
            "查看资产无需解锁；每笔买卖或转账需输入密钥库密码确认。",
            "Viewing assets needs no unlock. Enter your vault password for each trade or transfer.",
          )}
        </p>
      )}
      {account && !account.backed_up && (
        <p className="desktop-account-status">
          {t(
            "请在管理本地账号中完成加密备份。",
            "Create an encrypted backup in local account management.",
          )}
        </p>
      )}
      {(error || wallet.error) && (
        <p role="alert" className="wallet-error">
          {error || wallet.error}
        </p>
      )}
    </div>
  );
}
