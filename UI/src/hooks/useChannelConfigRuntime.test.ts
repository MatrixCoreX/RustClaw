import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

import type { ApiResponse, WechatConfigResponse } from "../types/api";
import { useChannelConfigRuntime } from "./useChannelConfigRuntime";

const initialWechatConfig: WechatConfigResponse = {
  config_path: "configs/channels/wechat.toml",
  enabled: false,
  listen: "127.0.0.1:8792",
  clawd_base_url: "http://127.0.0.1:8787",
  api_base_url: "https://ilinkai.weixin.qq.com",
  wechat_uin_base64: "",
  request_timeout_seconds: 30,
  longpoll_timeout_ms: 35_000,
  text_chunk_chars: 1_200,
  bot_token_configured: false,
  saved_session_present: false,
  restart_required: true,
};

function response(data: WechatConfigResponse): Response {
  const body: ApiResponse<WechatConfigResponse> = { ok: true, data, error: null };
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

test("WeChat enable preserves the loaded channel configuration", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let submitted: Record<string, unknown> | null = null;
  const apiFetch = async (path: string, init?: RequestInit): Promise<Response> => {
    assert.equal(path, "/v1/wechat/config");
    if (!init?.method) return response(initialWechatConfig);
    submitted = JSON.parse(String(init.body)) as Record<string, unknown>;
    return response({ ...initialWechatConfig, enabled: true });
  };
  let runtime: ReturnType<typeof useChannelConfigRuntime> | null = null;
  function Probe() {
    runtime = useChannelConfigRuntime({ apiFetch, t: (zh) => zh });
    return null;
  }

  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await runtime!.fetchWechatConfig();
  });
  let enabled = false;
  await act(async () => {
    enabled = await runtime!.setWechatEnabled(true);
  });

  assert.equal(enabled, true);
  assert.deepEqual(submitted, {
    enabled: true,
    listen: initialWechatConfig.listen,
    clawd_base_url: initialWechatConfig.clawd_base_url,
    api_base_url: initialWechatConfig.api_base_url,
    wechat_uin_base64: initialWechatConfig.wechat_uin_base64,
    request_timeout_seconds: initialWechatConfig.request_timeout_seconds,
    longpoll_timeout_ms: initialWechatConfig.longpoll_timeout_ms,
    text_chunk_chars: initialWechatConfig.text_chunk_chars,
  });
  assert.equal(runtime!.wechatConfigData?.enabled, true);
  assert.equal(runtime!.wechatConfigSaving, false);

  await act(async () => {
    renderer!.unmount();
  });
});
