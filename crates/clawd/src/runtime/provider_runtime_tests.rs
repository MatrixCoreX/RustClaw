use super::*;

#[test]
fn task_snapshot_round_trip_preserves_the_pinned_agent_contract() {
    let config = AgentConfig {
        id: "review-agent".to_string(),
        persona_profile: "reviewer".to_string(),
        preferred_vendor: Some("minimax".to_string()),
        preferred_model: Some("MiniMax-M3".to_string()),
        allowed_skills: vec!["health_check".to_string(), "kb".to_string()],
        ..AgentConfig::default()
    };
    let runtime = AgentRuntimeConfig::from_config(&config, Vec::new());
    let restored = AgentRuntimeConfig::from_task_snapshot(&runtime.task_snapshot_json())
        .expect("valid task snapshot");

    assert_eq!(restored.id, runtime.id);
    assert_eq!(restored.persona_profile, runtime.persona_profile);
    assert_eq!(restored.persona_digest, runtime.persona_digest);
    assert_eq!(restored.runtime_digest, runtime.runtime_digest);
    assert_eq!(restored.preferred_vendor, runtime.preferred_vendor);
    assert_eq!(restored.preferred_model, runtime.preferred_model);
    assert_eq!(restored.allowed_skills, runtime.allowed_skills);
}

#[test]
fn persona_profiles_do_not_change_model_or_skill_execution_projection() {
    let mut baseline = None;
    for profile in [
        "executor",
        "companion",
        "expert",
        "teacher",
        "advisor",
        "reviewer",
    ] {
        let config = AgentConfig {
            id: "main".to_string(),
            persona_profile: profile.to_string(),
            preferred_vendor: Some("minimax".to_string()),
            preferred_model: Some("MiniMax-M3".to_string()),
            allowed_skills: vec!["kb".to_string(), "health_check".to_string()],
            ..AgentConfig::default()
        };
        let runtime = AgentRuntimeConfig::from_config(&config, Vec::new());
        let mut skills = runtime.allowed_skills.iter().cloned().collect::<Vec<_>>();
        skills.sort();
        let projection = (
            runtime.preferred_vendor.clone(),
            runtime.preferred_model.clone(),
            runtime.restrict_skills,
            skills,
        );
        if let Some(expected) = baseline.as_ref() {
            assert_eq!(&projection, expected);
        } else {
            baseline = Some(projection);
        }
    }
}
