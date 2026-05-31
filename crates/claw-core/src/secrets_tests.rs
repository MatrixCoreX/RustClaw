use super::*;

/// 生成一个进程内唯一的小写 secret name。
/// 注意：env 是进程全局态，必须用唯一名字避免并发测试相互覆盖。
/// 返回值已经是合法的 [a-z0-9_] canonical secret name。
fn fresh_var(suffix: &str) -> String {
    format!("claw_test_e1a_{}_{}", std::process::id(), suffix)
}

#[test]
fn secret_value_debug_does_not_leak_plaintext() {
    let s = SecretValue::new("super-secret-token");
    let printed = format!("{s:?}");
    assert!(!printed.contains("super-secret-token"), "{printed}");
    assert!(printed.contains("[REDACTED]"), "{printed}");
    // len 字段还在
    assert!(printed.contains("len: 18"), "{printed}");
}

#[test]
fn secret_value_expose_returns_plaintext_only_via_explicit_call() {
    let s = SecretValue::new("abc");
    assert_eq!(s.expose(), "abc");
    assert_eq!(s.len(), 3);
    assert!(!s.is_empty());
}

#[test]
fn validate_secret_name_accepts_canonical_usage_vendor_form() {
    for ok in [
        "image_generation_minimax_api_key",
        "image_edit_qwen_api_key",
        "image_vision_openai_api_key",
        "text_openai_api_key",
        "chat_minimax_api_key",
        // 长度恰到 64：a + _ * 62 + a = 64 字符
        &format!("a{}a", "_".repeat(62)),
    ] {
        assert!(
            validate_secret_name(ok).is_ok(),
            "expected `{ok}` to pass validation"
        );
    }
}

#[test]
fn validate_secret_name_rejects_bad_shapes() {
    for bad in ["", "  ", "Has-Dash", "has space", "UPPER", "中文"] {
        assert!(
            validate_secret_name(bad).is_err(),
            "expected `{bad}` to be rejected"
        );
    }
    let too_long = "a".repeat(65);
    assert!(validate_secret_name(&too_long).is_err());
}

#[test]
fn validate_secret_name_rejects_bare_vendor_pattern() {
    // 与 Capability::parse 的拒绝集合保持同步
    for bare in [
        "openai_api_key",
        "minimax_api_key",
        "anthropic_api_key",
        "claude_api_key",
        "openai",
        "minimax",
    ] {
        let err = validate_secret_name(bare)
            .err()
            .unwrap_or_else(|| panic!("expected `{bare}` to be rejected"));
        let msg = format!("{err}");
        assert!(
            msg.contains("bare vendor naming"),
            "expected bare-vendor message, got: {msg}"
        );
        assert!(
            msg.contains("<usage>_"),
            "expected hint with usage prefix, got: {msg}"
        );
    }
}

#[test]
fn env_broker_returns_none_when_var_missing() {
    let broker = EnvSecretsBroker::new();
    // 用绝对不会被设置的随机名（带 PID 隔离并发测试）
    let canonical = fresh_var("missing");
    // 不 set
    let result = broker.lookup(&canonical).unwrap();
    assert!(result.is_none());
}

#[test]
fn env_broker_returns_some_when_var_set_and_nonempty() {
    let broker = EnvSecretsBroker::new();
    let canonical = fresh_var("present");
    let env_var = broker.env_var_name(&canonical);
    // SAFETY: 单测，自己设自己读，结尾清理
    unsafe { env::set_var(&env_var, "actual-value-xyz") };
    let result = broker.lookup(&canonical).unwrap();
    unsafe { env::remove_var(&env_var) };
    let secret = result.expect("should have found secret");
    assert_eq!(secret.expose(), "actual-value-xyz");
}

#[test]
fn env_broker_treats_empty_env_value_as_missing() {
    let broker = EnvSecretsBroker::new();
    let canonical = fresh_var("empty");
    let env_var = broker.env_var_name(&canonical);
    unsafe { env::set_var(&env_var, "") };
    let result = broker.lookup(&canonical).unwrap();
    unsafe { env::remove_var(&env_var) };
    assert!(
        result.is_none(),
        "empty env value should be treated as None"
    );
}

#[test]
fn env_broker_with_prefix_isolates_lookup_namespace() {
    let broker = EnvSecretsBroker::with_prefix("CLAW_TEST_E1A_PFX_");
    let canonical = fresh_var("ns");
    let env_var = broker.env_var_name(&canonical);
    // 验证 prefix 实际起作用：env_var 必须以 prefix 开头
    assert!(env_var.starts_with("CLAW_TEST_E1A_PFX_CLAW_TEST_E1A_"));
    unsafe { env::set_var(&env_var, "ns-value") };
    let result = broker.lookup(&canonical).unwrap();
    unsafe { env::remove_var(&env_var) };
    let secret = result.expect("prefixed lookup should resolve");
    assert_eq!(secret.expose(), "ns-value");

    // 反向验证：不带 prefix 的同 canonical 在 plain broker 上应当找不到
    let plain = EnvSecretsBroker::new();
    assert!(
        plain.lookup(&canonical).unwrap().is_none(),
        "plain broker must not see the prefixed env var"
    );
}

