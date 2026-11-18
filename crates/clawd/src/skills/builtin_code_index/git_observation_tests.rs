use super::*;

#[test]
fn safe_settings_preserve_declarative_boolean_and_line_ending_semantics() {
    let config = safe_core_settings(
        b"core.filemode\nfalse\0core.ignorecase\nYES\0core.autocrlf\ninput\0core.eol\ncrlf\0core.checkstat\nminimal\0core.symlinks\0core.trustctime\n\0",
    ).unwrap();
    assert_eq!(config, "[core]\nfilemode = false\nignorecase = true\nautocrlf = input\neol = crlf\ncheckstat = minimal\nsymlinks = true\ntrustctime = false\n");
}

#[test]
fn safe_settings_reject_command_fields_and_configuration_injection() {
    for invalid in [
        &b"filter.owned.clean\nexit 73\0"[..],
        &b"core.fsmonitor\n/bin/sh\0"[..],
        &b"core.eol\nlf\n[filter owned]\nclean = exit 73\0"[..],
        &b"include.path\n../private/config\0"[..],
        &b"core.symlinks\nnot-a-boolean\0"[..],
    ] {
        assert!(safe_core_settings(invalid).is_err());
    }
    assert!(safe_core_settings(&vec![b'a'; MAX_METADATA_BYTES as usize + 1]).is_err());
}

#[test]
fn directory_pointer_paths_preserve_internal_newlines_and_raw_os_bytes() {
    assert_eq!(
        metadata_path(b"../repo\nname-\xff\n").unwrap(),
        PathBuf::from(OsString::from_vec(b"../repo\nname-\xff".to_vec()))
    );
    assert!(metadata_path(b"\n").is_err());
    assert!(metadata_path(b"invalid\0path\n").is_err());
}

#[test]
fn optional_metadata_reads_are_bounded_regular_files_and_policy_checked() {
    use std::os::unix::fs::DirBuilderExt;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let private = root.join("private");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&private)
        .unwrap();
    std::fs::write(private.join("marker"), b"private-marker").unwrap();
    std::fs::write(root.join("public"), b"12345").unwrap();
    let filesystem = HostFilesystemPolicy::pin_directories(&[private.clone()], root).unwrap();
    assert_eq!(
        read_optional(&filesystem, root, &root.join("public"), 5)
            .unwrap()
            .unwrap(),
        b"12345"
    );
    assert_eq!(
        read_optional(&filesystem, root, &root.join("public"), 4)
            .unwrap_err()
            .code,
        "git_metadata_too_large"
    );
    assert!(read_optional(&filesystem, root, &root.join("missing"), 5)
        .unwrap()
        .is_none());
    assert_eq!(
        read_optional(&filesystem, root, &private.join("marker"), 20)
            .unwrap_err()
            .code,
        "host_private_directory_access_denied"
    );
    std::os::unix::fs::symlink(private.join("marker"), root.join("alias")).unwrap();
    assert_eq!(
        read_optional(&filesystem, root, &root.join("alias"), 20)
            .unwrap_err()
            .code,
        "host_private_directory_access_denied"
    );
}

#[test]
fn metadata_copies_share_a_streaming_byte_budget_and_leave_sources_unchanged() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    for name in ["one", "two", "three"] {
        std::fs::write(source.path().join(name), b"12345").unwrap();
    }
    let filesystem = HostFilesystemPolicy::without_private_authority();
    let mut budget = ObservationBudget {
        metadata_bytes_left: 10,
        repositories_left: 1,
        visited: BTreeSet::new(),
    };
    for name in ["one", "two"] {
        copy_optional(
            &filesystem,
            source.path(),
            name,
            target.path(),
            5,
            &mut budget,
        )
        .unwrap();
        assert_eq!(std::fs::read(target.path().join(name)).unwrap(), b"12345");
    }
    assert_eq!(budget.metadata_bytes_left, 0);
    assert_eq!(
        copy_optional(
            &filesystem,
            source.path(),
            "three",
            target.path(),
            5,
            &mut budget
        )
        .unwrap_err()
        .code,
        "git_metadata_budget_exceeded"
    );
    assert_eq!(
        std::fs::read(source.path().join("three")).unwrap(),
        b"12345"
    );
}

#[test]
fn exhausted_repository_and_depth_budgets_are_explicit_not_empty_complete_results() {
    let root = tempfile::tempdir().unwrap();
    let filesystem = HostFilesystemPolicy::without_private_authority();
    for (repositories, depth) in [(0, 0), (1, MAX_SUBMODULE_DEPTH + 1)] {
        let result = observe_repository(
            root.path(),
            &filesystem,
            ToolSandboxBackend::RemoteContainer,
            depth,
            &mut ObservationBudget {
                metadata_bytes_left: MAX_TOTAL_METADATA_BYTES,
                repositories_left: repositories,
                visited: BTreeSet::new(),
            },
        )
        .unwrap();
        assert_eq!(result.uninspected_submodules, 1);
        assert!(result.paths.is_empty());
    }
}
