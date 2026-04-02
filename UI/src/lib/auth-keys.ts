export async function copyAuthKeyValue(options: {
  keyId?: number | null;
  plaintextKey?: string | null;
  fetchFullAuthKey: (keyId: number) => Promise<string>;
  writeClipboard: (value: string) => Promise<void>;
}): Promise<string> {
  const plaintextKey = options.plaintextKey?.trim() ?? "";
  if (plaintextKey) {
    await options.writeClipboard(plaintextKey);
    return plaintextKey;
  }

  if (options.keyId == null) {
    throw new Error("missing auth key id");
  }

  const fullKey = (await options.fetchFullAuthKey(options.keyId)).trim();
  if (!fullKey) {
    throw new Error("empty auth key");
  }

  await options.writeClipboard(fullKey);
  return fullKey;
}
