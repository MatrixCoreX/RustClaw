import path from 'node:path';
import { normalizePath } from '../../UI/node_modules/vite/dist/node/index.js';

/** Desktop-only slots: one shared page layout, native account data and signing. */
export function adaptWalletUi(source: string, id: string, root: string, uiRoot: string): string {
  const assets = id === uiRoot + 'components/AssetsPage.tsx';
  const bancor = id === uiRoot + 'components/BancorPage.tsx';
  const transfer = id === uiRoot + 'components/AssetTransferDialog.tsx';
  if (!assets && !bancor && !transfer) return source;
  let text = source;
  const once = (before: string, after: string) => {
    if (text.split(before).length !== 2) throw new Error(`Shared wallet UI contract changed: ${id}: ${before}`);
    text = text.replace(before, after);
  };
  const imports = ['useDesktopAssetAccount'];
  if (assets || bancor) {
    imports.push('LocalAccountHistory');
    const selector = assets
      ? /\{selectedAssetAccount \? \(\n            <label[^>]+data-assets-account-selector="true">[\s\S]*?            <\/div>\n          \)\}/g
      : /\{selectedAssetAccount \? \(\n              <label[^>]+data-bancor-account-selector="true">[\s\S]*?            \) : null\}/g;
    const matches = [...text.matchAll(selector)];
    if (matches.length !== 1) throw new Error(`Shared UI account selector contract changed: ${id}`);
    once(matches[0][0], `<AccountSelector page="${assets ? 'assets' : 'bancor'}" hardwarePublicKey={assetOwnerPubkey} t={t} />`);
    text = `import { AccountSelector } from ${JSON.stringify(normalizePath(path.resolve(root, 'frontend/wallet/AccountSelector.tsx')))};\n` + text;
  }
  if (assets) {
    once('  const assetAccountOptions = useMemo(', '  const desktopAccount = useDesktopAssetAccount();\n  const assetAccountOptions = useMemo(');
    once('  const statusMessage = selectedAssetAccount?.source', '  const statusMessage = desktopAccount ? desktopAccount.statusMessage : selectedAssetAccount?.source');
    once('  useEffect(() => {\n    const publicKey = selectedAssetAccount?.publicKey', '  useEffect(() => {\n    if (desktopAccount) return;\n    const publicKey = selectedAssetAccount?.publicKey');
    for (const name of ['historySource', 'historyDirection']) {
      once(`value={${name}}`, `value={${name}}\n              disabled={Boolean(desktopAccount)}\n              title={desktopAccount ? t("当前服务暂不支持流水筛选。", "This service does not support activity filters yet.") : undefined}`);
    }
    once('        {!selectedAssetAccount ? (', '        {desktopAccount ? <LocalAccountHistory t={t} /> : <>\n        {!selectedAssetAccount ? (');
    once('      </section>\n\n      {transferMessage && !transferDialogOpen', '        </>}\n      </section>\n\n      {transferMessage && !transferDialogOpen');
  }
  if (bancor) {
    once('  const [side, setSide] = useState<BancorTradeSide>', '  const desktopAccount = useDesktopAssetAccount();\n  const [side, setSide] = useState<BancorTradeSide>');
    once('disabled={accountLoading || !signingDeviceReady}', 'disabled={accountLoading || (desktopAccount ? !desktopAccount.runtime.ready : !signingDeviceReady)}');
    once('{assetSigningReady\n                  ? t(', '{desktopAccount ? (assetSigningReady ? t("交易将在本地安全窗口确认并签名。", "Confirm and sign the trade in the local secure window.") : desktopAccount.statusMessage) : assetSigningReady\n                  ? t(');
    once('{t(\n                "强制流动性算法。', '{desktopAccount ? t("强制流动性算法。使用当前桌面账号交易，在本地安全窗口确认并签名。", "A forced-liquidity algorithm. Trade with the selected desktop account and confirm in the local secure window.") : t(\n                "强制流动性算法。');
    once('{t("这里只显示当前设备公钥签署的交易。", "Only trades signed by this device key are shown.")}', '{desktopAccount ? t("当前桌面账号的交易资产变动。", "Trading asset movements for the selected desktop account.") : t("这里只显示当前设备公钥签署的交易。", "Only trades signed by this device key are shown.")}');
    once('{account?.total ?? 0} {t("笔", "trades")}', '{desktopAccount ? t("当前账户", "Current account") : <>{account?.total ?? 0} {t("笔", "trades")}</>}');
    once('          <div className="mt-4 grid gap-2">\n            {account?.trades.length', '          {desktopAccount ? <LocalAccountHistory t={t} tradesOnly /> : <>\n          <div className="mt-4 grid gap-2">\n            {account?.trades.length');
    once('        </article>\n\n        <article', '          </>}\n        </article>\n\n        <article');
  }
  if (transfer) {
    imports.push('NativeTransferAuthorization');
    once('  const privateKeyOperationsAllowed = nniPrivateKeyOperationsAllowed();', '  const desktopAccount = useDesktopAssetAccount();\n  const privateKeyOperationsAllowed = !desktopAccount && nniPrivateKeyOperationsAllowed();');
    once('    if (result) setCompleted(true);', '    if (result) { if (desktopAccount) onClose(); else setCompleted(true); }');
    once('            <fieldset>', '            {desktopAccount ? <NativeTransferAuthorization t={t} /> : <fieldset>');
    once('            </fieldset>', '            </fieldset>}');
    once('            {!privateKeyOperationsAllowed ? (', '            {!privateKeyOperationsAllowed && !desktopAccount ? (');
    once('{authorizationMode === "delegated_hardware" ? t("硬件设备代签", "Hardware signing") : t("资产私钥签名", "Asset private key")}', '{desktopAccount ? t("桌面密钥库签名", "Desktop vault signing") : authorizationMode === "delegated_hardware" ? t("硬件设备代签", "Hardware signing") : t("资产私钥签名", "Asset private key")}');
  }
  return `import { ${imports.join(', ')} } from ${JSON.stringify(normalizePath(path.resolve(root, 'frontend/wallet/shared-parts.tsx')))};\n` + text;
}