#[test]
fn env_broker_rejects_invalid_secret_names_before_touching_env() {
    let broker = EnvSecretsBroker::new();
    let err = broker.lookup("openai_api_key").err().unwrap();
    assert!(matches!(err, SecretsError::InvalidName { .. }));
}

#[test]
fn env_broker_env_var_name_uppercases_and_prefixes() {
    let plain = EnvSecretsBroker::new();
    assert_eq!(
        plain.env_var_name("image_generation_minimax_api_key"),
        "IMAGE_GENERATION_MINIMAX_API_KEY"
    );
    let prefixed = EnvSecretsBroker::with_prefix("APP_");
    assert_eq!(
        prefixed.env_var_name("text_openai_api_key"),
        "APP_TEXT_OPENAI_API_KEY"
    );
}

#[test]
fn env_broker_label_is_stable() {
    let broker = EnvSecretsBroker::new();
    assert_eq!(broker.label(), "env");
}

fn fresh_token_store_dir(suffix: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "rustclaw-secret-token-test-{}-{}",
        std::process::id(),
        suffix
    ))
}

#[test]
fn issue_and_redeem_secret_token_round_trips_once() {
    let dir = fresh_token_store_dir("roundtrip");
    let token = issue_secret_token_value_in_dir(
        &dir,
        &SecretValue::new("secret-123"),
        Duration::from_secs(60),
    )
    .expect("token issue");
    let first = redeem_secret_token_reference_in_dir(&dir, &token)
        .expect("first redeem")
        .expect("first redeem should yield value");
    assert_eq!(first, "secret-123");
    let second = redeem_secret_token_reference_in_dir(&dir, &token).unwrap_err();
    assert!(matches!(second, SecretTokenError::Missing { .. }));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn expired_secret_token_is_rejected() {
    let dir = fresh_token_store_dir("expired");
    let token = issue_secret_token_value_in_dir(
        &dir,
        &SecretValue::new("secret-123"),
        Duration::from_millis(1),
    )
    .expect("token issue");
    std::thread::sleep(Duration::from_millis(5));
    let err = redeem_secret_token_reference_in_dir(&dir, &token).unwrap_err();
    assert!(matches!(err, SecretTokenError::Expired { .. }));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn env_non_empty_resolved_redeems_token_and_caches_by_env_name() {
    let dir = fresh_token_store_dir("env-cache");
    let previous_dir = env::var(SECRET_TOKEN_STORE_DIR_ENV).ok();
    let key = fresh_var("token_env").to_ascii_uppercase();
    let token = issue_secret_token_value_in_dir(
        &dir,
        &SecretValue::new("token-secret"),
        Duration::from_secs(60),
    )
    .expect("token issue");
    unsafe { env::set_var(SECRET_TOKEN_STORE_DIR_ENV, &dir) };
    unsafe { env::set_var(&key, &token) };
    let first = env_non_empty_resolved(&key)
        .expect("first resolve")
        .expect("first resolve value");
    let second = env_non_empty_resolved(&key)
        .expect("second resolve")
        .expect("second resolve value");
    assert_eq!(first, "token-secret");
    assert_eq!(second, "token-secret");
    unsafe { env::remove_var(&key) };
    match previous_dir {
        Some(value) => unsafe { env::set_var(SECRET_TOKEN_STORE_DIR_ENV, value) },
        None => unsafe { env::remove_var(SECRET_TOKEN_STORE_DIR_ENV) },
    }
    let _ = fs::remove_dir_all(dir);
}

/// 验证 trait 是 dyn-safe：能放进 `Box<dyn SecretsBroker>`，便于运行期注入。
#[test]
fn secrets_broker_trait_is_dyn_safe() {
    let _boxed: Box<dyn SecretsBroker> = Box::new(EnvSecretsBroker::new());
}

// ========================================================================
// §E1.b: provision_secret_envs 单测
// ========================================================================

/// 测试用 mock broker：内存 map，单测里独立可控，不依赖 env。
struct MockBroker {
    map: std::collections::HashMap<String, String>,
    fail_on: Option<String>, // 命中此 name 时返回 BackendIo
}
impl MockBroker {
    fn new(pairs: &[(&str, &str)]) -> Self {
        Self {
            map: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            fail_on: None,
        }
    }
    fn fail_on(mut self, name: &str) -> Self {
        self.fail_on = Some(name.to_string());
        self
    }
}
impl SecretsBroker for MockBroker {
    fn lookup(&self, name: &str) -> Result<Option<SecretValue>, SecretsError> {
        if Some(name.to_string()) == self.fail_on {
            return Err(SecretsError::BackendIo {
                name: name.to_string(),
                source: std::io::Error::other("mock backend down"),
            });
        }
        Ok(self.map.get(name).map(|v| SecretValue::new(v.clone())))
    }
    fn label(&self) -> &str {
        "mock"
    }
}

#[test]
fn provision_skips_non_secret_capabilities() {
    let broker = MockBroker::new(&[]);
    let caps = vec![
        Capability::Llm,
        Capability::Net,
        Capability::FsRead,
        Capability::FsWrite,
        Capability::Exec,
        Capability::ExecSudo,
    ];
    let out = provision_secret_envs(&broker, &caps).unwrap();
    assert!(
        out.is_empty(),
        "non-secret caps must not produce env entries"
    );
}

#[test]
fn provision_translates_secret_name_to_uppercase_env() {
    let broker = MockBroker::new(&[
        ("image_generation_minimax_api_key", "image-secret"),
        ("text_openai_api_key", "text-secret"),
    ]);
    let caps = vec![
        Capability::Llm,
        Capability::Secrets("image_generation_minimax_api_key".to_string()),
        Capability::Secrets("text_openai_api_key".to_string()),
    ];
    let out = provision_secret_envs(&broker, &caps).unwrap();
    // 字典序：IMAGE_... 在 TEXT_... 前
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].0, "IMAGE_GENERATION_MINIMAX_API_KEY");
    assert_eq!(out[0].1.expose(), "image-secret");
    assert_eq!(out[1].0, "TEXT_OPENAI_API_KEY");
    assert_eq!(out[1].1.expose(), "text-secret");
}

