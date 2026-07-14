import type { TaskLifecycleProjection } from "../lib/task-lifecycle";

export interface ApiResponse<T> {
  ok: boolean;
  data?: T;
  error?: string;
}

export interface HealthResponse {
  version: string;
  queue_length: number;
  worker_state: string;
  uptime_seconds: number;
  memory_rss_bytes?: number | null;
  running_length: number;
  task_timeout_seconds: number;
  running_oldest_age_seconds: number;
  telegramd_healthy?: boolean | null;
  telegramd_process_count?: number | null;
  telegramd_memory_rss_bytes?: number | null;
  channel_gateway_healthy?: boolean | null;
  channel_gateway_process_count?: number | null;
  channel_gateway_memory_rss_bytes?: number | null;
  whatsappd_healthy?: boolean | null;
  whatsappd_process_count?: number | null;
  whatsappd_memory_rss_bytes?: number | null;
  telegram_bot_healthy?: boolean | null;
  telegram_bot_process_count?: number | null;
  telegram_bot_memory_rss_bytes?: number | null;
  telegram_configured_bot_count?: number;
  gateway_instance_statuses?: Array<{ kind: string }>;
  whatsapp_cloud_healthy?: boolean | null;
  whatsapp_cloud_process_count?: number | null;
  whatsapp_cloud_memory_rss_bytes?: number | null;
  whatsapp_web_healthy?: boolean | null;
  whatsapp_web_process_count?: number | null;
  whatsapp_web_memory_rss_bytes?: number | null;
  wechatd_healthy?: boolean | null;
  wechatd_process_count?: number | null;
  wechatd_memory_rss_bytes?: number | null;
  feishud_healthy?: boolean | null;
  feishud_process_count?: number | null;
  feishud_memory_rss_bytes?: number | null;
  larkd_healthy?: boolean | null;
  larkd_process_count?: number | null;
  larkd_memory_rss_bytes?: number | null;
  user_count?: number;
  bound_channel_count?: number;
  bound_channels?: string[];
  future_adapters_enabled?: string[];
}

export interface TaskQueryResponse {
  task_id: string;
  status: "queued" | "running" | "succeeded" | "failed" | "canceled" | "timeout";
  result_json?: unknown | null;
  error_text?: string | null;
  lifecycle?: TaskLifecycleProjection | null;
}

export interface TaskLlmDebugUsage {
  prompt_tokens?: number | null;
  completion_tokens?: number | null;
  total_tokens?: number | null;
  input_tokens?: number | null;
  output_tokens?: number | null;
  reasoning_tokens?: number | null;
  cached_tokens?: number | null;
  cache_creation_input_tokens?: number | null;
  cache_read_input_tokens?: number | null;
}

export interface TaskLlmDebugFlow {
  prompt_label?: string | null;
  flow_stage?: string | null;
  flow_node?: string | null;
  code_module?: string | null;
  code_entrypoint?: string | null;
  trigger_kind?: string | null;
}

export interface TaskLlmDebugFlowStageSummary {
  flow_stage: string;
  call_count: number;
  prompt_labels: string[];
  flow_nodes: string[];
  code_modules: string[];
  code_entrypoints: string[];
  trigger_counts: Record<string, number>;
  status_counts: Record<string, number>;
  provider_error_count: number;
}

export interface TaskLlmDebugFlowSummary {
  call_count: number;
  stage_count: number;
  stages: TaskLlmDebugFlowStageSummary[];
  modules: string[];
  retry_count: number;
  verifier_call_count: number;
  finalizer_call_count: number;
  provider_error_count: number;
  status_counts: Record<string, number>;
  trigger_counts: Record<string, number>;
}

