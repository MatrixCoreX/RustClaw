use super::normalize_action;

#[test]
fn normalizes_git_basic_action_aliases() {
    assert_eq!(normalize_action("branches"), "branch");
    assert_eq!(normalize_action("list-branches"), "branch");
    assert_eq!(normalize_action("get_current_branch"), "current_branch");
    assert_eq!(normalize_action("current-branch-name"), "current_branch");
    assert_eq!(normalize_action("cached_diff"), "diff_cached");
    assert_eq!(normalize_action("changed-file-names"), "changed_files");
}