#[test]
fn provision_dedupes_identical_secret_capabilities() {
    // 同一个 secret 名出现两次（manifest 写错时偶发），不应注入两遍。
    let broker = MockBroker::new(&[("text_openai_api_key", "v")]);
    let caps = vec![
        Capability::Secrets("text_openai_api_key".to_string()),
        Capability::Secrets("text_openai_api_key".to_string()),
    ];
    let out = provision_secret_envs(&broker, &caps).unwrap();
    assert_eq!(out.len(), 1);
}

#[test]
fn provision_fails_loud_with_all_missing_secrets_at_once() {
    // 三条 declared，broker 只有一条 → 一次性报告 2 条 missing
    let broker = MockBroker::new(&[("text_openai_api_key", "v")]);
    let caps = vec![
        Capability::Secrets("text_openai_api_key".to_string()),
        Capability::Secrets("image_generation_minimax_api_key".to_string()),
        Capability::Secrets("image_edit_qwen_api_key".to_string()),
    ];
    let err = provision_secret_envs(&broker, &caps).unwrap_err();
    match err {
        ProvisionError::MissingSecrets { mut missing } => {
            missing.sort();
            assert_eq!(
                missing,
                vec![
                    "image_edit_qwen_api_key".to_string(),
                    "image_generation_minimax_api_key".to_string(),
                ]
            );
        }
        other => panic!("expected MissingSecrets, got {other:?}"),
    }
}

#[test]
fn provision_propagates_backend_io_error_immediately() {
    let broker =
        MockBroker::new(&[("text_openai_api_key", "v")]).fail_on("image_edit_qwen_api_key");
    let caps = vec![
        Capability::Secrets("text_openai_api_key".to_string()),
        Capability::Secrets("image_edit_qwen_api_key".to_string()),
    ];
    let err = provision_secret_envs(&broker, &caps).unwrap_err();
    match err {
        ProvisionError::Lookup { name, .. } => {
            assert_eq!(name, "image_edit_qwen_api_key");
        }
        other => panic!("expected Lookup error, got {other:?}"),
    }
}

#[test]
fn provision_output_is_alphabetically_stable() {
    // 不依赖输入顺序：故意倒序声明 capability，输出仍按 env name 字典序
    let broker = MockBroker::new(&[
        ("text_openai_api_key", "x"),
        ("chat_minimax_api_key", "y"),
        ("image_vision_anthropic_api_key", "z"),
    ]);
    let caps = vec![
        Capability::Secrets("text_openai_api_key".to_string()),
        Capability::Secrets("image_vision_anthropic_api_key".to_string()),
        Capability::Secrets("chat_minimax_api_key".to_string()),
    ];
    let out = provision_secret_envs(&broker, &caps).unwrap();
    let names: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "CHAT_MINIMAX_API_KEY",
            "IMAGE_VISION_ANTHROPIC_API_KEY",
            "TEXT_OPENAI_API_KEY",
        ]
    );
}

#[test]
fn provision_handles_empty_capabilities_list() {
    let broker = MockBroker::new(&[]);
    let out = provision_secret_envs(&broker, &[]).unwrap();
    assert!(out.is_empty());
}