export interface TaskLlmDebugCall {
  call_index?: number | null;
  flow?: TaskLlmDebugFlow | null;
  ts?: number | null;
  task_id?: string | null;
  call_id?: string | null;
  vendor?: string | null;
  provider?: string | null;
  provider_type?: string | null;
  model?: string | null;
  model_kind?: string | null;
  status?: string | null;
  mode?: string | null;
  prompt_source?: string | null;
  prompt_hash?: string | null;
  prompt_file?: string | null;
  prompt?: string | null;
  request_payload?: unknown | null;
  response?: string | null;
  raw_response?: string | null;
  clean_response?: string | null;
  sanitized?: boolean | null;
  error?: string | null;
  usage?: TaskLlmDebugUsage | null;
}

export interface TaskLlmDebugResponse {
  task_id: string;
  call_count?: number | null;
  flow_summary?: TaskLlmDebugFlowSummary | null;
  calls?: TaskLlmDebugCall[] | null;
  entries?: TaskLlmDebugCall[] | null;
  memory_trace?: unknown | null;
}

export interface ActiveTaskItem {
  index: number;
  task_id: string;
  kind: string;
  status: string;
  summary: string;
  age_seconds: number;
  lifecycle?: TaskLifecycleProjection | null;
}

export interface ActiveTasksResponse {
  count: number;
  tasks: ActiveTaskItem[];
}

export interface SubmitTaskResponse {
  task_id: string;
}

export type WorkspaceUpdateMode = "full" | "ui_only" | "clawd_only" | "release_deploy";

export interface WorkspaceUpdateStatus {
  status: "idle" | "running" | "succeeded" | "failed" | "canceled" | "restarting" | "up_to_date" | string;
  step: string;
  mode?: WorkspaceUpdateMode | string;
  started_ts?: number | null;
  finished_ts?: number | null;
  old_commit?: string | null;
  new_commit?: string | null;
  remote_commit?: string | null;
  exit_code?: number | null;
  stdout_tail: string;
  stderr_tail: string;
  error?: string | null;
  next_step?: string | null;
}

export interface PiAppStatusResponse {
  available: boolean;
  is_raspberry_pi: boolean;
  model?: string | null;
  script_exists?: boolean;
}

export interface LocalInteractionContextResponse {
  user_id: number;
  chat_id: number;
  role: string;
}

export interface AuthIdentityResponse extends LocalInteractionContextResponse {
  user_key: string;
}

export interface AuthKeyListItem {
  key_id: number;
  user_key: string;
  user_key_masked: string;
  role: string;
  enabled: boolean;
  created_at: string;
  last_used_at: string | null;
  webd_username?: string | null;
  current_key?: boolean;
}

export interface ResolveChannelBindingResponse {
  bound: boolean;
  identity?: AuthIdentityResponse | null;
}

export interface SkillListItem {
  name: string;
  description?: string | null;
  kind?: string | null;
  planner_kind?: string | null;
  adapter_category?: string | null;
  background_job_capable?: boolean | null;
  group?: string | null;
  risk_level?: string | null;
  auto_invocable?: boolean | null;
  requires_confirmation?: boolean | null;
  side_effect?: boolean | null;
  retryable?: boolean | null;
  output_kind?: string | null;
  enabled?: boolean | null;
  runtime_available?: boolean | null;
  unavailable_reason?: string | null;
  current_os?: string | null;
  unsupported_os?: string[] | null;
  missing_required_bins?: string[] | null;
  missing_optional_bins?: string[] | null;
  supported_os?: string[] | null;
  required_bins?: string[] | null;
  optional_bins?: string[] | null;
  platform_notes?: string[] | null;
  planner_capabilities?: string[] | null;
  planner_capability_policies?: PlannerCapabilityPolicyItem[] | null;
  capabilities?: string[] | null;
}

export interface PlannerCapabilityPolicyItem {
  capability: string;
  isolation_profile?: string | null;
  network_access?: boolean | null;
  filesystem_write?: boolean | null;
  external_publish?: boolean | null;
  credential_access?: boolean | null;
}

export interface SkillsResponse {
  skills: string[];
  skill_items?: SkillListItem[];
  skill_runner_path?: string;
}

