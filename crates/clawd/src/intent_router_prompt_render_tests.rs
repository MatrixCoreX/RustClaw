// Prompt rendering and compaction tests for intent_router.

#[test]
fn compact_normalizer_prompt_pins_output_contract_schema() {
    let route_view = crate::task_context_builder::RouteContextView {
        request_surface_hints: "locator_target_pair: Cargo.toml | Cargo.lock".to_string(),
        ..Default::default()
    };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true current_process_cwd=/home/guagua/rustclaw",
        "zh-CN",
        "list current toml files and briefly explain them",
    );

    assert!(prompt.contains("Allowed output_contract keys only"));
    assert!(prompt.contains("output_contract as a JSON object, never as a string token"));
    assert!(prompt.contains("Use ALIASES only for temporary references"));
    assert!(prompt.contains("ALIASES: <none>"));
    assert!(prompt.contains("CAPABILITIES:"));
    assert!(
        prompt.contains("Allowed response_shape: free, one_sentence, strict, scalar, file_token")
    );
    assert!(prompt.contains("Allowed semantic_kind: none, raw_command_output"));
    assert!(prompt.contains("document_heading"));
    assert!(prompt.contains("semantic_kind=\"document_heading\""));
    assert!(prompt.contains("semantic_kind=\"hidden_entries_check\""));
    assert!(prompt.contains("semantic_kind=\"existence_with_path\""));
    assert!(prompt.contains("file/path metadata comparisons"));
    assert!(prompt.contains("semantic_kind=\"quantity_comparison\""));
    assert!(prompt.contains("state_patch.quantity_comparison"));
    assert!(prompt.contains("\"source\":\"recent_count_inventory\""));
    assert!(prompt.contains("\"selection\":\"max\"|\"min\""));
    assert!(prompt.contains("semantic_kind=\"execution_failed_step\""));
    assert!(prompt.contains("preserve the whole ordered action sequence"));
    assert!(prompt.contains("do not downgrade failed-action or success/failure-step reporting"));
    assert!(prompt.contains("RECENT_OBSERVED_JUDGMENT"));
    assert!(prompt.contains("do not turn them into fresh file_names/path lookup"));
    assert!(prompt.contains("Text drafting/composition is not file delivery by default"));
    assert!(prompt.contains("Write a long article about RustClaw"));
    assert!(prompt.contains("presence judgment is not numeric counting"));
    assert!(prompt.contains("Do not emit exact_format, required_evidence, fields"));
    assert!(prompt.contains("instead of inventing enum values"));
    assert!(prompt.contains("Every enum field must be exactly one listed schema token"));
    assert!(prompt.contains("clarify is a decision, never a turn_type or resume_behavior"));
    assert!(prompt.contains("state_patch must be a JSON object or null"));
    assert!(prompt.contains("Use decision=\"planner_execute\" when the request inspects"));
    assert!(prompt.contains("generic baseline diagnostics"));
    assert!(prompt.contains("semantic_kind=\"service_status\""));
    assert!(
        prompt.contains("task_control queue/running/cancel status=>planner_execute service_status")
    );
    assert!(
        prompt.contains("Do not use runtime_status_query.kind=\"approval_wait\" for task queue")
    );
    assert!(prompt.contains("current_user, host_name, or kernel_release"));
    assert!(prompt.contains("kb.ingest=>planner_execute filesystem_mutation_result"));
    assert!(prompt.contains("do not classify these as service_status"));
    assert!(
        prompt.contains("Filesystem lifecycle mutation contracts outrank command_output_summary")
    );
    assert!(prompt.contains("inherit only the slice/count constraint"));
    assert!(prompt.contains("Never ask the user to paste local file contents"));
    assert!(prompt.contains("Output exactly one raw JSON object and then stop"));
    assert!(prompt.contains("Normalizer protocol is internal only"));
    assert!(prompt.contains("Inline-data transform invariant"));
    assert!(prompt.contains("Always include all top-level schema keys"));
    assert!(prompt.contains("If ACTIVE_TASK is <none>, do not use task_append"));
    assert!(prompt.contains("turn_type=\"task_append\", target_task_policy=\"reuse_active\""));
    assert!(prompt.contains("never force planner_execute for a presentation-only constraint"));
    assert!(prompt.contains("Current REQUEST overrides RECENT/MEMORY"));
    assert!(prompt.contains("Do not import a prior directory/path scope"));
    assert!(prompt.contains("Fresh unresolved deictic executable targets are missing locators"));
    assert!(prompt.contains("Do not resolve a fresh deictic target from MEMORY alone"));
    assert!(prompt.contains("current_workspace_scope_from_current_request"));
    assert!(prompt.contains("resolved current workspace scope, not a missing locator"));
}

