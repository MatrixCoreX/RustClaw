use super::*;

#[test]
fn batch_directory_delivery_uses_current_workspace_locator_kind_without_text_reparse() {
    let root = TempDirGuard::new("current_workspace_batch_delivery");
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        delivery_intent: OutputDeliveryIntent::DirectoryBatchFiles,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        ..IntentOutputContract::default()
    };
    assert_eq!(
        resolve_directory_locator_input(&contract, "please do it", root.path()),
        Some(DirectoryLookupInput::ExplicitPath {
            directory_path: root
                .path()
                .canonicalize()
                .expect("canonical root")
                .display()
                .to_string()
        })
    );
}

#[test]
fn batch_directory_delivery_formats_multiline_file_tokens() {
    let root = TempDirGuard::new("batch_tokens");
    let a = root.path().join("a.txt");
    let b = root.path().join("b.txt");
    write_text_file(&a);
    write_text_file(&b);

    let mut files = vec![
        a.canonicalize().expect("canonical a"),
        b.canonicalize().expect("canonical b"),
    ];
    files.sort();
    let text = format_batch_delivery_tokens(&files, None);
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|line| line.starts_with("FILE:")));
}

#[test]
fn batch_directory_delivery_intent_is_distinct_from_single_file_delivery() {
    assert_eq!(
        classify_batch_directory_delivery_input("把这个文件夹下面的文件发我"),
        None
    );
    assert_eq!(
        classify_batch_directory_delivery_input("把 reports 目录下的 daily.md 发给我"),
        None
    );
}

#[test]
fn batch_directory_delivery_only_sends_current_level_and_adds_child_dir_hint() {
    let root = TempDirGuard::new("batch_current_level");
    let dir = root.path().join("output");
    fs::create_dir_all(dir.join("nested")).expect("create nested");
    let current = dir.join("one.txt");
    let nested = dir.join("nested/two.txt");
    write_text_file(&current);
    write_text_file(&nested);

    let listed = list_current_level_files_for_delivery(&dir, 200);
    let CurrentLevelDeliveryEntriesResult::Ready(entries) = listed else {
        panic!("expected ready entries");
    };
    assert!(entries.has_child_dirs);
    assert_eq!(entries.files.len(), 1);
    assert!(entries.files[0]
        .to_string_lossy()
        .ends_with("/output/one.txt"));
    let resolved = build_batch_directory_delivery_response(
        entries,
        "该目录当前层没有可发送的文件",
        "这个目录下还有其他子目录，如需继续发送，请提供更准确路径",
    );
    match resolved {
        BatchDirectoryDeliveryResolution::FileTokens(text) => {
            assert!(text.contains("FILE:"));
            assert!(!text.contains("nested/two.txt"));
            assert!(text.contains("这个目录下还有其他子目录"));
        }
        other => panic!("expected file tokens, got {other:?}"),
    }
}

#[test]
fn batch_directory_delivery_messages_keep_file_tokens_with_child_dir_hint() {
    let mut state = test_state_with_i18n(&[(
        "clawd.msg.directory.child_dirs_hint",
        "This directory also contains subdirectories.",
    )]);
    let root = TempDirGuard::new("batch_message_tokens");
    let dir = root.path().join("output");
    fs::create_dir_all(dir.join("nested")).expect("create nested");
    let file = dir.join("one.txt");
    write_text_file(&file);
    state.skill_rt.workspace_root = root.path().to_path_buf();
    state.skill_rt.default_locator_search_dir = root.path().to_path_buf();

    let contract = IntentOutputContract {
        exact_sentence_count: None,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::DirectoryBatchFiles,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: dir.display().to_string(),
        response_shape: OutputResponseShape::FileToken,
        ..IntentOutputContract::default()
    };

    let (text, messages) = intercept_response_payload_for_delivery(
        &state,
        "send files from output",
        true,
        &contract,
        String::new(),
        Vec::new(),
    );

    assert!(text.starts_with("FILE:"));
    assert!(text.contains("subdirectories"));
    assert_eq!(messages.len(), 1);
    assert!(messages[0].starts_with("FILE:"));
    assert!(messages[0].contains("subdirectories"));
}

#[test]
fn batch_directory_delivery_returns_no_sendable_files_message_when_current_level_has_no_files() {
    let entries = CurrentLevelDeliveryEntries {
        files: Vec::new(),
        has_child_dirs: false,
    };
    let resolved = build_batch_directory_delivery_response(
        entries,
        "该目录当前层没有可发送的文件",
        "这个目录下还有其他子目录，如需继续发送，请提供更准确路径",
    );
    assert_eq!(
        resolved,
        BatchDirectoryDeliveryResolution::UserMessage("该目录当前层没有可发送的文件".to_string())
    );
}

#[test]
fn batch_directory_delivery_no_files_with_child_dirs_appends_hint() {
    let entries = CurrentLevelDeliveryEntries {
        files: Vec::new(),
        has_child_dirs: true,
    };
    let resolved = build_batch_directory_delivery_response(
        entries,
        "该目录当前层没有可发送的文件",
        "这个目录下还有其他子目录，如需继续发送，请提供更准确路径",
    );
    match resolved {
        BatchDirectoryDeliveryResolution::UserMessage(text) => {
            assert!(text.contains("该目录当前层没有可发送的文件"));
            assert!(text.contains("这个目录下还有其他子目录"));
        }
        other => panic!("expected user message, got {other:?}"),
    }
}

#[test]
fn batch_directory_delivery_stops_when_entries_exceed_limit() {
    let root = TempDirGuard::new("batch_too_many");
    let dir = root.path().join("bulk");
    fs::create_dir_all(&dir).expect("create bulk");
    for idx in 0..6 {
        write_text_file(&dir.join(format!("f{idx}.txt")));
    }

    let listed = list_current_level_files_for_delivery(&dir, 3);
    assert_eq!(
        listed,
        CurrentLevelDeliveryEntriesResult::UserMessage(
            DeliveryMessageKind::DirectoryEntriesTooMany
        )
    );
}

#[test]
fn batch_directory_delivery_directory_not_found_does_not_enter_file_tokens() {
    let system_root = TempDirGuard::new("batch_dir_miss_system");
    let project_root = TempDirGuard::new("batch_dir_miss_project");
    let locator = directory_lookup_input_from_hint("missing_dir").expect("batch locator");
    let resolved =
        resolve_directory_target(locator, system_root.path(), project_root.path(), 3, 200);
    assert_eq!(
        resolved,
        DirectoryLookupResolution::UserMessage(DeliveryMessageKind::DirectoryBothRootsMiss)
    );
}
