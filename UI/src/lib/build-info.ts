function requiredUiVersion(value: string | undefined): string {
  const configured = value?.trim();
  if (!configured) throw new Error("UI build version is missing.");
  return configured;
}

export const UI_BUILD_VERSION = requiredUiVersion(
  typeof __APP_UI_VERSION__ === "undefined"
    ? globalThis.__APP_UI_VERSION__
    : __APP_UI_VERSION__,
);
