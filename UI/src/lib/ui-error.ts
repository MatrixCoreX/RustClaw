export type UiErrorTranslate = (zh: string, en: string) => string;

const MACHINE_ERROR_TOKEN = /^[a-z][a-z0-9]*(?:[._:-][a-z0-9]+)+$/i;

export function formatUiError(
  cause: unknown,
  t: UiErrorTranslate,
  fallbackZh: string,
  fallbackEn: string,
): string {
  const message = cause instanceof Error
    ? cause.message.trim()
    : typeof cause === "string"
      ? cause.trim()
      : "";
  if (!message || MACHINE_ERROR_TOKEN.test(message)) return t(fallbackZh, fallbackEn);
  return message;
}
