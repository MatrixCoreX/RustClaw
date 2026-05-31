use super::{
    package_install_packages_from_commandish_text, package_install_packages_from_preview_text,
};

#[test]
fn extracts_packages_from_sudo_apt_get_preview_sentence() {
    assert_eq!(
        package_install_packages_from_commandish_text(
            "実行予定コマンドは sudo -n apt-get install -y ripgrep です。"
        ),
        Some(vec!["ripgrep".to_string()])
    );
}

#[test]
fn extracts_packages_from_brew_preview() {
    assert_eq!(
        package_install_packages_from_commandish_text("command: brew install jq"),
        Some(vec!["jq".to_string()])
    );
}

#[test]
fn ignores_text_without_command_shape() {
    assert_eq!(
        package_install_packages_from_commandish_text("install command for ripgrep"),
        None
    );
}

#[test]
fn preview_text_extracts_single_safe_package_token() {
    assert_eq!(
        package_install_packages_from_preview_text(
            "ripgrep 설치는 하지 말고 dry-run 으로 어떤 명령이 될지만 알려줘."
        ),
        Some(vec!["ripgrep".to_string()])
    );
}

#[test]
fn preview_text_ignores_ambiguous_control_sentence() {
    assert_eq!(
        package_install_packages_from_preview_text("dry-run install for ripgrep"),
        None
    );
}
