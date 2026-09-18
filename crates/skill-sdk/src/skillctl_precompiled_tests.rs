use super::*;

#[test]
fn help_lists_verified_precompiled_installation() {
    let help = run(vec!["help".to_string()]).expect("help");
    assert!(help["commands"].as_array().unwrap().iter().any(|command| {
        command
            .as_str()
            .unwrap_or_default()
            .starts_with("install-precompiled ")
    }));
}

#[test]
fn precompiled_install_requires_all_paths_before_touching_files() {
    let args = [
        "install-precompiled",
        "missing-manifest",
        "missing-workspace",
        "missing-package-root",
    ];
    for length in 1..=args.len() {
        let error = run(args[..length]
            .iter()
            .map(|value| (*value).to_string())
            .collect())
        .expect_err("incomplete request");
        assert_eq!(error.code, "cli_argument_missing");
    }
}
