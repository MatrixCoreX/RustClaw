use super::*;

fn domain_metadata_config() -> PhotoOrganizeConfig {
    PhotoOrganizeConfig {
        photo_child_dir_hints: Some(vec![
            "DCIM".to_string(),
            "Photos".to_string(),
            "Pictures".to_string(),
            "Camera".to_string(),
            "照片".to_string(),
            "相机".to_string(),
        ]),
        camera_brand_aliases: Some(vec![
            CameraBrandAliasConfig {
                canonical: "Canon".to_string(),
                aliases: vec!["canon".to_string(), "佳能".to_string()],
            },
            CameraBrandAliasConfig {
                canonical: "Fujifilm".to_string(),
                aliases: vec![
                    "fujifilm".to_string(),
                    "fuji".to_string(),
                    "富士".to_string(),
                ],
            },
        ]),
        ..PhotoOrganizeConfig::default()
    }
}

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "execution_failed");
    assert_eq!(
        extra["message_key"],
        "skill.photo_organize.execution_failed"
    );
    assert_eq!(extra["retryable"], false);
}

#[test]
fn mountinfo_discovery_keeps_real_media_mounts() {
    let raw = "\
36 24 8:1 / / rw,relatime - ext4 /dev/root rw\n\
50 24 0:20 / /media rw,relatime - tmpfs tmpfs rw\n\
51 24 0:21 / /mnt rw,relatime - tmpfs tmpfs rw\n\
52 24 8:17 / /media/guagua/CAMERA\\040CARD rw,nosuid,nodev,relatime - vfat /dev/sdb1 rw\n\
53 24 8:33 / /mnt/photo-disk rw,relatime - exfat /dev/sdc1 rw\n\
54 24 0:22 / /run/media rw,relatime - tmpfs tmpfs rw\n";
    let roots = linux_external_roots_from_mountinfo(raw)
        .into_iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        roots,
        vec![
            "/media/guagua/CAMERA CARD".to_string(),
            "/mnt/photo-disk".to_string(),
        ]
    );
}

#[test]
fn media_style_discovery_handles_raspberry_pi_user_mounts() {
    let base = std::env::temp_dir().join(format!(
        "rustclaw-photo-organize-test-{}",
        std::process::id()
    ));
    let media = base.join("media");
    let pi_camera = media.join("pi").join("CAMERA_CARD");
    let direct_usb = media.join("usb0");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(pi_camera.join("DCIM")).unwrap();
    fs::create_dir_all(direct_usb.join("DCIM")).unwrap();

    let cfg = domain_metadata_config();
    let roots = discover_media_style_roots(media.to_str().unwrap(), &cfg)
        .into_iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();

    let pi_camera_text = pi_camera.display().to_string();
    let pi_container_text = media.join("pi").display().to_string();
    let direct_usb_text = direct_usb.display().to_string();
    let pi_camera_pos = roots
        .iter()
        .position(|path| path == &pi_camera_text)
        .expect("expected /media/pi/<disk> style root");
    if let Some(pi_container_pos) = roots.iter().position(|path| path == &pi_container_text) {
        assert!(pi_camera_pos < pi_container_pos);
    }
    assert!(roots.iter().any(|path| path == &direct_usb_text));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn auto_source_only_selects_unique_external_root() {
    assert_eq!(
        preferred_auto_source_root(vec![PathBuf::from("/media/pi/CAMERA")]),
        Some(PathBuf::from("/media/pi/CAMERA"))
    );
    assert_eq!(
        preferred_auto_source_root(vec![
            PathBuf::from("/media/pi/CAMERA"),
            PathBuf::from("/mnt/photo-disk")
        ]),
        None
    );
    assert_eq!(preferred_auto_source_root(Vec::new()), None);
}

#[test]
fn structured_action_aliases_map_to_default_modes() {
    assert_eq!(
        default_mode_for_action_alias("plan"),
        Some(OrganizeMode::Plan)
    );
    assert_eq!(
        default_mode_for_action_alias("preview"),
        Some(OrganizeMode::Plan)
    );
    assert_eq!(
        default_mode_for_action_alias("copy"),
        Some(OrganizeMode::Copy)
    );
    assert_eq!(
        default_mode_for_action_alias("move"),
        Some(OrganizeMode::Move)
    );
    assert_eq!(default_mode_for_action_alias("organize"), None);
}

#[test]
fn structured_group_by_accepts_year_and_date_fields() {
    let fields = parse_group_by_value(Some(&json!(["year", "date", "model", "year"])))
        .expect("expected parsed fields");
    assert_eq!(
        fields
            .iter()
            .map(|field| field.as_arg_str())
            .collect::<Vec<_>>(),
        vec!["year", "date", "model"]
    );
}

#[test]
fn camera_brand_aliases_are_canonicalized_before_matching() {
    let cfg = domain_metadata_config();
    assert_eq!(
        canonical_brand_name("佳能", &PhotoOrganizeConfig::default()),
        Some("佳能".to_string())
    );
    assert_eq!(
        canonical_brand_name("佳能", &cfg),
        Some("Canon".to_string())
    );
    assert_eq!(
        canonical_brand_name("FUJI", &cfg),
        Some("Fujifilm".to_string())
    );
    assert!(brand_matches("Canon Inc.", &["佳能".to_string()], &cfg));
    assert!(brand_matches(
        "FUJIFILM Corporation",
        &["富士".to_string()],
        &cfg
    ));
    assert!(!brand_matches("Nikon", &["Sony".to_string()], &cfg));
}

#[test]
fn structured_date_filters_normalize_common_machine_shapes() {
    assert_eq!(normalize_capture_year("2026-04-03"), "2026");
    assert_eq!(normalize_capture_month("202604"), "2026-04");
    assert_eq!(normalize_capture_month("2026/4"), "2026-04");
    assert_eq!(normalize_capture_date("20260403"), "2026-04-03");
    assert_eq!(normalize_capture_date("2026/4/3"), "2026-04-03");
}

#[test]
fn selector_list_preserves_multi_word_camera_models() {
    assert_eq!(
        parse_selector_list(Some(&json!("EOS R6, α7 IV"))),
        vec!["EOS R6".to_string(), "α7 IV".to_string()]
    );
    assert!(text_matches_any(
        Some("Canon EOS R6 Mark II"),
        &["eos r6".to_string()]
    ));
    assert!(!text_matches_any(
        Some("Canon EOS R5"),
        &["eos r6".to_string()]
    ));
}

#[test]
fn capture_date_parts_include_year_month_and_day() {
    assert_eq!(
        parse_capture_date_parts("2026:04:03 12:34:56"),
        (
            Some("2026".to_string()),
            Some("2026-04".to_string()),
            Some("2026-04-03".to_string())
        )
    );
}
