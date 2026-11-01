use super::*;
use std::sync::Mutex;

static STRICT_ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

// `apply_skill_runner_env_isolation` reads the process environment directly, so
// these tests use an explicit source map to verify the whitelist deterministically.
#[test]
fn skill_env_strict_is_mandatory_and_cannot_be_disabled_by_environment() {
    let _guard = STRICT_ENV_TEST_LOCK.lock().expect("strict env test lock");
    let prev = claw_core::product_identity::env_os("SKILL_ENV_STRICT");
    std::env::remove_var("APP_SKILL_ENV_STRICT");
    assert!(skill_runner_env_strict_enabled(), "boundary must be ON");

    std::env::set_var("APP_SKILL_ENV_STRICT", "");
    assert!(
        skill_runner_env_strict_enabled(),
        "empty value keeps default ON"
    );

    for val in ["0", "false", "FALSE", "off", "no", "1", "true"] {
        std::env::set_var("APP_SKILL_ENV_STRICT", val);
        assert!(
            skill_runner_env_strict_enabled(),
            "APP_SKILL_ENV_STRICT={val:?} must not weaken isolation"
        );
    }

    match prev {
        Some(v) => std::env::set_var("APP_SKILL_ENV_STRICT", v),
        None => std::env::remove_var("APP_SKILL_ENV_STRICT"),
    }
}

#[test]
fn whitelist_keeps_only_listed_keys_and_drops_secrets_or_unknown() {
    let source = vec![
        ("PATH", "/usr/bin:/bin"),
        ("HOME", "/home/u"),
        ("LANG", "en_US.UTF-8"),
        ("OPENAI_API_KEY", "sk-fake-leak"),
        ("MINIMAX_API_KEY", "sk-fake-leak2"),
        ("MIMO_API_KEY", "sk-fake-leak3"),
        ("XIAOMI_API_KEY", "sk-fake-leak4"),
        ("APP_USER_KEY", "rk-leak"),
        ("DATABASE_URL", "postgres://leak"),
        ("AWS_ACCESS_KEY_ID", "AKIA..."),
    ];
    let kept = collect_whitelisted_env_pairs(source);
    let kept_keys: Vec<&str> = kept.iter().map(|(k, _)| k.to_str().unwrap()).collect();
    assert_eq!(kept_keys, vec!["HOME", "LANG", "PATH"]);
    for (key, _) in &kept {
        assert!(SKILL_RUNNER_ENV_WHITELIST.contains(&key.to_str().unwrap()));
    }
}

#[test]
fn whitelist_drops_empty_value_to_avoid_silent_propagation() {
    let source = vec![("PATH", "/usr/bin"), ("HOME", ""), ("LC_ALL", "C")];
    let kept = collect_whitelisted_env_pairs(source);
    let kept_keys: Vec<&str> = kept.iter().map(|(k, _)| k.to_str().unwrap()).collect();
    assert_eq!(kept_keys, vec!["LC_ALL", "PATH"]);
}

#[test]
fn whitelist_does_not_invent_keys_for_missing_source() {
    let source: Vec<(&str, &str)> = vec![("UNRELATED", "x")];
    let kept = collect_whitelisted_env_pairs(source);
    assert!(kept.is_empty());
}

#[cfg(unix)]
#[test]
fn whitelist_preserves_raw_values_and_deduplicates_without_expanding_allowed_keys() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let raw_home = OsString::from_vec(b"/tmp/home-\xff".to_vec());
    let kept = collect_whitelisted_env_pairs([
        (OsString::from("PATH"), OsString::from("/first")),
        (
            OsString::from_vec(b"PATH\xfe".to_vec()),
            OsString::from("private"),
        ),
        (OsString::from("HOME"), raw_home.clone()),
        (OsString::from("PATH"), OsString::from("/last")),
        (OsString::from("HOME"), OsString::new()),
    ]);
    assert_eq!(
        kept,
        vec![("HOME".into(), raw_home), ("PATH".into(), "/last".into())]
    );
}

#[test]
fn pinned_receipt_environment_passes_only_declared_non_sensitive_values() {
    let declared = vec![
        "WHISPER_BIN".to_string(),
        "WHISPER_MODEL".to_string(),
        "MEDIA_API_KEY".to_string(),
    ];
    let source = vec![
        ("WHISPER_BIN", "/opt/whisper/whisper-cli"),
        ("WHISPER_MODEL", "/opt/whisper/ggml-small.bin"),
        ("MEDIA_API_KEY", "must-not-reach-runner"),
        ("UNDECLARED_SETTING", "must-not-reach-runner"),
    ];

    assert_eq!(
        collect_declared_skill_env_pairs(&declared, source),
        vec![
            (
                "WHISPER_BIN".to_string(),
                std::ffi::OsString::from("/opt/whisper/whisper-cli"),
            ),
            (
                "WHISPER_MODEL".to_string(),
                std::ffi::OsString::from("/opt/whisper/ggml-small.bin"),
            ),
        ]
    );
}

