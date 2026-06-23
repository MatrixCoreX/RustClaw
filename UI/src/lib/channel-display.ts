import type { ChannelName } from "../types/api";

export type UiLanguage = "zh" | "en";

const CHANNEL_NAMES = new Set<string>(["telegram", "whatsapp", "ui", "wechat", "feishu", "lark"]);

function copy(lang: UiLanguage, zh: string, en: string): string {
  return lang === "zh" ? zh : en;
}

function isChannelName(value: string): value is ChannelName {
  return CHANNEL_NAMES.has(value);
}

export function channelLabel(channel: ChannelName, lang: UiLanguage): string {
  const labels: Record<ChannelName, string> = {
    telegram: "Telegram",
    whatsapp: "WhatsApp",
    ui: "UI",
    wechat: copy(lang, "微信", "WeChat"),
    feishu: "Feishu",
    lark: "Lark",
  };
  return labels[channel];
}

export function boundChannelsLabel(channels: string[] | null | undefined, lang: UiLanguage): string {
  if (!channels?.length) return "";
  return channels
    .map((channel) => (isChannelName(channel) ? channelLabel(channel, lang) : channel))
    .join(" / ");
}