#[test]
fn compact_prompt_slot_preserves_head_and_tail_when_truncated() {
    let value = format!(
        "project background: {}\nvalidation goal: continuous state memory context should remain usable",
        "long middle context ".repeat(80)
    );
    let slot = super::compact_prompt_slot("MEMORY", &value, 180);

    assert!(slot.contains("MEMORY: project background"));
    assert!(slot.contains("...<snip>..."));
    assert!(slot.contains("validation goal:"));
    assert!(slot.contains("state memory context"));
}

#[test]
fn compact_normalizer_prompt_keeps_followup_anchor_next_to_request_tail() {
    let route_view = crate::task_context_builder::RouteContextView {
            active_execution_anchor_context:
                "### ACTIVE_EXECUTION_ANCHOR\nfollowup_source_request: list logs\nfollowup_ordered_entries: 1:act_plan.log | 2:clawd.log | 3:clawd.run.log"
                    .to_string(),
            memory_context: "older document list memory ".repeat(160),
            recent_assistant_replies: "older assistant document list ".repeat(160),
            ..Default::default()
        };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true current_process_cwd=/home/guagua/rustclaw",
        "zh-CN",
        "inspect the second item from the latest list",
    );
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1485);

    assert!(compact_tail.contains("FOLLOWUP_ANCHOR_PRIORITY"));
    assert!(compact_tail.contains("RUNTIME_STATUS"));
    assert!(compact_tail.contains("followup_ordered_entries"));
    assert!(compact_tail.contains("2:clawd.log"));
    assert!(compact_tail.contains("REQUEST: inspect the second item from the latest list"));
}

#[test]
fn compact_normalizer_prompt_keeps_summary_recall_guard_in_head_and_tail() {
    let route_view = crate::task_context_builder::RouteContextView {
        recent_turns_full: "recent turn noise ".repeat(120),
        memory_context: "memory noise ".repeat(120),
        ..Default::default()
    };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let request = "请用一句话总结这个连续会话测试主要验证什么。";
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true current_process_cwd=/home/guagua/rustclaw",
        "zh-CN",
        request,
    );
    let compact_head = crate::providers::utf8_safe_prefix(&prompt, 1485);
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1485);

    assert!(compact_head.contains("High-priority"));
    assert!(compact_head.contains("mainly verifies or means"));
    assert!(compact_tail.contains("SUMMARY_RECALL"));
    assert!(compact_tail.contains(request));
}

#[test]
fn compact_normalizer_prompt_tail_preserves_memory_recall_near_request() {
    let test_id = "client-like-continuous-20260430_134427";
    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("STABLE_FACTS: test number is {test_id}"),
        recent_turns_full: "recent turn noise ".repeat(120),
        last_turn_full: "last turn noise ".repeat(40),
        recent_assistant_replies: "assistant noise ".repeat(20),
        ..Default::default()
    };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let request = "刚才我让你记住的测试编号是什么？只回答编号。";
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true current_process_cwd=/home/guagua/rustclaw",
        "zh-CN",
        request,
    );
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1485);

    assert!(compact_tail.contains(test_id));
    assert!(compact_tail.contains(request));
    assert!(compact_tail.find("MEMORY:").is_some_and(|memory_idx| {
        compact_tail
            .find("REQUEST:")
            .is_some_and(|request_idx| memory_idx < request_idx)
    }));
}

#[test]
fn compact_normalizer_prompt_tail_keeps_assistant_scalar_and_marks_scores_metadata() {
    let test_id = "client-like-continuous-20260430_174102";
    let route_view = crate::task_context_builder::RouteContextView {
            memory_context: "### MEMORY_CONTEXT\n#### RECENT_RELATED_EVENTS\n- 0.55 user asked to remember a long context\n- 0.70 unrelated relevance score".to_string(),
            recent_assistant_replies: format!(
                "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] relative_index=-1 short_preview=已收到 has_code_block=false\n- turn_id=assistant[-2] relative_index=-2 short_preview=已记录。测试编号 `{test_id}` 已记住，后续询问时可直接使用。 has_code_block=false"
            ),
            recent_turns_full: "recent turn noise ".repeat(120),
            last_turn_full: "last turn noise ".repeat(40),
            ..Default::default()
        };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let request = "刚才我让你记住的测试编号是什么？只回答编号。";
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true current_process_cwd=/home/guagua/rustclaw",
        "zh-CN",
        request,
    );
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1815);

    assert!(compact_tail.contains("memory scores are metadata"));
    assert!(compact_tail.contains("ASSISTANT:"));
    assert!(compact_tail.contains(test_id));
    assert!(compact_tail.contains(request));
    assert!(compact_tail.find("MEMORY:").is_some_and(|memory_idx| {
        compact_tail
            .find("ASSISTANT:")
            .is_some_and(|assistant_idx| memory_idx < assistant_idx)
    }));
    assert!(compact_tail
        .find("ASSISTANT:")
        .is_some_and(|assistant_idx| {
            compact_tail
                .find("REQUEST:")
                .is_some_and(|request_idx| assistant_idx < request_idx)
        }));
}

