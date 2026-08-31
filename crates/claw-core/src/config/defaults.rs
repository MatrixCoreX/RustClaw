pub(super) fn default_skill_timeout_seconds() -> u64 {
    30
}

pub(super) fn default_skill_max_concurrency() -> usize {
    1
}

pub(super) fn default_runner_warm_pool_max_idle_per_skill() -> usize {
    1
}

pub(super) fn default_runner_warm_pool_min_available_memory_mib() -> u64 {
    512
}

pub(super) fn default_runner_warm_pool_idle_timeout_seconds() -> u64 {
    60
}

pub(super) fn default_uninstalled_skills() -> Vec<String> {
    Vec::new()
}

pub(super) fn default_skills_list() -> Vec<String> {
    Vec::new()
}

pub(super) fn default_global_rpm() -> usize {
    60
}

pub(super) fn default_user_rpm() -> usize {
    20
}

pub(super) fn default_context_tool_observation_reserve_tokens() -> usize {
    2_048
}

pub(super) fn default_context_estimator_safety_margin_tokens() -> usize {
    512
}

pub(super) fn default_cleanup_interval_seconds() -> u64 {
    300
}

pub(super) fn default_tasks_retention_days() -> u64 {
    7
}

pub(super) fn default_tasks_max_rows() -> usize {
    2000
}

pub(super) fn default_audit_retention_days() -> u64 {
    14
}

pub(super) fn default_audit_max_rows() -> usize {
    10000
}

pub(super) fn default_memory_mark_llm_reply_in_short_term() -> bool {
    true
}

pub(super) fn default_memory_config_path() -> String {
    "configs/memory.toml".to_string()
}

pub(super) fn default_memory_prefer_llm_assistant_memory() -> bool {
    false
}

pub(super) fn default_memory_prompt_recall_limit() -> usize {
    3
}

pub(super) fn default_memory_recall_limit() -> usize {
    8
}

pub(super) fn default_memory_background_job_concurrency() -> usize {
    2
}

pub(super) fn default_memory_background_idle_seconds() -> u64 {
    5
}

pub(super) fn default_memory_background_lease_seconds() -> u64 {
    90
}

pub(super) fn default_memory_background_max_attempts() -> usize {
    5
}

pub(super) fn default_memory_raw_candidate_retention_days() -> u64 {
    14
}

pub(super) fn default_memory_raw_candidate_max_rows_per_principal() -> usize {
    2_000
}

pub(super) fn default_memory_storage_soft_limit_bytes() -> u64 {
    512 * 1024 * 1024
}

pub(super) fn default_memory_principal_max_bytes() -> u64 {
    64 * 1024 * 1024
}

pub(super) fn default_memory_principal_background_cost_microunits() -> u64 {
    1_000_000
}

pub(super) fn default_memory_item_max_chars() -> usize {
    2000
}

pub(super) fn default_memory_prompt_max_chars() -> usize {
    8000
}

pub(super) fn default_memory_retention_days() -> u64 {
    30
}

pub(super) fn default_memory_max_rows() -> usize {
    50000
}

pub(super) fn default_memory_long_term_enabled() -> bool {
    true
}

pub(super) fn default_memory_long_term_every_rounds() -> usize {
    6
}

pub(super) fn default_memory_long_term_source_rounds() -> usize {
    20
}

pub(super) fn default_memory_long_term_summary_max_chars() -> usize {
    3000
}

pub(super) fn default_memory_long_term_recall_max_chars() -> usize {
    1200
}

pub(super) fn default_memory_long_term_retention_days() -> u64 {
    180
}

pub(super) fn default_memory_long_term_max_rows() -> usize {
    10000
}

pub(super) fn default_memory_write_filter_enabled() -> bool {
    true
}

pub(super) fn default_memory_write_min_chars() -> usize {
    12
}

pub(super) fn default_memory_enable_preference_extraction() -> bool {
    true
}

pub(super) fn default_memory_llm_preference_fallback_enabled() -> bool {
    false
}

pub(super) fn default_memory_llm_preference_min_confidence() -> f32 {
    0.72
}

pub(super) fn default_memory_llm_preference_max_chars() -> usize {
    900
}

pub(super) fn default_memory_preference_recall_limit() -> usize {
    8
}

pub(super) fn default_memory_recent_relevance_enabled() -> bool {
    true
}

pub(super) fn default_memory_recent_relevance_min_score() -> f32 {
    0.16
}

pub(super) fn default_memory_safety_filter_enabled() -> bool {
    true
}

pub(super) fn default_memory_long_term_refresh_min_new_chars() -> usize {
    80
}

pub(super) fn default_memory_long_term_refresh_max_repeat_ratio() -> f32 {
    0.7
}

pub(super) fn default_memory_route_memory_enabled() -> bool {
    true
}

pub(super) fn default_memory_route_memory_max_chars() -> usize {
    1400
}

pub(super) fn default_memory_skill_memory_enabled() -> bool {
    true
}

pub(super) fn default_memory_skill_memory_max_chars() -> usize {
    1800
}

pub(super) fn default_memory_schedule_memory_include_long_term() -> bool {
    true
}

