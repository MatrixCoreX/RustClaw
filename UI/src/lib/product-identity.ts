function requiredDisplayName(value: string | undefined): string {
  const configured = value?.trim();
  if (!configured) throw new Error("Product identity display_name is missing.");
  return configured;
}

export const PRODUCT_DISPLAY_NAME = requiredDisplayName(
  typeof __APP_DISPLAY_NAME__ === "undefined"
    ? globalThis.__APP_DISPLAY_NAME__
    : __APP_DISPLAY_NAME__,
);
export const AUTH_KEY_HEADER = "X-Agent-Key";
export const CLIENT_ORIGIN_HEADER = "X-Agent-Client";

const PRODUCT_NAME_TOKEN = "{product_name}";
const STORAGE_NAMESPACE = "agent-runtime";

export function productCopy(text: string): string {
  return text.split(PRODUCT_NAME_TOKEN).join(PRODUCT_DISPLAY_NAME);
}

export function appStorageKey(suffix: string): string {
  return `${STORAGE_NAMESPACE}.${suffix}`;
}
