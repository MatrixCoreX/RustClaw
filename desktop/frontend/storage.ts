const globalKeys = new Set(['agent-runtime.monitor.lang', 'agent-runtime.monitor.themeMode']);
export function scopedStorage(storage: Storage, prefix: string): Storage {
  const resolve = (key: string) => globalKeys.has(key) ? key : prefix + key;
  const keys = () => Array.from({length: storage.length}, (_, i) => storage.key(i)).filter((key): key is string => key !== null && key.startsWith(prefix));
  return {
    get length() { return keys().length; },
    clear() { for (const key of keys()) storage.removeItem(key); },
    key(index) { return keys()[index]?.slice(prefix.length) ?? null; },
    getItem(key) { return storage.getItem(resolve(key)); },
    setItem(key, value) {
      // No authentication material or insecure transport approvals in browser persistence.
      if (/userKey|csrf|allowInsecureHttpPrivateKeySession/.test(key)) return;
      storage.setItem(resolve(key), value);
    },
    removeItem(key) { storage.removeItem(resolve(key)); },
  };
}
export function forgetDeviceStorage(storage: Storage, profileId: string) {
  const prefix = `agent-runtime.desktop.${profileId}.`;
  for (let i = storage.length - 1; i >= 0; i--) {
    const key = storage.key(i);
    if (key?.startsWith(prefix)) storage.removeItem(key);
  }
}
