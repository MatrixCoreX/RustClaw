import type { LlmConfigResponse } from "../types/api";

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function strings(value: unknown): value is string[] {
  return Array.isArray(value) && value.every(item => typeof item === "string");
}

/** Validate before React state changes; an HTTP success is not a schema proof. */
export function readLlmConfig(value: unknown): LlmConfigResponse {
  if (!record(value) || typeof value.config_path !== "string"
    || typeof value.selected_vendor !== "string" || typeof value.selected_model !== "string"
    || typeof value.restart_required !== "boolean" || !Array.isArray(value.vendors)
    || !value.vendors.every(vendor => record(vendor) && typeof vendor.name === "string"
      && typeof vendor.default_model === "string" && typeof vendor.base_url === "string"
      && typeof vendor.api_key_configured === "boolean" && strings(vendor.models)
      && (vendor.api_format === undefined || typeof vendor.api_format === "string")
      && (vendor.api_key_masked == null || typeof vendor.api_key_masked === "string")
      && (vendor.api_key_env_names === undefined || strings(vendor.api_key_env_names)))) {
    throw new Error("invalid_llm_config_response");
  }
  if (value.runtime != null && (!record(value.runtime)
    || typeof value.runtime.vendor !== "string" || typeof value.runtime.model !== "string")) {
    throw new Error("invalid_llm_config_response");
  }
  if (value.hosted_relay != null && (!record(value.hosted_relay)
    || value.hosted_relay.vendor !== "custom" || value.hosted_relay.api_format !== "openai_compat"
    || typeof value.hosted_relay.model !== "string" || typeof value.hosted_relay.base_url !== "string"
    || typeof value.hosted_relay.daily_request_limit !== "number"
    || !Number.isFinite(value.hosted_relay.daily_request_limit) || value.hosted_relay.daily_request_limit < 0)) {
    throw new Error("invalid_llm_config_response");
  }
  return value as unknown as LlmConfigResponse;
}