export interface SkillsConfigResponse {
  config_path: string;
  skills_list: string[];
  skill_switches: Record<string, boolean>;
  managed_skills: string[];
  /** 基本技能：UI 归类为「基础技能」，用于降低误关核心能力的风险 */
  base_skill_names?: string[];
  /** UI 保存时强制保持开启的技能；用于把开关按钮显示为不可关闭 */
  core_skill_names?: string[];
  /** planner_kind=tool 的底层工具能力；UI 归到工具分组并固定开启 */
  tool_skill_names?: string[];
  /** 后端判定的 UI 锁定名单，保存时也会被强制保持开启 */
  locked_skill_names?: string[];
  external_skill_names?: string[];
  skill_items?: SkillListItem[];
  effective_enabled_skills_preview: string[];
  runtime_enabled_skills: string[];
  restart_required: boolean;
}

export interface MemoryCounts {
  recent: number;
  preferences: number;
  facts_active: number;
  facts_total: number;
  long_term_summaries: number;
}

export interface MemoryOverviewResponse {
  user_key: string;
  user_id: number;
  chat_id: number;
  long_term_enabled: boolean;
  hybrid_recall_enabled: boolean;
  counts: MemoryCounts;
}

export interface MemoryPreferenceItem {
  id: string;
  raw_id: number;
  key: string;
  value: string;
  confidence: number;
  source: string;
  updated_at_ts: number;
}

export interface MemoryFactItem {
  id: string;
  raw_id: number;
  namespace: string;
  fact_key: string;
  fact_value: string;
  fact_text: string;
  confidence: number;
  source_kind: string;
  source_ref: string;
  reason: string;
  updated_at_ts: number;
  expires_at_ts?: number | null;
  conflict_group?: string | null;
  status: string;
}

export interface MemoryRecentItem {
  id: string;
  raw_id: number;
  role: string;
  memory_type: string;
  content: string;
  created_at_ts: number;
  safety_flag: string;
}

export interface MemoryDeleteResult {
  id: string;
  kind: string;
  deleted: boolean;
}

export interface MemoryExpireResult {
  id: string;
  kind: string;
  expired: boolean;
}

export interface MemoryClearResult {
  scope: string;
  recent_deleted: number;
  preferences_deleted: number;
  facts_deleted: number;
}

export interface MemorySettingsResult {
  config_path: string;
  long_term_enabled: boolean;
  restart_required: boolean;
}

export interface FactoryResetResponse {
  status: string;
  admin_user_key: string;
  webd_username: string;
  webd_password: string;
  database?: Record<string, number>;
  config?: {
    files_scanned: number;
    files_updated: number;
    fields_cleared: number;
    errors?: string[];
  };
  logs?: {
    files_deleted: number;
    directories_deleted: number;
    bytes_deleted: number;
    errors?: string[];
  };
  warnings?: string[];
}

export interface ImportedSkillResponse {
  skill_name: string;
  display_name: string;
  description: string;
  external_kind: string;
  bundle_dir: string;
  entry_file: string;
  runtime?: string | null;
  require_bins: string[];
  require_py_modules: string[];
  prompt_file: string;
  source: string;
}

export interface LlmVendorOption {
  name: string;
  default_model: string;
  models: string[];
  base_url: string;
  api_key?: string;
  api_format?: string;
  api_key_configured: boolean;
  api_key_masked?: string | null;
}

export interface LlmRuntimeInfo {
  vendor: string;
  model: string;
  provider_name?: string;
  provider_type?: string;
}

export interface LlmConfigResponse {
  config_path: string;
  selected_vendor: string;
  selected_model: string;
  vendors: LlmVendorOption[];
  runtime?: LlmRuntimeInfo | null;
  restart_required: boolean;
}

export interface LlmTestResponse {
  success: boolean;
  vendor: string;
  model: string;
  provider_type: string;
  message: string;
  response_text?: string;
}

export interface NniDeviceMeta {
  slot?: number | null;
  i2c_bus?: number | null;
  i2c_baud?: number | null;
  i2c_address?: string | null;
  lib_path?: string | null;
}

