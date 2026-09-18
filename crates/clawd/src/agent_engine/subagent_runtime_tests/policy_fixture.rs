pub(super) fn install(state: &crate::AppState) {
    crate::agent_engine::skill_execution::tests::install_test_registry(
        state,
        r#"
[[skills]]
name = "fs_basic"
enabled = true
kind = "builtin"
planner_kind = "tool"
planner_capability_aliases = { "filesystem.read_file" = "filesystem.read_text_range" }
planner_capabilities = [
  { name = "filesystem.read_text_range", action = "read_text_range", effect = "observe", isolation_profile = "read_only", network_access = false, filesystem_write = false, subprocess = false },
  { name = "filesystem.read_file", action = "read_text_range", effect = "observe", isolation_profile = "read_only", network_access = false, filesystem_write = false, subprocess = false },
  { name = "filesystem.write_text", action = "write_text", effect = "mutate", isolation_profile = "local_current_workspace", filesystem_write = true },
]
[[skills]]
name = "media_download"
enabled = false
kind = "runner"
planner_kind = "skill"
planner_capabilities = [
  { name = "media_download.download", action = "download", effect = "mutate", isolation_profile = "local_current_workspace", network_access = true, filesystem_write = true, subprocess = true },
]
"#,
        &["fs_basic", "media_download"],
    );
}