#[test]
fn compact_normalizer_prompt_tail_preserves_long_memory_goal_near_request() {
    let goal = "validation goal: continuous messages should keep recent turns, memory context, and clarification state usable";
    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!(
            "project background: {}\n{goal}",
            "multi-channel agent console context ".repeat(80)
        ),
        recent_turns_full: "recent turn noise ".repeat(120),
        last_turn_full: "last turn noise ".repeat(40),
        recent_assistant_replies: "assistant noise ".repeat(20),
        ..Default::default()
    };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let request = "Please summarize what this continuous conversation test validates.";
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true current_process_cwd=/home/guagua/rustclaw",
        "en",
        request,
    );
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1700);

    assert!(compact_tail.contains("MEMORY:"));
    assert!(compact_tail.contains("validation goal:"));
    assert!(compact_tail.contains("clarification state usable"));
    assert!(compact_tail.contains(request));
}

#[test]
fn compact_normalizer_prompt_tail_preserves_runtime_context_near_request() {
    let route_view = crate::task_context_builder::RouteContextView {
        recent_turns_full: "recent turn noise ".repeat(120),
        last_turn_full: "last turn noise ".repeat(40),
        recent_assistant_replies: "assistant noise ".repeat(20),
        memory_context: "memory noise ".repeat(40),
        ..Default::default()
    };
    let runtime_context = "### RUNTIME_CONTEXT\ncurrent_process_cwd: /tmp/rustclaw-workspace\nworkspace_root: /tmp/rustclaw-workspace";
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: Some(crate::task_context_builder::ExecutionContextView {
            budget_tier: crate::task_context_builder::ExecutionContextBudgetTier::Full,
            memory_ctx: crate::memory::service::PromptMemoryContext {
                prompt_with_memory: String::new(),
                chat_prompt_context: String::new(),
                memory_trace: None,
                long_term_summary: None,
                preferences: Vec::new(),
                recalled: Vec::new(),
                similar_triggers: Vec::new(),
                relevant_facts: Vec::new(),
                recent_related_events: Vec::new(),
            },
            runtime_context: runtime_context.to_string(),
            active_execution_anchor_context: "<none>".to_string(),
            session_alias_context: "<none>".to_string(),
            recent_turns_full: "<none>".to_string(),
            last_turn_full: "<none>".to_string(),
            recent_execution_anchor: "<none>".to_string(),
            recent_execution_context: "<none>".to_string(),
            image_context: None,
        }),
    };
    let request = "只输出当前工作目录的绝对路径，不要解释";
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true",
        "zh-CN",
        request,
    );
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1700);

    assert!(prompt.contains("CONTRACT: output_contract must be a JSON object"));
    assert!(compact_tail.contains("LOCAL_EXEC"));
    assert!(compact_tail.contains("no cannot-access-FS reply"));
    assert!(compact_tail.contains("RUNTIME:"));
    assert!(compact_tail.contains("current_process_cwd: /tmp/rustclaw-workspace"));
    assert!(compact_tail.contains("workspace_root: /tmp/rustclaw-workspace"));
    assert!(compact_tail.contains(request));
    assert!(compact_tail.find("RUNTIME:").is_some_and(|runtime_idx| {
        compact_tail
            .find("REQUEST:")
            .is_some_and(|request_idx| runtime_idx < request_idx)
    }));
}

#[test]
fn compact_normalizer_prompt_falls_back_to_auth_runtime_context() {
    let route_view = crate::task_context_builder::RouteContextView {
        recent_turns_full: "recent turn noise ".repeat(120),
        memory_context: "memory noise ".repeat(40),
        ..Default::default()
    };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let request = "只输出当前工作目录的绝对路径，不要解释";
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "current_auth_role: admin\nallow_path_outside_workspace_for_task: true\nworkspace_root: /home/guagua/rustclaw\ncurrent_process_cwd: /home/guagua/rustclaw",
        "zh-CN",
        request,
    );
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1700);

    assert!(compact_tail.contains("RUNTIME:"));
    assert!(compact_tail.contains("current_process_cwd: /home/guagua/rustclaw"));
    assert!(compact_tail.contains("workspace_root: /home/guagua/rustclaw"));
    assert!(!compact_tail.contains("RUNTIME: <none>"));
    assert!(compact_tail.contains(request));
}
