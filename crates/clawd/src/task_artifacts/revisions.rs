use super::*;

/// Honor only explicit replacements attached to a verified host-published
/// snapshot. A missing/tampered replacement cannot suppress a valid old file.
pub(super) fn select_current_revisions(
    workspace: &Path,
    task_id: &str,
    result: &Value,
    sources: &mut Vec<ArtifactSource>,
) {
    let mut suppressed = HashSet::new();
    let prefix = format!("artifact:task/{task_id}/");
    for pointer in [
        "/task_journal/trace/capability_results",
        "/final_result_json/task_journal/trace/capability_results",
    ] {
        let Some(results) = result.pointer(pointer).and_then(Value::as_array) else {
            continue;
        };
        for capability in results {
            if capability.get("status").and_then(Value::as_str) != Some("ok") {
                continue;
            }
            for pointer in ["/artifacts", "/data/extra/artifacts"] {
                let Some(artifacts) = capability.pointer(pointer).and_then(Value::as_array) else {
                    continue;
                };
                for artifact in artifacts {
                    let Some(id) = verified_revision_id(workspace, task_id, artifact) else {
                        continue;
                    };
                    // A later explicit publication of the same content is current again.
                    suppressed.remove(&id);
                    if let Some(digest) = artifact.get("sha256").and_then(Value::as_str) {
                        suppressed.remove(&format!("a_{}", &digest[..32]));
                    }
                    if let Some(refs) = artifact
                        .pointer("/metadata/replaces_artifact_refs")
                        .and_then(Value::as_array)
                    {
                        for reference in refs {
                            if let Some(old_id) = reference
                                .as_str()
                                .and_then(|reference| reference.strip_prefix(&prefix))
                            {
                                if old_id != id {
                                    suppressed.insert(old_id.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    sources.retain(|source| {
        !source.id.as_ref().is_some_and(|id| suppressed.contains(id))
            && !source
                .sha256
                .as_ref()
                .filter(|digest| digest.len() == 64)
                .is_some_and(|digest| suppressed.contains(&format!("a_{}", &digest[..32])))
    });
}

fn verified_revision_id(workspace: &Path, task_id: &str, artifact: &Value) -> Option<String> {
    if artifact.get("owner_task_id")?.as_str()? != task_id
        || artifact.pointer("/metadata/producer")?.as_str()? != "filesystem_text_delivery"
        || artifact.get("visibility")?.as_str()? != "user_delivery"
    {
        return None;
    }
    let digest = artifact.get("sha256")?.as_str()?;
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let expected_id = format!("a_{}", &digest[..32]);
    if artifact.get("id")?.as_str()? != expected_id {
        return None;
    }
    let source = validated_source_path(workspace, artifact.get("path")?.as_str()?)?;
    let root = workspace
        .join(claw_core::workspace_state::WORKSPACE_STATE_DIR_NAME)
        .join("artifacts")
        .join("generated-file")
        .join(machine_path_component(task_id, "task"));
    if source.parent()? != root || sha256_file(&source).ok()?.as_str() != digest {
        return None;
    }
    Some(expected_id)
}
