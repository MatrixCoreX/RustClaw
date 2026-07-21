pub(super) fn default_skill_timeout_seconds() -> u64 {
    30
}

pub(super) fn default_skill_max_concurrency() -> usize {
    1
}

/// UI 中归类为「基础技能」并默认固定开启：文件基础能力 + 系统维护基础能力。
pub fn base_skill_names() -> &'static [&'static str] {
    &[
        "run_cmd",
        "code_index",
        "fs_basic",
        "config_basic",
        "config_edit",
        "read_file",
        "write_file",
        "list_dir",
        "make_dir",
        "remove_file",
        "schedule",
        "extension_manager",
        "kb",
        "system_basic",
        "process_basic",
        "config_guard",
        "fs_search",
        "git_basic",
        "service_control",
        "archive_basic",
    ]
}

/// UI 保存技能开关时强制保持开启的技能；手工编辑 config.toml 仍遵守「false = 强制关闭」契约。
pub fn core_skills_always_enabled() -> &'static [&'static str] {
    &[
        "run_cmd",
        "code_index",
        "fs_basic",
        "config_basic",
        "config_edit",
        "read_file",
        "write_file",
        "list_dir",
        "make_dir",
        "remove_file",
        "schedule",
        "extension_manager",
        "kb",
        "system_basic",
        "process_basic",
        "config_guard",
        "fs_search",
        "git_basic",
        "service_control",
        "archive_basic",
    ]
}

pub(super) fn default_skills_list() -> Vec<String> {
    // Keep in sync with `configs/skills_registry.toml` [[skills]] names (no-registry fallback baseline).
    vec![
        "run_cmd".to_string(),
        "code_index".to_string(),
        "fs_basic".to_string(),
        "config_basic".to_string(),
        "config_edit".to_string(),
        "read_file".to_string(),
        "write_file".to_string(),
        "list_dir".to_string(),
        "make_dir".to_string(),
        "remove_file".to_string(),
        "schedule".to_string(),
        "x".to_string(),
        "system_basic".to_string(),
        "http_basic".to_string(),
        "git_basic".to_string(),
        "install_module".to_string(),
        "process_basic".to_string(),
        "package_manager".to_string(),
        "archive_basic".to_string(),
        "db_basic".to_string(),
        "docker_basic".to_string(),
        "fs_search".to_string(),
        "rss_fetch".to_string(),
        "image_vision".to_string(),
        "image_generate".to_string(),
        "image_edit".to_string(),
        "audio_transcribe".to_string(),
        "audio_synthesize".to_string(),
        "video_generate".to_string(),
        "music_generate".to_string(),
        "health_check".to_string(),
        "log_analyze".to_string(),
        "service_control".to_string(),
        "task_control".to_string(),
        "config_guard".to_string(),
        "map_merchant".to_string(),
        "crypto".to_string(),
        "stock".to_string(),
        "weather".to_string(),
        "doc_parse".to_string(),
        "transform".to_string(),
        "invest_copy".to_string(),
        "web_search_extract".to_string(),
        "kb".to_string(),
        "browser_web".to_string(),
        "extension_manager".to_string(),
    ]
}

pub(super) fn default_global_rpm() -> usize {
    60
}

pub(super) fn default_user_rpm() -> usize {
    20
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
    "local-hash-v1".to_string()
}

pub(super) fn default_memory_embedding_dims() -> usize {
    24
}

pub(super) fn default_memory_embedding_version() -> String {
    "local-hash-v1".to_string()
}

pub(super) fn default_memory_embedding_batch_size() -> usize {
    16
}

pub(super) fn default_memory_reindex_on_startup() -> bool {
    false
}

pub(super) fn default_worker_concurrency() -> usize {
    1
}

pub(super) fn default_worker_task_timeout_seconds() -> u64 {
    // 1 小时单任务硬上限。比 demo 模板里的 86400 (24h) 安全得多。
    // 真的需要长任务（视频处理、大批量同步）就在 toml 中显式覆盖。
    3600
}

pub(super) fn default_worker_llm_max_calls_per_task() -> u64 {
    40
}

pub(super) fn default_worker_llm_total_timeout_seconds() -> u64 {
    // Mimo 等慢模型经常需要 normalizer + planner + synthesis 连续调用；
    // 900s 仍低于单任务硬超时，但能覆盖长文 synthesis + verifier 的慢调用组合。
    900
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

pub(super) fn default_telegram_quick_result_wait_seconds() -> u64 {
    3
}

pub(super) fn default_telegram_task_delivery_timeout_seconds() -> u64 {
    600
}

pub(super) fn default_whatsapp_api_base() -> String {
    "https://graph.facebook.com".to_string()
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

pub(super) fn default_telegram_auto_vision_on_image_only() -> bool {
    true
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

pub(super) fn default_telegram_voice_mode_nl_intent_enabled() -> bool {
    true
}

pub(super) fn default_telegram_max_audio_input_bytes() -> usize {
    25 * 1024 * 1024
}

pub(super) fn default_telegram_ephemeral_image_saved_seconds() -> u64 {
    15
}

pub(super) fn default_sendfile_admin_only() -> bool {
    false
}

pub(super) fn default_sendfile_full_access() -> bool {
    true
}

pub(super) fn default_sendfile_allowed_dirs() -> Vec<String> {
    vec!["image/download".to_string(), "document".to_string()]
}

pub(super) fn default_tool_cmd_timeout_seconds() -> u64 {
    60
}

pub(super) fn default_tool_cmd_idle_timeout_seconds() -> u64 {
    60
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

pub(super) fn default_routing_locator_scan_max_depth() -> usize {
    2
}

pub(super) fn default_routing_locator_scan_max_files() -> usize {
    800
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
