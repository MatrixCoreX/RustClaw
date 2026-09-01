import type { TaskLifecycleProjection } from "../lib/task-lifecycle";

export interface ApiResponse<T> {
  ok: boolean;
  data?: T;
  error?: string;
}

export interface NniAssetTransferResponse {
  schema_version: 1;
  status: "asset_transfer_completed";
  request_id: string;
  idempotent_replay: boolean;
  node_url?: string;
  transfer: {
    transfer_id: string;
    from_asset_owner_pubkey: string;
    to_asset_owner_pubkey: string;
    asset: "AIC" | "USD";
    amount_units: string;
    amount: string;
    memo: string;
    from_balance_after_units: string;
    from_balance_after: string;
    to_balance_after_units: string;
    to_balance_after: string;
    authorization_mode: "delegated_hardware" | "asset_owner";
    created_at_unix: number;
  };
}

export interface NniAssetTransferHistoryAccountRef {
  account_kind: "asset_owner" | "pool" | "fee" | "system";
  address: string | null;
}

export interface NniAssetTransferHistoryFlow {
  flow_index: number;
  asset: "AIC" | "USD";
  amount_units: string;
  amount: string;
  from: NniAssetTransferHistoryAccountRef;
  to: NniAssetTransferHistoryAccountRef;
}

export interface NniAssetTransferHistoryRecord {
  transaction_id: string;
  transaction_kind: string;
  transaction_class: "peer_transfer" | "market_trade" | "system_issuance" | "other";
  created_at_unix: number;
  memo: string | null;
  flows: NniAssetTransferHistoryFlow[];
}

export interface NniAssetTransferHistoryResponse {
  schema_version: 1;
  status: "asset_transfer_history";
  owner_pubkey: string;
  page: number;
  per_page: number;
  total_transactions: number;
  total_pages: number;
  source_filter: "all" | "transfer" | "trade" | "issuance";
  direction_filter: "all" | "incoming" | "outgoing";
  transactions: NniAssetTransferHistoryRecord[];
  node_url?: string;
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
  execution_state?: string | null;
  goal?: unknown | null;
  task_plan?: unknown | null;
  skill_progress?: unknown | null;
  result_json?: unknown | null;
  error_text?: string | null;
  lifecycle?: TaskLifecycleProjection | null;
}

export interface ConversationHistoryTurn {
  schema_version: number;
  conversation_id: string;
  agent_id?: string | null;
  external_chat_id?: string | null;
  conversation_title?: string | null;
  task_id: string;
  status: TaskQueryResponse["status"];
  user_text?: string | null;
  assistant_text?: string | null;
  error_text?: string | null;
  user_text_result?: ConversationBodyDescriptor | null;
  assistant_text_result?: ConversationBodyDescriptor | null;
  error_text_result?: ConversationBodyDescriptor | null;
  attachment_count: number;
  attachment_kinds: string[];
  artifacts?: TaskArtifact[];
  artifact_delivery?: unknown;
  created_at: number;
  updated_at: number;
}

export interface AgentPersonaPreset {
  id: string;
  name_key: string;
  description_key: string;
}

export interface AgentConfigView {
  id: string;
  name: string;
  description: string;
  saved_profile: string;
  effective_profile: string;
  custom_persona: string;
  preferred_vendor?: string | null;
  preferred_model?: string | null;
  allowed_skills: string[];
  runtime_applied: boolean;
}

export interface AgentConfigResponse {
  schema_version: number;
  config_path: string;
  editable: boolean;
  applies_to: "new_tasks";
  notice_key: string;
  agents: AgentConfigView[];
  preset_catalog: AgentPersonaPreset[];
  constraints: {
    custom_persona_max_chars: number;
    allowed_control_characters: string[];
  };
}

export interface ConversationBodyContinuation {
  kind: "conversation_body_range";
  url: string;
  next_start_byte: number;
}

export interface ConversationBodyDescriptor {
  schema_version: 1;
  complete: boolean;
  original_size_bytes: number;
  returned_size_bytes: number;
  content_sha256: string;
  continuation?: ConversationBodyContinuation | null;
}

export interface ConversationBodyPage {
  schema_version: 1;
  status: "ok";
  task_id: string;
  field: "user" | "assistant" | "error";
  text: string;
  start_byte: number;
  end_byte: number;
  total_size_bytes: number;
  complete: boolean;
  next_start_byte?: number | null;
  content_sha256: string;
}

export interface ConversationTitleUpdate {
  schema_version: number;
  status: "ok";
  conversation_id: string;
  title: string;
  updated_at: number;
}

export interface ConversationArchiveUpdate {
  schema_version: number;
  status: "ok";
  conversation_id: string;
  archived_at: number;
}

export interface ConversationHistoryPage {
  schema_version: number;
  status: "ok";
  turns: ConversationHistoryTurn[];
  next_cursor?: string | null;
  truncated: boolean;
  content_sha256: string;
}

export type TaskApprovalDecision = "approve_once" | "always_for_scope" | "deny";

