use crate::agent_engine::LoopState;

pub(super) fn readbacks_for_local_code_projection(
    loop_state: &LoopState,
    current_write_paths: &[String],
) -> Vec<super::FsReadback> {
    if current_write_paths.is_empty() {
        return super::successful_code_readbacks(loop_state);
    }
    let mut readbacks =
        super::successful_fs_readbacks_after_latest_writes(loop_state, current_write_paths);
    for readback in
        supplemental_source_readbacks_for_unwritten_sources(loop_state, current_write_paths)
    {
        if !readbacks
            .iter()
            .any(|existing| super::projection_paths_match(&existing.path, &readback.path))
        {
            readbacks.push(readback);
        }
    }
    readbacks
}

fn supplemental_source_readbacks_for_unwritten_sources(
    loop_state: &LoopState,
    current_write_paths: &[String],
) -> Vec<super::FsReadback> {
    let project_dir = super::common_parent_path(current_write_paths);
    super::successful_code_readbacks(loop_state)
        .into_iter()
        .filter(|readback| !super::path_looks_like_test_file(&readback.path))
        .filter(|readback| {
            !current_write_paths
                .iter()
                .any(|write_path| super::projection_paths_match(&readback.path, write_path))
        })
        .filter(|readback| {
            project_dir
                .as_deref()
                .is_none_or(|dir| projection_path_is_under_dir(&readback.path, dir))
        })
        .collect()
}

fn projection_path_is_under_dir(path: &str, dir: &str) -> bool {
    let path = super::normalize_projection_path(path);
    let dir = super::normalize_projection_path(dir)
        .trim_end_matches('/')
        .to_string();
    path == dir || path.starts_with(&format!("{dir}/"))
}
