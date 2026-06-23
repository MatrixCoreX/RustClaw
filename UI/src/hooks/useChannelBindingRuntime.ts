import { useState } from "react";

import type {
  ApiResponse,
  AuthIdentityResponse,
  ChannelName,
  ResolveChannelBindingResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface UseChannelBindingRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  activeUserKey: string;
  channelLabel: (channel: ChannelName) => string;
  onIdentityApplied: (identity: AuthIdentityResponse) => void;
  onHealthRefresh: () => Promise<void>;
}

export function useChannelBindingRuntime({
  apiFetch,
  t,
  activeUserKey,
  channelLabel,
  onIdentityApplied,
  onHealthRefresh,
}: UseChannelBindingRuntimeParams) {
  const [channelBindingChannel, setChannelBindingChannel] = useState<ChannelName>("telegram");
  const [channelBindingExternalUserId, setChannelBindingExternalUserId] = useState("");
  const [channelBindingExternalChatId, setChannelBindingExternalChatId] = useState("");
  const [channelResolveLoading, setChannelResolveLoading] = useState(false);
  const [channelResolveError, setChannelResolveError] = useState<string | null>(null);
  const [channelResolveResult, setChannelResolveResult] = useState<ResolveChannelBindingResponse | null>(null);
  const [channelBindLoading, setChannelBindLoading] = useState(false);
  const [channelBindError, setChannelBindError] = useState<string | null>(null);
  const [channelBindMessage, setChannelBindMessage] = useState<string | null>(null);

  const buildChannelBindingBody = () => {
    const body: Record<string, unknown> = {
      channel: channelBindingChannel,
    };
    const externalUserId = channelBindingExternalUserId.trim();
    const externalChatId = channelBindingExternalChatId.trim();
    if (externalUserId) {
      body.external_user_id = externalUserId;
    }
    if (externalChatId) {
      body.external_chat_id = externalChatId;
    }
    return body;
  };

  const resolveChannelBinding = async () => {
    setChannelResolveLoading(true);
    setChannelResolveError(null);
    setChannelBindMessage(null);
    try {
      const res = await apiFetch(`/v1/auth/channel/resolve`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(buildChannelBindingBody()),
      });
      const resp = (await res.json()) as ApiResponse<ResolveChannelBindingResponse>;
      if (!res.ok || !resp.ok || !resp.data) {
        throw new Error(resp.error || `channel binding resolve failed (${res.status})`);
      }
      setChannelResolveResult(resp.data);
      return resp.data;
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setChannelResolveError(message);
      return null;
    } finally {
      setChannelResolveLoading(false);
    }
  };

  const bindChannelToCurrentKey = async () => {
    setChannelBindLoading(true);
    setChannelBindError(null);
    setChannelBindMessage(null);
    try {
      const res = await apiFetch(`/v1/auth/channel/bind`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          ...buildChannelBindingBody(),
          ...(activeUserKey ? { user_key: activeUserKey } : {}),
        }),
      });
      const resp = (await res.json()) as ApiResponse<AuthIdentityResponse>;
      if (!res.ok || !resp.ok || !resp.data) {
        throw new Error(resp.error || `channel binding failed (${res.status})`);
      }
      setChannelResolveResult({ bound: true, identity: resp.data });
      setChannelBindMessage(
        t(
          `绑定成功：${channelLabel(channelBindingChannel)} 已绑定到当前 key`,
          `${channelLabel(channelBindingChannel)} has been bound to the current key`,
        ),
      );
      onIdentityApplied(resp.data);
      await onHealthRefresh();
      return resp.data;
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setChannelBindError(message);
      return null;
    } finally {
      setChannelBindLoading(false);
    }
  };

  return {
    channelBindingChannel,
    setChannelBindingChannel,
    channelBindingExternalUserId,
    setChannelBindingExternalUserId,
    channelBindingExternalChatId,
    setChannelBindingExternalChatId,
    channelResolveLoading,
    channelResolveError,
    channelResolveResult,
    channelBindLoading,
    channelBindError,
    channelBindMessage,
    resolveChannelBinding,
    bindChannelToCurrentKey,
  };
}