export interface NniDeviceStatusResponse {
  nni_available: boolean;
  helper_available: boolean;
  signature_chip_present: boolean;
  status: string;
  message: string;
  next_step?: string | null;
  helper_path?: string | null;
  supported_actions?: string[];
  pubkey?: string | null;
  pubkey_preview?: string | null;
  pubkey_fingerprint?: string | null;
  meta?: NniDeviceMeta | null;
  error?: string | null;
}

export interface NniDevicePayload {
  ok?: boolean;
  action?: string;
  pubkey?: string;
  timestamp?: number;
  signature?: string;
  device_cert_hex?: string;
  device_cert_hex_size?: number;
  signer_cert_hex?: string;
  signer_cert_hex_size?: number;
  root_cert_hex?: string;
  root_cert_hex_size?: number;
  slot?: number;
  i2c_bus?: number;
  i2c_baud?: number;
  i2c_address?: string;
  lib_path?: string;
  [key: string]: unknown;
}

export interface NniDeviceActionResponse {
  action: string;
  signature_chip_present: boolean;
  message: string;
  payload?: NniDevicePayload;
  meta?: NniDeviceMeta | null;
}

export interface NniJoinTaskResponse {
  status: string;
  task_id: string;
  challenge: string;
  device_pubkey: string;
  node_url: string;
  expires_at_ts: number;
  request_interval_seconds: number;
}

export interface NniJoinVerifyResponse {
  status: string;
  task_id: string;
  device_pubkey: string;
  node_url: string;
  compliant: boolean;
  joined: boolean;
  verified_at_ts: number;
  next_allowed_ts: number;
}

export interface NniConfigResponse {
  remote_nodes: string[];
  joined: boolean;
  heartbeat_interval_seconds: number;
  heartbeat_network_retry_limit: number;
  heartbeat_request_count: number;
  last_heartbeat_at_ts?: number | null;
  last_heartbeat_error?: string | null;
  last_heartbeat_error_at_ts?: number | null;
  last_heartbeat_network_failures: number;
  config_path: string;
}

export interface NniHeartbeatRecord {
  id: number | null;
  request_kind: string;
  task_id?: string | null;
  user_key?: string | null;
  device_pubkey?: string | null;
  node_url?: string | null;
  compliant?: boolean | null;
  status: string;
  error_code?: string | null;
  created_at_ts?: number | null;
  signature_present?: boolean;
  challenge_present?: boolean;
}

export interface NniHeartbeatRecordsResponse {
  status: string;
  page: number;
  per_page: number;
  total: number;
  total_pages: number;
  records: NniHeartbeatRecord[];
}

export interface NniHeartbeatErrorRecord {
  id: number;
  created_at_ts?: number | null;
  error: string;
  network: boolean;
}

export interface NniHeartbeatErrorsResponse {
  status: string;
  page: number;
  per_page: number;
  total: number;
  total_pages: number;
  records: NniHeartbeatErrorRecord[];
}

export interface WechatConfigResponse {
  config_path: string;
  enabled: boolean;
  listen: string;
  clawd_base_url: string;
  api_base_url: string;
  wechat_uin_base64: string;
  request_timeout_seconds: number;
  longpoll_timeout_ms: number;
  text_chunk_chars: number;
  bot_token_configured: boolean;
  saved_session_present: boolean;
  restart_required: boolean;
}

export interface FeishuConfigResponse {
  config_path: string;
  enabled: boolean;
  mode: string;
  listen: string;
  clawd_base_url: string;
  api_base_url: string;
  app_id: string;
  app_secret: string;
  verification_token_configured: boolean;
  encrypt_key_configured: boolean;
  bind_ready: boolean;
  current_key_bound: boolean;
  restart_required: boolean;
}

export interface AgentConfigItem {
  id: string;
  name?: string;
  description?: string;
  persona_prompt?: string;
  preferred_vendor?: string | null;
  preferred_model?: string | null;
  allowed_skills?: string[];
}