pub(super) fn default_memory_schedule_memory_include_preferences() -> bool {
    true
}

pub(super) fn default_memory_schedule_memory_max_chars() -> usize {
    1600
}

pub(super) fn default_memory_image_memory_include_long_term() -> bool {
    true
}

pub(super) fn default_memory_image_memory_include_preferences() -> bool {
    true
}

pub(super) fn default_memory_image_memory_max_chars() -> usize {
    1400
}

pub(super) fn default_memory_hybrid_recall_enabled() -> bool {
    true
}

pub(super) fn default_memory_fts_candidate_limit() -> usize {
    24
}

pub(super) fn default_memory_vector_candidate_limit() -> usize {
    24
}

pub(super) fn default_memory_trigger_anchor_limit() -> usize {
    2
}

pub(super) fn default_memory_fact_card_limit() -> usize {
    3
}

pub(super) fn default_memory_chat_memory_budget_chars() -> usize {
    1200
}

pub(super) fn default_memory_agent_memory_budget_chars() -> usize {
    2200
}

pub(super) fn default_memory_route_trigger_budget_chars() -> usize {
    900
}

pub(super) fn default_memory_embedding_model() -> String {
    "local-hash-v2".to_string()
}

pub(super) fn default_memory_embedding_dims() -> usize {
    24
}

pub(super) fn default_memory_embedding_version() -> String {
    "local-hash-v2".to_string()
}

pub(super) fn default_memory_embedding_batch_size() -> usize {
    16
}

pub(super) fn default_memory_embedding_provider_kind() -> String {
    "local".to_string()
}

pub(super) fn default_memory_embedding_normalization() -> String {
    "unit_length".to_string()
}

pub(super) fn default_memory_embedding_metric() -> String {
    "cosine".to_string()
}

pub(super) fn default_memory_embedding_query_timeout_ms() -> u64 {
    1_500
}

pub(super) fn default_memory_embedding_connect_timeout_ms() -> u64 {
    1_000
}

pub(super) fn default_memory_embedding_idle_timeout_ms() -> u64 {
    1_500
}

pub(super) fn default_memory_embedding_retry_max_attempts() -> usize {
    5
}

pub(super) fn default_memory_embedding_circuit_failure_threshold() -> usize {
    3
}

pub(super) fn default_memory_embedding_circuit_reset_seconds() -> u64 {
    30
}

pub(super) fn default_memory_embedding_query_cache_ttl_seconds() -> u64 {
    300
}

pub(super) fn default_memory_embedding_query_cache_max_bytes() -> usize {
    1_048_576
}

pub(super) fn default_memory_embedding_max_request_bytes() -> usize {
    2_097_152
}

pub(super) fn default_memory_embedding_remote_opt_in_required() -> bool {
    true
}

pub(super) fn default_memory_embedding_reindex_batch_delay_ms() -> u64 {
    25
}

pub(super) fn default_memory_reindex_on_startup() -> bool {
    false
}

pub(super) fn default_worker_concurrency() -> usize {
    1
}

pub(super) fn default_worker_poll_interval_ms() -> u64 {
    500
}

pub(super) fn default_worker_queue_limit() -> usize {
    64
}

pub(super) fn default_worker_task_heartbeat_seconds() -> u64 {
    30
}

pub(super) fn default_worker_running_no_progress_timeout_seconds() -> u64 {
    20 * 60
}

pub(super) fn default_worker_running_recovery_check_interval_seconds() -> u64 {
    60
}

pub(super) fn default_tools_profile() -> String {
    "coding".to_string()
}

pub(super) fn default_admin_tools_profile() -> String {
    "full".to_string()
}

pub(super) fn default_tool_access_profiles() -> std::collections::HashMap<String, Vec<String>> {
    #[derive(serde::Deserialize)]
    struct ProfileFile {
        profiles: std::collections::HashMap<String, Vec<String>>,
    }

    toml::from_str::<ProfileFile>(include_str!(
        "../../../../configs/tool_access_profiles.toml"
    ))
    .expect("configs/tool_access_profiles.toml must be valid")
    .profiles
}

pub(super) fn default_telegram_quick_result_wait_seconds() -> u64 {
    3
}

pub(super) fn default_telegram_task_delivery_timeout_seconds() -> u64 {
    600
}

pub(super) fn default_whatsapp_api_base() -> String {
    "https://graph.facebook.com".to_string()
}

pub(super) fn default_whatsapp_template_language() -> String {
    "en_US".to_string()
}

pub(super) fn default_whatsapp_webhook_listen() -> String {
    "127.0.0.1:8091".to_string()
}

pub(super) fn default_whatsapp_webhook_path() -> String {
    "/webhook".to_string()
}

pub(super) fn default_whatsapp_quick_result_wait_seconds() -> u64 {
    3
}

pub(super) fn default_whatsapp_task_delivery_timeout_seconds() -> u64 {
    600
}

pub(super) fn default_whatsapp_i18n_path() -> String {
    "configs/i18n/whatsapp-cloud.en-US.toml".to_string()
}

pub(super) fn default_whatsapp_language() -> String {
    "en-US".to_string()
}

