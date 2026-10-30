import { useEffect, useMemo, useState } from "react";

import type {
  ApiResponse,
  FeishuConfigResponse,
  LarkConfigResponse,
  TelegramBotConfigItem,
  TelegramConfigResponse,
  WechatConfigResponse,
} from "../types/api";
import { formatUiError } from "../lib/ui-error";
import { PageRequestLifecycle } from "../lib/page-request-lifecycle";
import { useRequestOwner } from "./useRequestOwner";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type ConfigChannel = "wechat" | "feishu" | "lark" | "telegram";

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
  requestScope: string;
  isAdminIdentity: boolean;
}

export function useChannelConfigRuntime({ apiFetch, t, requestScope, isAdminIdentity }: UseChannelConfigRuntimeParams) {
  const isCurrent = useRequestOwner(requestScope, isAdminIdentity ? "admin" : "user");
  const requests = useMemo(() => ({
    reads: { wechat: new PageRequestLifecycle(), feishu: new PageRequestLifecycle(),
      lark: new PageRequestLifecycle(), telegram: new PageRequestLifecycle() },
    writes: new Set<ConfigChannel>(),
    wechatData: null as WechatConfigResponse | null,
    telegramData: null as TelegramConfigResponse | null,
    telegramDraft: null as TelegramConfigResponse | null,
    draftRevision: 0,
  }), [isCurrent]);
  const [wechatConfigLoading, setWechatConfigLoading] = useState(false);
  const [wechatConfigSaving, setWechatConfigSaving] = useState(false);
  const [wechatConfigError, setWechatConfigError] = useState<string | null>(null);
  const [wechatConfigData, setWechatConfigData] = useState<WechatConfigResponse | null>(null);
  const [feishuConfigLoading, setFeishuConfigLoading] = useState(false);
  const [feishuConfigError, setFeishuConfigError] = useState<string | null>(null);
  const [feishuConfigData, setFeishuConfigData] = useState<FeishuConfigResponse | null>(null);
  const [larkConfigLoading, setLarkConfigLoading] = useState(false);
  const [larkConfigError, setLarkConfigError] = useState<string | null>(null);
  const [larkConfigData, setLarkConfigData] = useState<LarkConfigResponse | null>(null);
  const [telegramConfigLoading, setTelegramConfigLoading] = useState(false);
  const [telegramConfigError, setTelegramConfigError] = useState<string | null>(null);
  const [telegramConfigData, setTelegramConfigData] = useState<TelegramConfigResponse | null>(null);
  const [telegramConfigDraft, setTelegramConfigDraft] = useState<TelegramConfigResponse | null>(null);
  const [telegramConfigSaving, setTelegramConfigSaving] = useState(false);
  const [telegramConfigSaveMessage, setTelegramConfigSaveMessage] = useState<string | null>(null);

  useEffect(() => {
    setWechatConfigLoading(false);
    setWechatConfigSaving(false);
    setWechatConfigError(null);
    setWechatConfigData(null);
    setFeishuConfigLoading(false);
    setFeishuConfigError(null);
    setFeishuConfigData(null);
    setLarkConfigLoading(false);
    setLarkConfigError(null);
    setLarkConfigData(null);
    setTelegramConfigLoading(false);
    setTelegramConfigError(null);
    setTelegramConfigData(null);
    setTelegramConfigDraft(null);
    setTelegramConfigSaving(false);
    setTelegramConfigSaveMessage(null);
  }, [isCurrent]);

  const fetchConfig = async <T,>(
    channel: ConfigChannel,
    apply: (data: T) => void,
    setLoading: (loading: boolean) => void,
    setError: (error: string | null) => void,
    fallbackZh: string,
    fallbackEn: string,
  ) => {
    if (!isCurrent() || requests.writes.has(channel)) return;
    const lifecycle = requests.reads[channel];
    const request = lifecycle.begin(false)!;
    const ownsRead = () => isCurrent() && lifecycle.isCurrent(request) && !requests.writes.has(channel);
    setLoading(true);
    setError(null);
    try {
      const res = await apiFetch(`/v1/${channel}/config`);
      const body = (await res.json()) as ApiResponse<T>;
      if (!ownsRead()) return;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `${channel}_config_fetch_http_${res.status}`);
      }
      apply(body.data);
    } catch (err) {
      if (ownsRead()) setError(formatUiError(err, t, fallbackZh, fallbackEn));
    } finally {
      if (ownsRead() && lifecycle.finish(request)) setLoading(false);
    }
  };

  const fetchWechatConfig = () => fetchConfig<WechatConfigResponse>("wechat", data => {
    requests.wechatData = data;
    setWechatConfigData(data);
  }, setWechatConfigLoading, setWechatConfigError, "微信配置暂时无法读取。", "WeChat configuration is temporarily unavailable.");

  const setWechatEnabled = async (enabled: boolean): Promise<boolean> => {
    if (!isAdminIdentity || !isCurrent() || requests.writes.has("wechat")) return false;
    const config = requests.wechatData;
    if (!config) {
      setWechatConfigError(t("微信配置尚未载入，请刷新后重试。", "WeChat configuration is not loaded yet. Refresh and try again."));
      return false;
    }
    requests.writes.add("wechat");
    requests.reads.wechat.invalidate();
    setWechatConfigLoading(false);
    setWechatConfigSaving(true);
    setWechatConfigError(null);
    try {
      const res = await apiFetch(`/v1/wechat/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          enabled,
          listen: config.listen,
          clawd_base_url: config.clawd_base_url,
          api_base_url: config.api_base_url,
          wechat_uin_base64: config.wechat_uin_base64,
          request_timeout_seconds: config.request_timeout_seconds,
          longpoll_timeout_ms: config.longpoll_timeout_ms,
          text_chunk_chars: config.text_chunk_chars,
        }),
      });
      const body = (await res.json()) as ApiResponse<WechatConfigResponse>;
      if (!isCurrent()) return false;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `wechat_config_save_http_${res.status}`);
      }
      requests.wechatData = body.data;
      setWechatConfigData(body.data);
      return true;
    } catch (err) {
      if (!isCurrent()) return false;
      const message = formatUiError(err, t, "微信配置保存失败，请稍后重试。", "WeChat configuration could not be saved. Try again shortly.");
      setWechatConfigError(message);
      return false;
    } finally {
      if (isCurrent()) {
        requests.writes.delete("wechat");
        setWechatConfigSaving(false);
      }
    }
  };

  const fetchFeishuConfig = () => fetchConfig<FeishuConfigResponse>("feishu", setFeishuConfigData,
    setFeishuConfigLoading, setFeishuConfigError, "飞书配置暂时无法读取。", "Feishu configuration is temporarily unavailable.");

  const fetchLarkConfig = () => fetchConfig<LarkConfigResponse>("lark", setLarkConfigData,
    setLarkConfigLoading, setLarkConfigError, "Lark 配置暂时无法读取。", "Lark configuration is temporarily unavailable.");

  const fetchTelegramConfig = () => {
    const revision = requests.draftRevision;
    const dirty = JSON.stringify(requests.telegramData) !== JSON.stringify(requests.telegramDraft);
    return fetchConfig<TelegramConfigResponse>("telegram", data => {
      requests.telegramData = data;
      setTelegramConfigData(data);
      if (!requests.telegramDraft || (!dirty && revision === requests.draftRevision)) {
        requests.telegramDraft = data;
        setTelegramConfigDraft(data);
      }
    }, setTelegramConfigLoading, setTelegramConfigError, "Telegram 配置暂时无法读取。", "Telegram configuration is temporarily unavailable.");
  };

  const setTelegramPrimaryBotDraftField = (key: keyof TelegramBotConfigItem, value: TelegramBotConfigItem[keyof TelegramBotConfigItem]) => {
    if (!isAdminIdentity || !isCurrent() || !requests.telegramDraft) return;
    const prev = requests.telegramDraft;
    const bots = prev.bots.length > 0 ? [...prev.bots] : [buildDefaultTelegramBot()];
    const primaryIndex = bots.findIndex((bot) => bot.is_primary);
    const targetIndex = primaryIndex >= 0 ? primaryIndex : 0;
    bots[targetIndex] = { ...(bots[targetIndex] ?? buildDefaultTelegramBot()), [key]: value, is_primary: true };
    // Keep synchronous edits visible to a save dispatched in the same event.
    requests.telegramDraft = { ...prev, bots };
    requests.draftRevision++;
    setTelegramConfigDraft(requests.telegramDraft);
    setTelegramConfigSaveMessage(null);
  };

  const saveTelegramConfig = async () => {
    if (!isAdminIdentity || !isCurrent() || requests.writes.has("telegram") || !requests.telegramDraft) return;
    const draft = requests.telegramDraft;
    const revision = requests.draftRevision;
    requests.writes.add("telegram");
    requests.reads.telegram.invalidate();
    setTelegramConfigLoading(false);
    setTelegramConfigSaving(true);
    setTelegramConfigSaveMessage(null);
    setTelegramConfigError(null);
    try {
      const bots = draft.bots.length > 0 ? draft.bots : [buildDefaultTelegramBot()];
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
        }),
      });
      const body = (await res.json()) as ApiResponse<TelegramConfigResponse>;
      if (!isCurrent()) return;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `telegram_config_save_http_${res.status}`);
      }
      requests.telegramData = body.data;
      setTelegramConfigData(body.data);
      if (revision === requests.draftRevision) {
        requests.telegramDraft = body.data;
        setTelegramConfigDraft(body.data);
        setTelegramConfigSaveMessage(
          t(
            "Telegram 配置已保存。下一步请启动 telegramd，然后发一条测试消息。",
            "Telegram config was saved. Next, start telegramd and send a test message.",
          ),
        );
      }
    } catch (err) {
      if (!isCurrent()) return;
      const message = formatUiError(err, t, "Telegram 配置保存失败，请稍后重试。", "Telegram configuration could not be saved. Try again shortly.");
      setTelegramConfigError(message);
    } finally {
      if (isCurrent()) {
        requests.writes.delete("telegram");
        setTelegramConfigSaving(false);
      }
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
    wechatConfigSaving,
    wechatConfigError,
    wechatConfigData,
    feishuConfigLoading,
    feishuConfigError,
    feishuConfigData,
    larkConfigLoading,
    larkConfigError,
    larkConfigData,
    telegramConfigLoading,
    telegramConfigError,
    telegramConfigData,
    telegramConfigSaving,
    telegramConfigSaveMessage,
    primaryTelegramBot,
    telegramBotTokenConfigured,
    hasUnsavedTelegramConfigChanges,
    fetchWechatConfig,
    setWechatEnabled,
    fetchFeishuConfig,
    fetchLarkConfig,
    fetchTelegramConfig,
    setTelegramPrimaryBotDraftField,
    saveTelegramConfig,
  };
}
