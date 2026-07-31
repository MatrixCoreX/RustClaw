import { useCallback, useEffect, useRef, useState } from "react";

import type { AgentConfigResponse, ApiResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export function useAgentConfigRuntime({
  apiFetch,
  t,
  enabled,
}: {
  apiFetch: ApiFetch;
  t: Translate;
  enabled: boolean;
}) {
  const [agentConfig, setAgentConfig] = useState<AgentConfigResponse | null>(null);
  const [agentConfigLoading, setAgentConfigLoading] = useState(false);
  const [agentConfigSaving, setAgentConfigSaving] = useState(false);
  const [agentConfigError, setAgentConfigError] = useState<string | null>(null);
  const [agentConfigMessage, setAgentConfigMessage] = useState<string | null>(null);
  const apiFetchRef = useRef(apiFetch);
  const tRef = useRef(t);
  apiFetchRef.current = apiFetch;
  tRef.current = t;

  const fetchAgentConfig = useCallback(async () => {
    if (!enabled) return null;
    setAgentConfigLoading(true);
    setAgentConfigError(null);
    try {
      const response = await apiFetchRef.current("/v1/agents/config");
      const body = (await response.json()) as ApiResponse<AgentConfigResponse>;
      if (!response.ok || !body.ok || !body.data) {
        throw new Error(body.error || `agent_config_http_${response.status}`);
      }
      setAgentConfig(body.data);
      return body.data;
    } catch (error) {
      setAgentConfigError(
        error instanceof Error
          ? formatAgentConfigError(error.message, tRef.current)
          : tRef.current("读取 Agent 设置失败。", "Failed to load Agent settings."),
      );
      return null;
    } finally {
      setAgentConfigLoading(false);
    }
  }, [enabled]);

  const saveAgentPersona = useCallback(
    async (agentId: string, personaProfile: string, customPersona: string) => {
      setAgentConfigSaving(true);
      setAgentConfigError(null);
      setAgentConfigMessage(null);
      try {
        const response = await apiFetchRef.current("/v1/agents/config", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            agent_id: agentId,
            persona_profile: personaProfile,
            custom_persona: customPersona,
          }),
        });
        const body = (await response.json()) as ApiResponse<AgentConfigResponse>;
        if (!response.ok || !body.ok || !body.data) {
          throw new Error(body.error || `agent_config_http_${response.status}`);
        }
        setAgentConfig(body.data);
        setAgentConfigMessage(
          tRef.current(
            "已保存；只对之后新建的任务生效。",
            "Saved; this only applies to tasks created from now on.",
          ),
        );
        return true;
      } catch (error) {
        setAgentConfigError(
          error instanceof Error
            ? formatAgentConfigError(error.message, tRef.current)
            : tRef.current("保存 Agent 设置失败。", "Failed to save Agent settings."),
        );
        return false;
      } finally {
        setAgentConfigSaving(false);
      }
    },
    [],
  );

  useEffect(() => {
    if (enabled) void fetchAgentConfig();
  }, [enabled, fetchAgentConfig]);

  return {
    agentConfig,
    agentConfigLoading,
    agentConfigSaving,
    agentConfigError,
    agentConfigMessage,
    fetchAgentConfig,
    saveAgentPersona,
  };
}

function formatAgentConfigError(message: string, t: Translate): string {
  if (message.includes("custom_persona_too_long")) {
    return t("自定义语气超过长度限制，请缩短后重试。", "The custom style is too long. Shorten it and try again.");
  }
  if (message.includes("control_character")) {
    return t("自定义语气包含不支持的字符，请删除后重试。", "The custom style contains an unsupported character. Remove it and try again.");
  }
  if (message.includes("admin_required")) {
    return t("只有管理员可以保存这项设置。", "Only an administrator can save this setting.");
  }
  if (message.includes("agent_not_found")) {
    return t("这个 Agent 已不存在，请刷新后重新选择。", "This Agent no longer exists. Refresh and choose again.");
  }
  return t("保存或读取失败，请检查连接后重试。", "The setting could not be loaded or saved. Check the connection and try again.");
}
