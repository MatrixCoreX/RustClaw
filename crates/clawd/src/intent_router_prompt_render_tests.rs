// Prompt rendering and compaction tests for intent_router.

#[test]
fn normalizer_compact_prompt_is_default_for_agent_loop_convergence() {
    assert!(super::prompt_render::intent_normalizer_compact_prompt_default_enabled());
}

fn empty_normalizer_out_for_retry_test() -> super::IntentNormalizerOut {
    super::IntentNormalizerOut {
        boundary_envelope: None,
        resolved_user_intent: String::new(),
        resume_behavior: String::new(),
        schedule_kind: String::new(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "boundary_only".to_string(),
        confidence: 0.9,
        schedule_intent: None,
        output_contract: None,
        execution_recipe: None,
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    }
}

#[test]
fn normalizer_retry_preserves_base_execution_recipe_machine_field() {
    let mut retry_out = empty_normalizer_out_for_retry_test();
    let mut report = super::ContractRepairReport::default();
    let base = r#"{
        "resolved_user_intent": "create files and run validation",
        "execution_recipe": {"kind": "ops_closed_loop"},
        "output_contract": {"response_shape": "file_token", "delivery_required": true}
    }"#;

    super::prompt_render::preserve_base_execution_recipe_for_retry(
        &mut retry_out,
        base,
        &mut report,
    );

    assert_eq!(
        retry_out
            .execution_recipe
            .as_ref()
            .map(|recipe| recipe.kind.as_str()),
        Some("ops_closed_loop")
    );
    assert!(report
        .detail_csv()
        .contains("preserved_base_execution_recipe"));
    assert!(
        retry_out.output_contract.is_none(),
        "retry recovery should not preserve noisy delivery/output-contract shape"
    );
}

#[test]
fn normalizer_retry_keeps_retry_execution_recipe_when_present() {
    let mut retry_out = empty_normalizer_out_for_retry_test();
    retry_out.execution_recipe = Some(super::IntentExecutionRecipeOut {
        kind: "runtime_async_job".to_string(),
        ..Default::default()
    });
    let mut report = super::ContractRepairReport::default();
    let base = r#"{"execution_recipe":{"kind":"ops_closed_loop"}}"#;

    super::prompt_render::preserve_base_execution_recipe_for_retry(
        &mut retry_out,
        base,
        &mut report,
    );

    assert_eq!(
        retry_out
            .execution_recipe
            .as_ref()
            .map(|recipe| recipe.kind.as_str()),
        Some("runtime_async_job")
    );
    assert!(!report
        .detail_csv()
        .contains("preserved_base_execution_recipe"));
}

#[test]
fn compact_normalizer_prompt_pins_boundary_schema() {
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

    assert!(prompt.contains("Compact boundary normalizer"));
    assert!(prompt.contains("This stage extracts boundaries only"));
    assert!(prompt.contains("agent loop owns ordinary respond / clarify / act"));
    assert!(prompt.contains("Prefer the compact boundary envelope"));
    assert!(prompt.contains("Runtime fills missing compatibility schema slots"));
    assert!(!prompt.contains("resolved_user_intent, answer_candidate"));
    assert!(prompt.contains("Do not emit legacy decision fields"));
    assert!(prompt.contains("Do not emit answer_candidate"));
    assert!(prompt.contains("Boundary extraction scope"));
    assert!(prompt.contains("Do not classify ordinary capability families"));
    assert!(prompt.contains("let the planner/resolver choose from CAPABILITIES"));
    assert!(prompt.contains("If emitted, keep contract_marker=\"none\""));
    assert!(prompt.contains("never create or select feature contract markers"));
    assert!(prompt.contains("If output_contract is emitted, allowed keys only"));
    assert!(
        prompt.contains("Allowed response_shape: free, one_sentence, strict, scalar, file_token")
    );
    assert!(prompt.contains("Every enum field must contain one exact schema token"));
    assert!(prompt.contains("state_patch may carry only machine fields"));
    assert!(prompt.contains("ALIASES: <none>"));
    assert!(prompt.contains("CAPABILITIES:"));
    assert!(prompt.contains("BOUNDARY_ONLY no ordinary capability-family routing"));
    assert!(prompt.contains("REQUEST: list current toml files and briefly explain them"));

    assert!(!prompt.contains("Always include boundary schema keys"));
    assert!(!prompt.contains("capability_ref=weather.current"));
    assert!(!prompt.contains("capability_ref=web.search_results"));
    assert!(!prompt.contains("capability_ref=image_vision"));
    assert!(!prompt.contains("capability_ref=package.detect_manager"));
    assert!(!prompt.contains("directory_purpose_summary"));
    assert!(!prompt.contains("semantic_kind=\"file_names\""));
    assert!(!prompt.contains("semantic_kind=\"directory_names\""));
    assert!(!prompt.contains("semantic_kind=\"service_status\""));
    assert!(!prompt.contains("CONTRACT: output_contract"));
    assert!(!prompt.contains("SCALAR_COUNT_GUARD"));
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

    assert!(compact_head.contains("Do not emit answer_candidate"));
    assert!(prompt.contains("Do not emit answer_candidate"));
    assert!(!prompt.contains("answer_candidate is legacy compatibility"));
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

    assert!(
        prompt.contains("output_contract is an optional compatibility evidence/delivery envelope")
    );
    assert!(compact_tail.contains("LOCAL_EXEC"));
    assert!(compact_tail.contains("let the loop act"));
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