pub(super) fn default_whatsapp_image_inbox_dir() -> String {
    "image/upload".to_string()
}

pub(super) fn default_whatsapp_audio_inbox_dir() -> String {
    "audio/upload".to_string()
}

pub(super) fn default_whatsapp_web_bridge_listen() -> String {
    "127.0.0.1:8092".to_string()
}

pub(super) fn default_whatsapp_web_bridge_base_url() -> String {
    "http://127.0.0.1:8092".to_string()
}

pub(super) fn default_whatsapp_web_wrapper_listen() -> String {
    "127.0.0.1:8094".to_string()
}

pub(super) fn default_whatsapp_web_auth_dir() -> String {
    "data/wa-web-auth".to_string()
}

pub(super) fn default_whatsapp_web_quick_result_wait_seconds() -> u64 {
    3
}

pub(super) fn default_whatsapp_web_max_outbound_image_bytes() -> u64 {
    100 * 1024 * 1024
}

pub(super) fn default_whatsapp_web_max_outbound_video_bytes() -> u64 {
    100 * 1024 * 1024
}

pub(super) fn default_whatsapp_web_max_outbound_audio_bytes() -> u64 {
    100 * 1024 * 1024
}

pub(super) fn default_whatsapp_web_max_outbound_file_bytes() -> u64 {
    2 * 1024 * 1024 * 1024
}

pub(super) fn default_whatsapp_web_i18n_path() -> String {
    "configs/i18n/whatsapp-webd.en-US.toml".to_string()
}

pub(super) fn default_whatsapp_web_language() -> String {
    "en-US".to_string()
}

pub(super) fn default_telegram_i18n_path() -> String {
    "configs/i18n/telegramd.zh-CN.toml".to_string()
}

pub(super) fn default_telegram_access_mode() -> String {
    "public".to_string()
}

pub(super) fn default_telegram_language() -> String {
    "zh-CN".to_string()
}

pub(super) fn default_telegram_update_mode() -> String {
    "polling".to_string()
}

pub(super) fn default_telegram_webhook_listen() -> String {
    "127.0.0.1:8090".to_string()
}

pub(super) fn default_telegram_webhook_secret_env() -> String {
    "TELEGRAM_WEBHOOK_SECRET".to_string()
}

pub(super) fn default_telegram_image_inbox_dir() -> String {
    "data/telegramd/image".to_string()
}

pub(super) fn default_telegram_video_inbox_dir() -> String {
    "data/telegramd/video".to_string()
}

pub(super) fn default_telegram_file_inbox_dir() -> String {
    "data/telegramd/file".to_string()
}

pub(super) fn default_telegram_audio_inbox_dir() -> String {
    "data/telegramd/audio".to_string()
}

pub(super) fn default_telegram_voice_reply_mode() -> String {
    "voice".to_string()
}

pub(super) fn default_telegram_max_audio_input_bytes() -> usize {
    25 * 1024 * 1024
}

pub(super) fn default_telegram_ephemeral_image_saved_seconds() -> u64 {
    15
}

pub(super) fn default_tool_cmd_timeout_seconds() -> u64 {
    180
}

pub(super) fn default_tool_cmd_idle_timeout_seconds() -> u64 {
    120
}

pub(super) fn default_tool_cmd_async_retention_seconds() -> u64 {
    86_400
}

pub(super) fn default_tool_cmd_terminate_grace_seconds() -> u64 {
    5
}

pub(super) fn default_tool_cmd_max_output_bytes() -> usize {
    8000
}

pub(super) fn default_tool_max_cmd_length() -> usize {
    240
}

pub(super) fn default_llm_timeout_seconds() -> u64 {
    30
}

pub(super) fn default_llm_max_concurrency() -> usize {
    1
}

pub(super) fn default_image_default_output_dir() -> String {
    "image".to_string()
}

pub(super) fn default_image_timeout_seconds() -> u64 {
    90
}

pub(super) fn default_image_max_concurrency() -> usize {
    1
}

pub(super) fn default_image_max_images() -> usize {
    6
}

pub(super) fn default_image_max_input_bytes() -> usize {
    10 * 1024 * 1024
}

pub(super) fn default_command_intent_default_locale() -> String {
    "zh-CN".to_string()
}

pub(super) fn default_schedule_timezone() -> String {
    "Asia/Shanghai".to_string()
}

pub(super) fn default_schedule_intent_prompt_path() -> String {
    "prompts/schedule_intent_prompt.md".to_string()
}

pub(super) fn default_schedule_intent_rules_path() -> String {
    "prompts/schedule_intent_rules.md".to_string()
}

pub(super) fn default_schedule_locale() -> String {
    "zh-CN".to_string()
}

pub(super) fn default_schedule_i18n_dir() -> String {
    "configs/i18n".to_string()
}

pub(super) fn default_routing_default_locator_search_dir() -> String {
    ".".to_string()
}

pub(super) fn default_persona_profile() -> String {
    "executor".to_string()
}

pub(super) fn default_persona_dir() -> String {
    "prompts/personas".to_string()
}

pub(super) fn default_agent_id() -> String {
    "main".to_string()
}