export interface ApprovalScopeGrantView {
  grant_id: string;
  scope_kind: string;
  scope_fingerprint: string;
  scope: {
    entries?: Array<{
      capability?: string;
      action?: string;
      effect?: string;
      resource_kind?: string;
      resources?: string[];
    }>;
  } | null;
  channel: string;
  chat_id: number;
  issued_at: number;
  expires_at: number;
  revoked_at?: number | null;
  use_count: number;
  last_used_at?: number | null;
  source_task_id: string;
}

export interface ApprovalScopeGrantListResponse {
  schema_version: number;
  count: number;
  grants: ApprovalScopeGrantView[];
}

export interface TaskEventEnvelope {
  schema_version: number;
  seq?: number;
  timestamp_ms?: number;
  task_id: string;
  thread_id?: string | null;
  session_id?: string | null;
  parent_task_id?: string | null;
  child_task_id?: string | null;
  event_kind: string;
  event_type?: string;
  payload?: Record<string, unknown> | null;
  redaction?: {
    applied?: boolean;
    field_count?: number;
  } | null;
  artifact_refs?: unknown[];
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

export interface TaskLlmDebugEntry {
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

export interface TaskLlmDebugCall extends TaskLlmDebugEntry {
  call_index?: number | null;
  flow?: TaskLlmDebugFlow | null;
  entry?: TaskLlmDebugEntry | null;
}

export interface TaskLlmDebugResponse {
  task_id: string;
  trace_schema_version?: number | null;
  trace_availability?: {
    status?: "available" | "metadata_only" | "pending" | "unavailable" | string | null;
    reason_code?: string | null;
    source?: string | null;
    retention_days?: number | null;
  } | null;
  access?: {
    opt_in?: boolean | null;
    scope?: string | null;
  } | null;
  redaction?: {
    applied?: boolean | null;
    field_count?: number | null;
    policy?: string | null;
  } | null;
  trace_layers?: {
    provider_data?: {
      classification?: string | null;
      fields?: string[] | null;
    } | null;
    agent_decisions?: {
      classification?: string | null;
      fields?: string[] | null;
    } | null;
    [compatibilityField: string]: {
      classification?: string | null;
      fields?: string[] | null;
    } | null | undefined;
  } | null;
  call_count?: number | null;
  flow_summary?: TaskLlmDebugFlowSummary | null;
  calls?: TaskLlmDebugCall[] | null;
  entries?: TaskLlmDebugCall[] | null;
  memory_trace?: unknown | null;
  model_catalog_trace?: unknown | null;
  resume_trace?: unknown | null;
}

export interface ActiveTaskItem {
  index: number;
  task_id: string;
  kind: string;
  status: string;
  channel: ChannelName;
  source_user_id: string;
  external_user_id?: string | null;
  summary: string;
  age_seconds: number;
  lifecycle?: TaskLifecycleProjection | null;
}

export interface ActiveTasksResponse {
  count: number;
  tasks: ActiveTaskItem[];
}

export interface TaskHistoryItem {
  task_id: string;
  kind: string;
  status: "succeeded" | "failed" | "canceled" | "timeout";
  channel: ChannelName;
  source_user_id: string;
  external_user_id?: string | null;
  summary: string;
  created_at_ts: number;
  updated_at_ts: number;
  duration_seconds: number;
}

export interface TaskHistoryResponse {
  count: number;
  total: number;
  limit: number;
  offset: number;
  has_more: boolean;
  tasks: TaskHistoryItem[];
}

export interface SubmitTaskResponse {
  task_id: string;
}

export type WorkspaceUpdateMode =
  | "full"
  | "full_preserve_nginx"
  | "ui_only"
  | "clawd_only"
  | "nginx_enable"
  | "nginx_disable"
  | "local_https_prepare"
  | "local_https_enable"
  | "local_https_restore"
  | "release_deploy"
  | "release_restore"
  | "source_checkout";

export interface WorkspaceUpdateStatus {
  status: "idle" | "running" | "succeeded" | "failed" | "canceled" | "restarting" | "up_to_date" | string;
  step: string;
  mode?: WorkspaceUpdateMode | string;
  installation_kind?: "source_checkout" | "release_package" | "standalone" | "unknown" | string;
  source_update_available?: boolean;
  started_ts?: number | null;
  finished_ts?: number | null;
  old_commit?: string | null;
  new_commit?: string | null;
  remote_commit?: string | null;
  current_version?: string | null;
  current_release_version?: string | null;
  latest_release_tag?: string | null;
  latest_release_check_status?: "unchecked" | "available" | "git_tag" | "stale" | "unavailable" | string;
  latest_release_check_error?: string | null;
  exit_code?: number | null;
  stdout_tail: string;
  stderr_tail: string;
  error?: string | null;
  next_step?: string | null;
  next_step_key?: string | null;
  next_step_args?: Record<string, unknown> | null;
}

export interface NginxUiStatus {
  supported: boolean;
  platform: string;
  installed: boolean;
  running: boolean;
  configured: boolean;
  ui_deployed: boolean;
  clawd_exposure: "loopback_only" | string;
  local_https_supported: boolean;
  local_https_prepared: boolean;
  local_https_enabled: boolean;
  local_https_ca_fingerprint_sha256?: string | null;
}

export interface LocalMdnsStatus {
  supported: boolean;
  platform: string;
  hostname: string;
  mdns_name: string;
  responder_installed: boolean;
  responder_running: boolean;
}

export interface LocalMdnsUpdateResult {
  status: LocalMdnsStatus;
  previous_mdns_name: string;
  https_certificate_refreshed: boolean;
  https_refresh_error_code?: string | null;
}

export interface WebdExposureStatus {
  supported: boolean;
  platform: string;
  enabled: boolean;
  running: boolean;
  listen: string;
  port: number;
  externally_accessible: boolean;
  nginx_compatible: boolean;
  restart_scheduled: boolean;
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

export interface WebdSessionListItem {
  session_handle: string;
  username: string;
  role: string;
  client_ip?: string;
  client_platform?: string;
  user_agent?: string;
  created_unix: number;
  last_activity_unix: number;
  expires_unix: number;
  current: boolean;
}

export interface ResolveChannelBindingResponse {
  bound: boolean;
  identity?: AuthIdentityResponse | null;
}

export interface SkillListItem {
  name: string;
  description?: string | null;
  description_zh?: string | null;
  semantic_tags?: string[] | null;
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
  fixed_on?: boolean | null;
  initial_core?: boolean | null;
  deferred?: boolean | null;
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
  config_files?: string[] | null;
  planner_capabilities?: string[] | null;
  planner_capability_details?: PlannerCapabilityDisplayItem[] | null;
  planner_capability_policies?: PlannerCapabilityPolicyItem[] | null;
  capabilities?: string[] | null;
}

export interface PlannerCapabilityDisplayItem {
  capability: string;
  action?: string | null;
  description?: string | null;
  effect?: "observe" | "mutate" | "validate" | "external" | string | null;
  required?: string[];
  optional?: string[];
}

export interface PlannerCapabilityPolicyItem {
  capability: string;
  isolation_profile?: string | null;
  network_access?: boolean | null;
  filesystem_write?: boolean | null;
  external_publish?: boolean | null;
  credential_access?: boolean | null;
  subprocess?: boolean | null;
  package_install?: boolean | null;
  privilege_escalation?: boolean | null;
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
  /** Registry-owned fixed-on skills. This supersedes the compatibility core list. */
  fixed_on_skill_names?: string[];
  /** Skills whose capability groups are present on the initial planner surface. */
  initial_core_skill_names?: string[];
  /** Planner-visible skills loaded only when the model requests their exact group. */
  deferred_skill_names?: string[];
  /** planner_kind=tool 的底层工具能力；UI 归到工具分组并固定开启 */
  tool_skill_names?: string[];
  /** 后端判定的 UI 锁定名单，保存时也会被强制保持开启 */
  locked_skill_names?: string[];
  external_skill_names?: string[];
  uninstalled_skill_names?: string[];
  skill_items?: SkillListItem[];
  effective_enabled_skills_preview: string[];
  runtime_enabled_skills: string[];
  restart_required: boolean;
}

export interface SkillStoreItem {
  name: string;
  description?: string | null;
  description_zh?: string | null;
  group?: string | null;
  catalog_section?: string | null;
  kind: string;
  source_kind: "bundled_core" | "bundled_optional" | "third_party";
  source?: string | null;
  installed: boolean;
  configured_installed?: boolean;
  package_available?: boolean;
  installation_issue?: "package_missing" | null;
  enabled: boolean;
  install_mode?: string | null;
  build_adapter?: "cargo" | "python" | "node" | "go" | "prebuilt" | "generic_process" | "http_json" | null;
  build_network_policy?: "deny" | "approval_required" | null;
  host_dependencies?: string[] | null;
  runtime_assets?: string[] | null;
  supported_os?: string[] | null;
  supported_arch?: string[] | null;
  package_version?: string | null;
  installed_version?: string | null;
  protocol?: string | null;
  config_files?: string[];
  existing_config_files?: string[];
  storage_kind?: string | null;
  private_data_state?: "present" | "empty" | null;
  skill: SkillListItem;
}

export interface SkillStoreResponse {
  items: SkillStoreItem[];
  uninstalled_skill_names: string[];
  active_operation?: SkillStoreOperation | null;
  recent_operations?: SkillStoreOperation[];
}

export interface SkillStoreDependencyStatus {
  id: string;
  kind: "host" | "runtime_asset";
  installed: boolean;
  status_code: "installed" | "missing" | "unknown" | string;
  version?: string | null;
}

export interface SkillStoreDependencyResponse {
  schema_version: 1;
  skill_name: string;
  checked_at_unix: number;
  all_installed: boolean;
  dependencies: SkillStoreDependencyStatus[];
}

export type SkillStoreOperationAction = "install" | "update" | "repair" | "rollback" | "remove";
export type SkillStoreOperationStatus = "queued" | "running" | "success" | "failure" | "cancelled";
export type SkillStoreOperationStage =
  | "queued"
  | "preflight"
  | "dependencies"
  | "build"
  | "smoke"
  | "activate"
  | "configure"
  | "remove"
  | "rollback"
  | "success"
  | "failure"
  | "cancelled";

export interface SkillStoreOperation {
  schema_version: number;
  operation_id: string;
  skill_name: string;
  action: SkillStoreOperationAction;
  status: SkillStoreOperationStatus;
  stage: SkillStoreOperationStage;
  created_at_unix: number;
  updated_at_unix: number;
  heartbeat_at_unix: number;
  cancel_requested: boolean;
  stages: Array<{ stage: SkillStoreOperationStage; recorded_at_unix: number }>;
  failure?: {
    error_code: string;
    message_key: string;
    phase?: string | null;
    retryable: boolean;
    diagnostic?: string | null;
  } | null;
  result?: SkillStoreMutationResponse | Record<string, unknown> | null;
}

export interface SkillStoreOperationResponse {
  operation: SkillStoreOperation;
}

export interface SkillStoreMutationResponse {
  skill_name: string;
  installed: boolean;
  enabled: boolean;
  package_installed?: boolean;
  adapter?: string | null;
  install_origin?: "source_build" | "built_artifact" | "platform_precompiled" | null;
  installed_version?: string | null;
  receipt_digest?: string | null;
  install_reused?: boolean;
  install_phases?: string[] | null;
  install_root?: string | null;
  package_removed?: boolean;
  config_preserved?: boolean;
  data_preserved?: boolean;
  reused_config_files?: string[];
  deleted_config_files?: string[];
  deleted_private_data?: {
    data_present_before: boolean;
    rows_deleted: number;
    files_deleted: number;
  } | null;
}

export interface MemoryCounts {
  recent: number;
  preferences: number;
  facts_active: number;
  facts_total: number;
  long_term_summaries: number;
}

export interface MemoryOverviewResponse {
  long_term_enabled: boolean;
  hybrid_recall_enabled: boolean;
  counts: MemoryCounts;
}

export interface MemoryPreferenceItem {
  id: string;
  key: string;
  value: string;
  confidence: number;
  source: string;
  updated_at_ts: number;
}

export interface MemoryFactItem {
  id: string;
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

export interface MemoryClearPreview {
  schema_version: number;
  mode: "transcript" | "transcript_and_derived";
  transcript_rows: number;
  derived_rows: number;
  pending_jobs: number;
}

export interface MemorySettingsResult {
  schema_version: number;
  scope: "admin" | "principal" | "conversation";
  target_principal_id: string;
  conversation_id?: string | null;
  requested: {
    use_mode: "inherit" | "enabled" | "disabled";
    generate_mode: "inherit" | "enabled" | "disabled";
    external_context_policy: "inherit" | "exclude" | "evidence_only" | "allow";
  };
  use_memory: boolean;
  generate_memory: boolean;
  external_context_policy: "inherit" | "exclude" | "evidence_only" | "allow";
  use_source: string;
  generate_source: string;
  external_context_source: string;
  managed_deny_reason?: string | null;
  revision: number;
  policy_digest: string;
  restart_required: boolean;
}

export interface MemoryListItem {
  id: string;
  revision: number;
  kind: "fact" | "preference" | "recent" | string;
  scope_kind: "conversation" | "principal" | "project" | string;
  origin: string;
  status: string;
  content: string;
  source: string;
  evidence_available: boolean;
  trust_tier: string;
  updated_at_ts: number;
  expires_at_ts?: number | null;
  supersedes_memory_id?: string | null;
  last_recalled_at_ts?: number | null;
  freshness: "fresh" | "stale" | string;
}

export interface MemoryPageResult {
  schema_version: number;
  items: MemoryListItem[];
  page: number;
  page_size: number;
  total: number;
  has_more: boolean;
}

export interface MemoryMutationResult {
  status: string;
  memory_id: string;
  replacement_memory_id?: string | null;
  revision: number;
  revision_id?: string | null;
  undo_until_ts?: number | null;
}

export interface MemoryExportResult {
  schema_version: number;
  exported_at_ts: number;
  scope_kind: string;
  items: MemoryListItem[];
  checksum: string;
}

export interface MemoryMarkdownExportResult {
  schema_version: number;
  exported_at_ts: number;
  content_type: string;
  content: string;
  checksum: string;
}

export interface MemoryImportPreviewResult {
  schema_version: number;
  import_id: string;
  payload_digest: string;
  accepted_items: number;
  skipped_items: number;
  duplicate_items: number;
  trust_tier: "imported_legacy";
  scope_kind: "principal";
}

export interface MemoryImportResult {
  schema_version: number;
  import_id: string;
  status: "confirmed";
  imported_items: number;
  existing_items: number;
}

export interface RemoteMemoryDisclosure {
  schema_version: number;
  consent_state: "inherit" | "exclude" | "evidence_only" | "allow";
  extraction_provider: string;
  extraction_model: string;
  consolidation_provider: string;
  consolidation_model: string;
  extraction_sends: string[];
  embedding_sends: string[];
  withdrawal_effect: string;
}

export interface MemoryVectorStatus {
  schema_version: number;
  provider_location: "local" | "remote";
  state: "ready" | "building" | "paused" | string;
  active_generation: number;
  queued_jobs: number;
  running_jobs: number;
  failed_jobs: number;
  indexed_rows: number;
  remote_consent: "inherit" | "exclude" | "evidence_only" | "allow";
}

export interface MemoryVectorMutationResult {
  schema_version: number;
  status: string;
  queued_rows: number;
  generation: number;
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
  build_adapter: string;
  launcher: string;
  package_version: string;
  receipt_digest: string;
  install_reused: boolean;
  bundle_dir: string;
  entry_file: string;
  supported_os: string[];
  supported_arch: string[];
  prompt_file: string;
  source: string;
  installed: boolean;
  enabled: boolean;
}

export interface LlmVendorOption {
  name: string;
  default_model: string;
  models: string[];
  base_url: string;
  api_format?: string;
  api_key_configured: boolean;
  api_key_masked?: string | null;
  api_key_source?:
    | "environment"
    | "systemd_credential"
    | "macos_keychain"
    | "private_file_fallback"
    | "device_enrollment"
    | "none";
  api_key_env_names?: string[];
}

export interface HostedRelayPreset {
  vendor: "custom";
  model: string;
  base_url: string;
  api_format: "openai_compat";
  daily_request_limit: number;
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
  hosted_relay?: HostedRelayPreset | null;
  runtime?: LlmRuntimeInfo | null;
  restart_required: boolean;
}

export interface LlmTestResponse {
  success: boolean;
  vendor: string;
  model: string;
  provider_type: string;
  message?: string | null;
  message_key?: string | null;
  message_args?: Record<string, unknown> | null;
  response_text?: string;
}

export interface McpServerConfigItem {
  server_id: string;
  enabled: boolean;
  trusted: boolean;
  transport: "stdio" | "streamable_http" | string;
  command?: string | null;
  args: string[];
  env_refs: Record<string, string>;
  url?: string | null;
  auth_token_env?: string | null;
  oauth_client_id_env?: string | null;
  oauth_client_secret_env?: string | null;
  oauth_scopes: string[];
  oauth_resource?: string | null;
  allowed_tools: string[];
  has_static_env: boolean;
  has_advanced_policy: boolean;
}

export interface McpConfigResponse {
  config_path: string;
  enabled: boolean;
  restart_required: boolean;
  servers: McpServerConfigItem[];
}

export interface McpLifecycleSnapshot {
  server_id: string;
  state: "disabled" | "starting" | "ready" | "degraded" | "stopped" | string;
  transport: string;
  auth_mode: string;
  tool_count: number;
  last_error_code?: string | null;
}

export interface McpToolSummary {
  capability: string;
  server_id: string;
  tool_name: string;
  required_args: string[];
  optional_args: string[];
}

export interface McpProbeOutcome {
  server_id: string;
  status: string;
  latency_ms: number;
}

export interface HookAdminHandler {
  id: string;
  stage: string;
  kind: string;
  enabled: boolean;
  blocking: boolean;
  trusted: boolean;
  trust_status: string;
  content_hash_configured: boolean;
  status: string;
  error_code?: string | null;
  redacted_config: Record<string, unknown>;
}

export interface HookAdminStatus {
  schema_version: number;
  config_path: string;
  setup_state: string;
  default_safe: boolean;
  fail_closed: boolean;
  enabled: boolean;
  handler_count: number;
  enabled_handler_count: number;
  valid_handler_count: number;
  invalid_handler_count: number;
  config_error_code?: string | null;
  supported_stages: string[];
  setup: {
    mode: string;
    ui_enable_supported: boolean;
    trust_required: boolean;
    content_hash_required_for_command: boolean;
    raw_config_redacted: boolean;
  };
  handlers: HookAdminHandler[];
}

export interface NniDeviceMeta {
  slot?: number | null;
  i2c_bus?: number | null;
  i2c_baud?: number | null;
  i2c_address?: string | null;
  lib_path?: string | null;
  simulated?: boolean;
  device_kind?: "hardware" | "simulated" | "unavailable" | string | null;
}

export interface NniDeviceStatusResponse {
  nni_available: boolean;
  helper_available: boolean;
  signature_chip_present: boolean;
  hardware_chip_present?: boolean;
  signer_available?: boolean;
  local_participation_eligible?: boolean;
  signer_kind?: "hardware" | "simulated" | "unavailable" | string;
  network_authorization?: "unknown" | "authorized" | "rejected" | string;
  simulated?: boolean;
  device_kind?: "hardware" | "simulated" | "unavailable" | string | null;
  simulation_available?: boolean;
  status: string;
  message?: string | null;
  message_key?: string | null;
  next_step?: string | null;
  next_step_key?: string | null;
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
  i2c_bus?: number | null;
  i2c_baud?: number | null;
  i2c_address?: string | null;
  lib_path?: string | null;
  simulated?: boolean;
  device_kind?: "hardware" | "simulated" | "unavailable" | string;
  signature_chip_present?: boolean;
  simulation_enabled?: boolean;
  [key: string]: unknown;
}

export interface NniDeviceActionResponse {
  action: string;
  signature_chip_present: boolean;
  simulated?: boolean;
  device_kind?: "hardware" | "simulated" | "unavailable" | string | null;
  message?: string | null;
  message_key?: string | null;
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
  asset_owner_pubkey?: string | null;
  authorization_epoch?: number | null;
  owner_signature_required?: boolean;
  previous_owner_signature_required?: boolean;
  previous_asset_owner_pubkey?: string | null;
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
  asset_owner_pubkey?: string | null;
  authorization_epoch?: number | null;
  authorization_status?: string;
}

export interface NniOwnerRecoveryResponse {
  status: string;
  asset_owner_pubkey: string;
  device_pubkey: string;
  authorization_epoch: number;
  authorization_status: string;
  authorized_at_unix: number;
  node_url: string;
}

export interface NniOwnerRecoveryChallengeResponse {
  schema_version: 1;
  status: "asset_recovery_challenge_created";
  task_id: string;
  signing_payload: string;
  device_signature: string;
  asset_owner_pubkey: string;
  previous_device_pubkey: string;
  new_device_pubkey: string;
  previous_authorization_epoch: number;
  authorization_epoch: number;
  device_signature_required: true;
  owner_signature_required: true;
  expires_at_unix: number;
  node_url: string;
}

export interface NniOwnerUnbindTaskResponse {
  status: string;
  task_id: string;
  signing_payload: string;
  device_pubkey: string;
  asset_owner_pubkey: string;
  authorization_epoch: number;
  device_signature_required: true;
  owner_signature_required: false;
  expires_at_unix: number;
  node_url: string;
}

export interface NniOwnerUnbindVerifyResponse {
  status: string;
  device_pubkey: string;
  asset_owner_pubkey: string;
  authorization_epoch: number;
  authorization_status: "revoked" | string;
  revoked_at_unix: number;
  node_url: string;
  joined: false;
}

export interface NniConfigResponse {
  remote_nodes: string[];
  selected_node_url?: string | null;
  bancor_service_node_url?: string | null;
  asset_service_node_url?: string | null;
  joined: boolean;
  asset_owner_pubkey?: string | null;
  heartbeat_interval_seconds: number;
  heartbeat_network_retry_limit: number;
  heartbeat_request_count: number;
  last_heartbeat_at_ts?: number | null;
  last_heartbeat_error?: string | null;
  last_heartbeat_error_code?: string | null;
  last_heartbeat_error_at_ts?: number | null;
  last_heartbeat_network_failures: number;
  last_heartbeat_attempt_at_ts?: number | null;
  consecutive_heartbeat_failures?: number;
  last_success_node_host?: string | null;
  network_authorization?: "unknown" | "authorized" | "rejected" | string;
  heartbeat_state?: "disabled" | "enabling" | "active" | "waiting_network" | "rejected" | "degraded" | string;
  next_heartbeat_due_at_ts?: number | null;
  worker_running?: boolean;
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

export interface NniRewardRecord {
  id: number;
  period_start_unix: number;
  period_end_unix: number;
  heartbeat_count_in_period: number;
  eligibility_units: 1;
  reward_aic_units: string;
  reward_aic_scale: 100000000;
  reward_aic: string;
  rounding_adjustment_units: number;
  awarded_at_unix: number;
}

export type NniRewardWindowKey = "week" | "month" | "year";

export interface NniRewardWindowSummary {
  key: NniRewardWindowKey;
  window_seconds: number;
  window_start_unix: number;
  window_end_unix: number;
  total_reward_units: string;
  total_reward_aic: string;
  reward_grant_count: number;
}

export interface NniNetworkDeviceStats {
  registered_device_count: number;
  active_device_count: number;
  active_period_start_unix: number | null;
  active_period_end_unix: number | null;
  first_heartbeat_unix: number | null;
  window_seconds: number;
}

export interface NniRewardPolicy {
  phase?: "disabled" | "scheduled" | "waiting_first_heartbeat" | "active" | string;
  accepting_reward_heartbeats?: boolean;
  activation_not_before_unix?: number;
  reward_start_time_unix?: number | null;
  starts_in_seconds?: number | null;
  first_settlement_at_unix?: number | null;
  interval_seconds: number;
  initial_reward_pool_aic: number;
  current_reward_pool_units: string | null;
  current_reward_pool_aic: string | null;
  distribution: "equal_per_eligible_device";
  halving_epoch_unix: number | null;
  halving_interval_seconds: number;
  halving_era: number | null;
  rewards_ended: boolean;
  next_halving_at_unix: number | null;
}

export interface NniNetworkRewards {
  total_distributed_reward_units: string;
  total_distributed_reward_aic: string;
  settled_period_count: number;
  first_period_start_unix: number | null;
  latest_period_end_unix: number | null;
}

export interface NniNetworkStatsResponse {
  schema_version: 1;
  status: "heartbeat_network_stats";
  node_url?: string;
  network_devices: NniNetworkDeviceStats;
  reward_policy: NniRewardPolicy;
  network_rewards: NniNetworkRewards;
}

export interface NniRewardsResponse {
  schema_version: 1;
  status: string;
  device_pubkey: string;
  node_url?: string;
  reward_aic_scale: 100000000;
  reward_decimal_places: 8;
  total_reward_units: string;
  total_reward_aic: string;
  reward_grant_count: number;
  first_period_start_unix?: number | null;
  latest_period_end_unix?: number | null;
  reward_windows?: NniRewardWindowSummary[];
  network_devices?: NniNetworkDeviceStats;
  reward_policy?: NniRewardPolicy;
  network_rewards?: NniNetworkRewards;
  page: number;
  per_page: number;
  total: number;
  total_pages: number;
  history_limit: number;
  history_truncated: boolean;
  records: NniRewardRecord[];
}

export interface NniBancorMarketResponse {
  schema_version: 1;
  status: "open" | "disabled" | "paused";
  market_id: string;
  aic_symbol: "AIC";
  usd_symbol: "USD";
  aic_scale: 100000000;
  usd_scale: 100000000;
  aic_reserve_units: string;
  aic_reserve: string;
  usd_reserve_units: string;
  usd_reserve: string;
  marginal_price_usd_per_aic: string;
  daily_marginal_price: {
    price_kind: "pool_marginal_usd_per_aic";
    timezone: "UTC";
    day_start_unix: number;
    open_usd_per_aic: string;
    high_usd_per_aic: string;
    low_usd_per_aic: string;
    change_percent: string;
    trade_count: number;
  };
  min_trade_usd: string;
  min_trade_usd_units: string;
  min_trade_aic: string;
  min_trade_aic_units: string;
  minimum_fee_units: string;
  minimum_output_units: string;
  fee_bps: number;
  version: number;
  last_trade_id?: string | null;
  updated_at_unix: number;
  node_url?: string;
}

export interface NniBancorCandle {
  bucket_start_unix: number;
  bucket_end_unix: number;
  open: string;
  high: string;
  low: string;
  close: string;
  aic_volume_units: string;
  aic_volume: string;
  usd_volume_units: string;
  usd_volume: string;
  trade_count: number;
  has_trades: boolean;
}

export interface NniBancorCandlesResponse {
  schema_version: 1;
  status: "bancor_candles";
  market_id: string;
  market_version: number;
  market_created_at_unix: number;
  price_kind: "execution_average_usd_per_aic";
  interval_seconds: number;
  start_time_unix: number;
  end_time_unix: number;
  price_scale: 1000000000000;
  price_decimal_places: 12;
  candles: NniBancorCandle[];
  node_url?: string;
}

export interface NniBancorQuoteResponse {
  schema_version: 1;
  status: string;
  side: "buy" | "sell";
  input_asset: "AIC" | "USD";
  input_units: string;
  input_amount: string;
  fee_asset: "AIC" | "USD";
  fee_units: string;
  fee_amount: string;
  curve_input_units: string;
  curve_input_amount: string;
  output_asset: "AIC" | "USD";
  output_units: string;
  output_amount: string;
  price_impact_bps: number;
  fee_bps: number;
  market_id: string;
  market_version: number;
  slippage_bps: number;
  min_output_units: string;
  min_output_amount: string;
  node_url?: string;
}

export interface NniBancorTradeRecord {
  trade_id: string;
  quote_id: string;
  market_id: string;
  side: "buy" | "sell";
  input_asset: "AIC" | "USD";
  input_units: string;
  input_amount: string;
  fee_units: string;
  fee_amount: string;
  output_asset: "AIC" | "USD";
  output_units: string;
  output_amount: string;
  market_version: number;
  created_at_unix: number;
}

export interface NniBancorMarketTradeRecord extends NniBancorTradeRecord {
  asset_owner_pubkey: string;
}

export interface NniBancorMarketTradesResponse {
  schema_version: 1;
  status: string;
  market_id: string;
  limit: 100;
  trades: NniBancorMarketTradeRecord[];
  node_url?: string;
}

export interface NniBancorAccountResponse {
  schema_version: 1;
  status: string;
  device_pubkey: string;
  aic_balance_units: string;
  aic_balance: string;
  usd_balance_units: string;
  usd_balance: string;
  account_version: number;
  page: number;
  per_page: number;
  total: number;
  total_pages: number;
  trades: NniBancorTradeRecord[];
  node_url?: string;
}

export interface NniBancorTradeResponse {
  schema_version: 1;
  status: string;
  device_pubkey: string;
  asset_owner_pubkey: string;
  authorization_epoch: number;
  authorization_mode: "delegated_hardware" | "asset_owner";
  trade: NniBancorTradeRecord;
  account: {
    aic_balance_units: string;
    aic_balance: string;
    usd_balance_units: string;
    usd_balance: string;
    version: number;
  };
  market: NniBancorMarketResponse;
  node_url?: string;
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

export type LarkConfigResponse = FeishuConfigResponse;

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
  runtime_enabled?: boolean | null;
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

export interface ModelCatalogEntry {
  schema_version: number;
  provider: string;
  model: string;
  models: string[];
  api_style: string;
  base_url_kind: string;
  context_window_tokens?: number | null;
  timeout_seconds?: number | null;
  credential_state?: string | null;
  input_modalities: string[];
  output_modalities: string[];
  supports_text: boolean;
  supports_image_input: boolean;
  supports_video_input: boolean;
  supports_audio_input: boolean;
  supports_image_understanding: boolean;
  supports_audio_transcription: boolean;
  supports_image_generation: boolean;
  supports_image_edit: boolean;
  supports_audio_generation: boolean;
  supports_video_generation: boolean;
  supports_music_generation: boolean;
  async_required: boolean;
  dry_run_supported: boolean;
  active_text_provider: boolean;
  config_source: string[];
  capability_source?: string[];
}

export interface ModelCatalogResponse {
  schema_version: number;
  selected_provider: string;
  selected_model: string;
  entries: ModelCatalogEntry[];
  last_guard_status?: {
    available: boolean;
    status: string;
    finding_count?: number;
    path?: string;
    modified_ts?: number | null;
  };
}

export interface LogLatestResponse {
  file: string;
  lines: number;
  text: string;
}

export interface LogFilesResponse {
  files: string[];
}

export interface WhatsappWebLoginStatus {
  adapter_mode?: "experimental_unofficial" | string;
  official_bot_api?: boolean;
  transport?: string;
  phase?: "starting" | "qr_ready" | "connected" | "reconnecting" | "logged_out" | "error" | string;
  connected?: boolean;
  qr_ready?: boolean;
  qr_data_url?: string | null;
  last_update_ts?: number;
  last_error_code?: string | null;
  last_diagnostic_id?: string | null;
  proactive_send_enabled?: boolean;
  local_safety_limits?: {
    image_bytes?: number;
    video_bytes?: number;
    audio_bytes?: number;
    file_bytes?: number;
  };
  /** Legacy bridge responses only. New adapters expose machine error fields above. */
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
  artifacts?: TaskArtifact[];
  artifactDelivery?: {
    schema_version: 1;
    candidate_count: number;
    delivered_count: number;
    truncated: boolean;
    max_items: number;
  };
  bodyResult?: ConversationBodyDescriptor | null;
}

interface TaskArtifactFields {
  id: string;
  filename: string;
  kind: string;
  mime_type: string;
  size_bytes: number;
  sha256: string;
  download_url: string;
  preview_url?: string | null;
}

export type TaskArtifact =
  | (TaskArtifactFields & {
      schema_version: 1;
      artifact_ref?: string;
    })
  | (TaskArtifactFields & {
      schema_version: 2;
      artifact_ref: string;
    });

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

export interface UiAttachmentConstraints {
  schema_version: 1;
  status: "ok";
  channel: "ui_base64";
  max_attachments: number;
  max_attachment_bytes: number;
  max_total_attachment_bytes: number;
  error_codes: string[];
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

export interface HostUnavailableField {
  field: string;
  code: string;
}

export interface HostCapacitySummary {
  total_bytes: number | null;
  available_bytes: number | null;
  available_ratio: number | null;
}

export interface HostSystemSummary {
  schema_version: number;
  collected_at_ts: number;
  os: {
    family: string;
    name: string | null;
    version: string | null;
    kernel: string | null;
  };
  architecture: string;
  deployment: string | null;
  memory: HostCapacitySummary;
  storage: HostCapacitySummary;
  uptime_seconds: number | null;
  unavailable_fields: HostUnavailableField[];
}

export type HostDependencyCategory = "runtime" | "build" | "tool" | "skill" | "optional";

export interface HostDependencySummary {
  total: number;
  installed: number;
  missing_required: number;
  missing_optional: number;
}

export interface HostDependencyStatus {
  id: string;
  category: HostDependencyCategory;
  required: boolean;
  installed: boolean;
  version: string | null;
  executable: string | null;
  package_manager: string | null;
  installable: boolean;
  used_by: string[];
  status_code: "installed" | "missing_required" | "missing_optional" | string;
}

export interface DependencyInstallOperation {
  schema_version: number;
  operation_id: string;
  dependency_id: string;
  status: "queued" | "running" | "succeeded" | "failed" | string;
  package_manager: string;
  started_ts: number | null;
  finished_ts: number | null;
  exit_code: number | null;
  log_tail: string;
  error_code: string | null;
}

export interface HostDependenciesSnapshot {
  schema_version: number;
  collected_at_ts: number;
  platform: string;
  package_manager: string | null;
  summary: HostDependencySummary;
  dependencies: HostDependencyStatus[];
  operations: DependencyInstallOperation[];
}

export interface ServiceActionNotice {
  tone: "success" | "error";
  text: string;
}

export type ChannelName = "telegram" | "whatsapp" | "ui" | "wechat" | "feishu" | "lark";
export type ConsolePage = "dashboard" | "chat" | "ai_learning" | "nni" | "nni_apr" | "bancor" | "assets" | "services" | "channels" | "models" | "skills" | "skill_store" | "memory" | "logs" | "tasks";
