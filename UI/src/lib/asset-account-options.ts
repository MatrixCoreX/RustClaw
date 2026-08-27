import { nniPublicKeyFormats, shortenHex } from "./nni-display";

type Translate = (zh: string, en: string) => string;

export interface AssetAccountOption {
  id: string;
  publicKey: string;
  source: "local_binding" | "external";
  label?: string;
}

export function buildAssetAccountOptions(
  localBindingPublicKey: string | null | undefined,
  additionalAccounts: readonly AssetAccountOption[] = [],
): AssetAccountOption[] {
  const options: AssetAccountOption[] = [];
  const seenPublicKeys = new Set<string>();
  const seenIds = new Set<string>();
  const localPublicKey = localBindingPublicKey?.trim();
  if (localPublicKey) {
    const id = `local-binding:${localPublicKey}`;
    options.push({
      id,
      publicKey: localPublicKey,
      source: "local_binding",
    });
    seenIds.add(id);
    seenPublicKeys.add(localPublicKey);
  }
  for (const account of additionalAccounts) {
    const id = account.id.trim();
    const publicKey = account.publicKey.trim();
    if (!id || !publicKey || seenIds.has(id) || seenPublicKeys.has(publicKey)) continue;
    options.push({ ...account, id, publicKey });
    seenIds.add(id);
    seenPublicKeys.add(publicKey);
  }
  return options;
}

export function formatAssetAccountOption(
  account: AssetAccountOption,
  t: Translate,
  options: { fullPublicKey?: boolean } = {},
): string {
  const defaultLabel = account.source === "local_binding"
    ? t("本机绑定账户", "Local bound account")
    : t("其他资产账户", "Other asset account");
  const compactPublicKey = nniPublicKeyFormats(account.publicKey)?.compact ?? account.publicKey;
  const visiblePublicKey = options.fullPublicKey
    ? compactPublicKey
    : shortenHex(compactPublicKey, 8, 8);
  return `${account.label?.trim() || defaultLabel} · ${visiblePublicKey}`;
}