export interface TelegramBotConfigItem {
  name: string;
  bot_token: string;
  bot_token_configured?: boolean;
  bot_token_masked?: string | null;
  agent_id: string;
  allowlist: number[];
  access_mode: string;
  allowed_telegram_usernames: string[];
  is_primary: boolean;
}

export interface TelegramConfigResponse {
  config_path: string;
  bots: TelegramBotConfigItem[];
  agents: AgentConfigItem[];
  restart_required: boolean;
}

export interface ModelConfigItem {
  vendor: string;
  model: string;
  base_url?: string;
  api_key?: string;
  api_key_configured?: boolean;
  api_key_masked?: string | null;
  capabilities?: string[];
  capability_family?: string | null;
  input_modalities?: string[];
  output_modalities?: string[];
  available_models?: string[];
  context_window_tokens?: number | null;
  async_job_supported?: boolean | null;
  shared_quota_group?: string | null;
  shared_quota_note_key?: string | null;
  model_list_source?: string | null;
  capability_source?: string | null;
  risk_level?: string | null;
  dry_run_supported?: boolean | null;
  external_provider?: boolean | null;
  provider_supported?: boolean | null;
  unsupported_reason?: string | null;
}

export interface ModelConfigResponse {
  llm: ModelConfigItem;
  image_edit: ModelConfigItem;
  image_generation: ModelConfigItem;
  image_vision: ModelConfigItem;
  audio_transcribe: ModelConfigItem;
  audio_synthesize: ModelConfigItem;
  video_generation: ModelConfigItem;
  music_generation: ModelConfigItem;
  restart_required: boolean;
}

export interface LogLatestResponse {
  file: string;
  lines: number;
  text: string;
}

export interface WhatsappWebLoginStatus {
  connected?: boolean;
  qr_ready?: boolean;
  qr_data_url?: string | null;
  last_update_ts?: number;
  last_error?: string | null;
}

export interface WechatLoginStatus {
  connected?: boolean;
  qr_ready?: boolean;
  session_key?: string | null;
  qr_status?: string | null;
  qrcode_url?: string | null;
  message?: string | null;
  last_update_ts?: number;
  last_error?: string | null;
  account_label?: string | null;
  status?: string | null;
}

export interface WechatQrStartResponse {
  session_key: string;
  qrcode_url: string;
  message?: string;
}

export interface WechatQrWaitResponse {
  connected?: boolean;
  qr_status?: string | null;
  message?: string;
  account_id?: string | null;
  user_id?: string | null;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  text: string;
  ts: number;
  attachments?: ChatAttachment[];
  images?: ChatAttachment[];
}

export type BrowserFileWithPath = File & {
  webkitRelativePath?: string;
};

export type ChatAttachmentKind = "image" | "audio" | "file";

export interface ChatAttachment {
  name: string;
  dataUrl: string;
  mimeType: string;
  size: number;
  kind: ChatAttachmentKind;
  durationMs?: number;
}

export type ChatImageAttachment = ChatAttachment;

export interface AdapterHealthRow {
  key: string;
  label: string;
  serviceName: "telegramd" | "whatsappd" | "whatsapp_webd" | "wechatd" | "feishud" | "larkd";
  healthy: boolean | null | undefined;
  processCount: number | null | undefined;
  memoryRssBytes: number | null | undefined;
}

export interface ChannelPreset {
  summary: string;
  userHint: string;
  chatHint: string;
  exampleUser: string;
  exampleChat: string;
  note: string;
}

export interface ServiceStatusRow extends AdapterHealthRow {
  category: "ready" | "attention" | "stopped" | "unknown";
  statusLabel: string;
  detail: string;
}

export interface DashboardCommunicationRow extends ServiceStatusRow {
  memoryLabel: string;
  usesSharedGatewayMemory: boolean;
}

export interface ServiceActionNotice {
  tone: "success" | "error";
  text: string;
}

export type ChannelName = "telegram" | "whatsapp" | "ui" | "wechat" | "feishu" | "lark";
export type ConsolePage = "dashboard" | "chat" | "nni" | "services" | "channels" | "models" | "skills" | "memory" | "logs" | "tasks";