#[test]
fn global_or_default_returns_env_broker_when_uninstalled() {
    // 注：本测试不调 install_global —— OnceLock 一旦 set 就锁死，会污染
    // 后续测试。这里只验证"未 install 时 fallback 是 env"。
    let b = global_or_default();
    assert_eq!(b.label(), "env");
}

// ========================================================================
// §P4.4 E3.a: <usage>_<vendor>_api_key 命名 helpers
// ========================================================================

#[test]
fn helper_lowercases_vendor_and_emits_canonical_text_name() {
    assert_eq!(text_secret_name_for_vendor("OpenAI"), "text_openai_api_key");
    assert_eq!(
        text_secret_name_for_vendor("  Anthropic  "),
        "text_anthropic_api_key"
    );
    assert_eq!(
        text_secret_name_for_vendor("MINIMAX"),
        "text_minimax_api_key"
    );
    assert_eq!(text_secret_name_for_vendor("MiMo"), "text_mimo_api_key");
}

#[test]
fn helper_emits_distinct_names_per_image_usage() {
    // 4 个 usage 必须互相独立 —— 同一 vendor 不同 usage 拿到不同 secret 名。
    let v = "qwen";
    let names = [
        text_secret_name_for_vendor(v),
        image_generation_secret_name_for_vendor(v),
        image_edit_secret_name_for_vendor(v),
        image_vision_secret_name_for_vendor(v),
    ];
    let unique: std::collections::HashSet<&str> = names.iter().map(|s| s.as_str()).collect();
    assert_eq!(unique.len(), 4, "4 usages must yield 4 distinct names");
    assert_eq!(names[0], "text_qwen_api_key");
    assert_eq!(names[1], "image_generation_qwen_api_key");
    assert_eq!(names[2], "image_edit_qwen_api_key");
    assert_eq!(names[3], "image_vision_qwen_api_key");
}

#[test]
fn helper_output_passes_validate_secret_name() {
    // 对每个 helper × 每个已知 vendor，输出都必须能过 broker 的二道防线。
    let vendors = [
        "openai",
        "google",
        "gemini",
        "anthropic",
        "claude",
        "grok",
        "xai",
        "deepseek",
        "qwen",
        "minimax",
        "mimo",
        "xiaomi",
    ];
    for v in vendors {
        for name in [
            text_secret_name_for_vendor(v),
            image_generation_secret_name_for_vendor(v),
            image_edit_secret_name_for_vendor(v),
            image_vision_secret_name_for_vendor(v),
        ] {
            assert!(
                validate_secret_name(&name).is_ok(),
                "expected `{name}` to pass validation but it was rejected"
            );
        }
    }
}

#[test]
fn helper_output_does_not_collapse_to_bare_vendor_pattern() {
    // 防御性回归：万一未来谁手抖把 usage 前缀去掉，validate 必须接得住。
    // 这里直接断"helper 不能产出 `openai_api_key` 这种裸形式"。
    for vendor in ["openai", "minimax", "anthropic"] {
        let n = text_secret_name_for_vendor(vendor);
        assert!(
            !n.starts_with(&format!("{vendor}_")),
            "helper must never collapse to bare-vendor naming: {n}"
        );
        assert!(n.starts_with("text_"), "must keep usage prefix: {n}");
    }
}

#[test]
fn helper_handles_empty_vendor_gracefully() {
    // 空 vendor 输入会产出 `text__api_key`，这种字符串能通过 [a-z0-9_]
    // 形态校验（双下划线合法）但语义无效；调用方有责任先确认 vendor 非空。
    // 本测试只锁定"helper 不 panic、输出形态可被进一步 validate"，把
    // "vendor 必须非空"的语义校验留给上层（LlmProviderRuntime::api_key 会
    // 在 strip 失败时直接 fallback 到 config.api_key，根本不调 helper）。
    let n = text_secret_name_for_vendor("");
    assert_eq!(n, "text__api_key");
    assert!(validate_secret_name(&n).is_ok());
}

#[test]
fn helper_round_trips_through_provision_secret_envs() {
    // 端到端：helper 产出的 name 喂给 Capability::Secrets，再走
    // provision_secret_envs 必须能取出对应 SecretValue。
    let n_text = text_secret_name_for_vendor("openai");
    let n_img = image_generation_secret_name_for_vendor("minimax");
    let broker = MockBroker::new(&[(n_text.as_str(), "text-key"), (n_img.as_str(), "img-key")]);
    let caps = vec![
        Capability::Secrets(n_text.clone()),
        Capability::Secrets(n_img.clone()),
    ];
    let out = provision_secret_envs(&broker, &caps).unwrap();
    let names: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["IMAGE_GENERATION_MINIMAX_API_KEY", "TEXT_OPENAI_API_KEY"]
    );
}
