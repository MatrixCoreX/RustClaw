use super::*;

#[test]
fn multilingual_requests_preserve_the_same_planner_capability_contract() {
    let state = test_state_with_registry();
    let requests = [
        "列出 logs 目录中的文件名",
        "List file names in the logs directory",
        "logs ディレクトリのファイル名を一覧表示してください",
        "logs 디렉터리의 파일 이름을 나열하세요",
    ];
    let expected = AgentAction::CallCapability {
        capability: "filesystem.list_file_names".to_string(),
        args: json!({"path": "logs", "max_entries": 4}),
    };

    let normalized = requests
        .iter()
        .map(|request| {
            normalize_planned_actions(
                &state,
                None,
                &LoopState::new(1),
                request,
                None,
                vec![expected.clone()],
            )
        })
        .collect::<Vec<_>>();

    assert!(normalized
        .windows(2)
        .all(|pair| { actions_as_json(&pair[0]) == actions_as_json(&pair[1]) }));
    let args = expect_planned_call(&normalized[0][0], "fs_basic", "list_dir");
    assert_eq!(args.get("path").and_then(Value::as_str), Some("logs"));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn multilingual_visible_copy_uses_the_same_explicit_machine_syntax() {
    let requests = [
        "请执行 `pwd` 并返回结果",
        "Run `pwd` and return the result",
        "`pwd` を実行して結果を返してください",
        "`pwd`를 실행하고 결과를 반환하세요",
    ];

    for request in requests {
        assert_eq!(
            crate::agent_engine::explicit_command_segment_for_policy(request).as_deref(),
            Some("pwd")
        );
    }
}