#[test]
fn whitelist_constant_does_not_include_obvious_secrets_or_clawd_specific_keys() {
    let banned = [
        "OPENAI_API_KEY",
        "MINIMAX_API_KEY",
        "MIMO_API_KEY",
        "XIAOMI_API_KEY",
        "QWEN_API_KEY",
        "ANTHROPIC_API_KEY",
        "APP_USER_KEY",
        "APP_ADMIN_KEY",
        "DATABASE_URL",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
    ];
    for needle in banned {
        assert!(!SKILL_RUNNER_ENV_WHITELIST.contains(&needle));
    }
}

#[tokio::test]
async fn run_cmd_does_not_inherit_undeclared_parent_secret() {
    let _guard = STRICT_ENV_TEST_LOCK.lock().expect("strict env test lock");
    let strict_before = claw_core::product_identity::env_os("SKILL_ENV_STRICT");
    let secret_before = claw_core::product_identity::env_os("TEST_PARENT_SECRET");
    std::env::remove_var("APP_SKILL_ENV_STRICT");
    std::env::set_var("APP_TEST_PARENT_SECRET", "must-not-reach-child");

    let output = run_safe_command(
        Path::new("."),
        "printf '%s' \"${APP_TEST_PARENT_SECRET-unset}\"",
        256,
        5,
        5,
        1024,
        false,
    )
    .await
    .expect("bounded run_cmd");
    assert_eq!(output, "unset");

    match strict_before {
        Some(value) => std::env::set_var("APP_SKILL_ENV_STRICT", value),
        None => std::env::remove_var("APP_SKILL_ENV_STRICT"),
    }
    match secret_before {
        Some(value) => std::env::set_var("APP_TEST_PARENT_SECRET", value),
        None => std::env::remove_var("APP_TEST_PARENT_SECRET"),
    }
}

#[cfg(unix)]
#[test]
fn strict_environment_accepts_non_utf8_parent_values_without_panicking() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "skills::tests::env_isolation::raw_parent_environment_child",
            "--ignored",
            "--test-threads=1",
            "--nocapture",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", OsString::from_vec(b"/tmp/raw-home-\xff".to_vec()))
        .env("LC_ALL", "")
        .env("APP_TEST_PARENT_SECRET", "must-not-reach-child")
        .env(OsString::from_vec(b"UNDECLARED_\xfe".to_vec()), "private")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated environment fixture failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "isolated raw-environment subprocess, selected by its parent test"]
async fn raw_parent_environment_child() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let mut command = tokio::process::Command::new("/usr/bin/env");
    let source: Vec<_> = std::env::vars_os().collect();
    assert!(source.len() >= 5, "all injected inputs must be present");
    for pair in [
        (OsString::from("LC_ALL"), OsString::new()),
        (
            OsString::from("APP_TEST_PARENT_SECRET"),
            OsString::from("must-not-reach-child"),
        ),
        (
            OsString::from_vec(b"UNDECLARED_\xfe".to_vec()),
            OsString::from("private"),
        ),
    ] {
        assert!(
            source.contains(&pair),
            "injected exclusion input is missing"
        );
    }
    let report = super::super::apply_skill_runner_env_isolation(&mut command).unwrap();
    assert_eq!(report.preserved, ["HOME", "PATH"]);
    assert_eq!(report.stripped_count, source.len() - 2);
    let declared = collect_declared_skill_env_pairs(
        &["HOME".into(), "APP_TEST_PARENT_SECRET".into()],
        source.clone(),
    );
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].0, "HOME");
    assert_eq!(declared[0].1, std::env::var_os("HOME").unwrap());
    command.env("LC_ALL", "");
    let output = command.output().await.unwrap();
    assert!(output.status.success());
    let actual: std::collections::BTreeSet<_> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    let expected: std::collections::BTreeSet<&[u8]> = [
        b"HOME=/tmp/raw-home-\xff".as_slice(),
        b"PATH=/usr/bin:/bin".as_slice(),
        b"LC_ALL=".as_slice(),
    ]
    .into_iter()
    .collect();
    assert_eq!(actual, expected);
    println!(
        "STRICT_ENV_ACCEPTANCE source_entries={} preserved=2 stripped={} raw_home=true exact_child_environment=true",
        source.len(), report.stripped_count
    );
}
