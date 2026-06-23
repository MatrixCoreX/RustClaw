import { useMemo, useState } from "react";

import type {
  ApiResponse,
  FeishuConfigResponse,
  TelegramBotConfigItem,
  TelegramConfigResponse,
  WechatConfigResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

function buildDefaultTelegramBot(): TelegramBotConfigItem {
  return {
    name: "primary",
    bot_token: "",
    bot_token_configured: false,
    bot_token_masked: null,
    agent_id: "main",
    allowlist: [],
    access_mode: "public",
    allowed_telegram_usernames: [],
    is_primary: true,
  };
}

export interface UseChannelConfigRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
}

export function useChannelConfigRuntime({ apiFetch, t }: UseChannelConfigRuntimeParams) {
  const [wechatConfigLoading, setWechatConfigLoading] = useState(false);
  const [wechatConfigError, setWechatConfigError] = useState<string | null>(null);
  const [wechatConfigData, setWechatConfigData] = useState<WechatConfigResponse | null>(null);
  const [feishuConfigLoading, setFeishuConfigLoading] = useState(false);
  const [feishuConfigError, setFeishuConfigError] = useState<string | null>(null);
  const [feishuConfigData, setFeishuConfigData] = useState<FeishuConfigResponse | null>(null);
  const [telegramConfigLoading, setTelegramConfigLoading] = useState(false);
  const [telegramConfigError, setTelegramConfigError] = useState<string | null>(null);
  const [telegramConfigData, setTelegramConfigData] = useState<TelegramConfigResponse | null>(null);
  const [telegramConfigDraft, setTelegramConfigDraft] = useState<TelegramConfigResponse | null>(null);
  const [telegramConfigSaving, setTelegramConfigSaving] = useState(false);
  const [telegramConfigSaveMessage, setTelegramConfigSaveMessage] = useState<string | null>(null);

  const fetchWechatConfig = async () => {
    setWechatConfigLoading(true);
    setWechatConfigError(null);
    try {
      const res = await apiFetch(`/v1/wechat/config`);
      const body = (await res.json()) as ApiResponse<WechatConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `WeChat config fetch failed (${res.status})`);
      }
      setWechatConfigData(body.data);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setWechatConfigError(message);
    } finally {
      setWechatConfigLoading(false);
    }
  };

  const fetchFeishuConfig = async () => {
    setFeishuConfigLoading(true);
    setFeishuConfigError(null);
    try {
      const res = await apiFetch(`/v1/feishu/config`);
      const body = (await res.json()) as ApiResponse<FeishuConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `Feishu config fetch failed (${res.status})`);
      }
      setFeishuConfigData(body.data);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setFeishuConfigError(message);
    } finally {
      setFeishuConfigLoading(false);
    }
  };

  const fetchTelegramConfig = async () => {
    setTelegramConfigLoading(true);
    setTelegramConfigError(null);
    try {
      const res = await apiFetch(`/v1/telegram/config`);
      const body = (await res.json()) as ApiResponse<TelegramConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `Telegram config fetch failed (${res.status})`);
      }
      setTelegramConfigData(body.data);
      setTelegramConfigDraft(body.data);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setTelegramConfigError(message);
    } finally {
      setTelegramConfigLoading(false);
    }
  };

  const setTelegramPrimaryBotDraftField = (key: keyof TelegramBotConfigItem, value: TelegramBotConfigItem[keyof TelegramBotConfigItem]) => {
    setTelegramConfigDraft((prev) => {
      if (!prev) return prev;
      const bots = prev.bots.length > 0 ? [...prev.bots] : [buildDefaultTelegramBot()];
      const primaryIndex = bots.findIndex((bot) => bot.is_primary);
      const targetIndex = primaryIndex >= 0 ? primaryIndex : 0;
      bots[targetIndex] = {
        ...(bots[targetIndex] ?? buildDefaultTelegramBot()),
        [key]: value,
        is_primary: true,
      };
      return { ...prev, bots };
    });
  };

  const saveTelegramConfig = async () => {
    if (!telegramConfigDraft) return;
    setTelegramConfigSaving(true);
    setTelegramConfigSaveMessage(null);
    setTelegramConfigError(null);
    try {
      const bots = telegramConfigDraft.bots.length > 0 ? telegramConfigDraft.bots : [buildDefaultTelegramBot()];
      const normalizedBots = bots.map((bot, index) => ({
        ...bot,
        name: bot.name?.trim() || (index === 0 ? "primary" : `bot-${index + 1}`),
        bot_token: bot.bot_token?.trim() || "",
        agent_id: bot.agent_id?.trim() || "main",
        is_primary: index === 0 ? true : bot.is_primary,
      }));
      const res = await apiFetch(`/v1/telegram/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          bots: normalizedBots,
          agents: telegramConfigDraft.agents ?? [],
        }),
      });
      const body = (await res.json()) as ApiResponse<TelegramConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `Telegram config save failed (${res.status})`);
      }
      setTelegramConfigData(body.data);
      setTelegramConfigDraft(body.data);
      setTelegramConfigSaveMessage(
        t(
          "Telegram 配置已保存。下一步请启动 telegramd，然后发一条测试消息。",
          "Telegram config was saved. Next, start telegramd and send a test message.",
        ),
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setTelegramConfigError(message);
    } finally {
      setTelegramConfigSaving(false);
    }
  };

  const primaryTelegramBot = useMemo(() => {
    const bots = telegramConfigDraft?.bots ?? telegramConfigData?.bots ?? [];
    return bots.find((bot) => bot.is_primary) ?? bots[0] ?? buildDefaultTelegramBot();
  }, [telegramConfigData, telegramConfigDraft]);

  const telegramBotTokenConfigured = useMemo(() => {
    const token = primaryTelegramBot.bot_token?.trim() || "";
    return (token.length > 0 && token !== "REPLACE_ME") || primaryTelegramBot.bot_token_configured === true;
  }, [primaryTelegramBot]);

  const hasUnsavedTelegramConfigChanges = useMemo(() => {
    if (!telegramConfigData || !telegramConfigDraft) return false;
    return JSON.stringify(telegramConfigData) !== JSON.stringify(telegramConfigDraft);
  }, [telegramConfigData, telegramConfigDraft]);

  return {
    wechatConfigLoading,
    wechatConfigError,
    wechatConfigData,
    feishuConfigLoading,
    feishuConfigError,
    feishuConfigData,
    telegramConfigLoading,
    telegramConfigError,
    telegramConfigData,
    telegramConfigSaving,
    telegramConfigSaveMessage,
    primaryTelegramBot,
    telegramBotTokenConfigured,
    hasUnsavedTelegramConfigChanges,
    fetchWechatConfig,
    fetchFeishuConfig,
    fetchTelegramConfig,
    setTelegramPrimaryBotDraftField,
    saveTelegramConfig,
  };
}
