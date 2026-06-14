use super::{
    should_reuse_route_clarify_question, should_suppress_recent_execution_in_clarify_context,
    structured_missing_locator_clarify_context,
};

fn clarify_route(locator_kind: crate::OutputLocatorKind) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "读取那个文件".to_string(),
        needs_clarify: true,
        route_reason: "need concrete locator".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: "你是指哪个文件？".to_string(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            locator_kind,
            requires_content_evidence: true,
            response_shape: crate::OutputResponseShape::Scalar,
            ..Default::default()
        },
    }
}

#[test]
fn scalar_missing_locator_attaches_structured_context() {
    let route = clarify_route(crate::OutputLocatorKind::Path);
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_read_target"));
    assert!(context.contains("locator_kind: path"));
}

#[test]
fn structured_missing_locator_records_explicit_route_question_as_context() {
    let mut route = clarify_route(crate::OutputLocatorKind::Filename);
    route.clarify_question = "LOCATOR_CLARIFY_PROMPT".to_string();
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::DirectoryLookup;
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_directory_locator"));
    assert!(context.contains("normalizer_clarify_question_candidate"));
}

#[test]
fn content_missing_locator_attaches_structured_context() {
    let mut route = clarify_route(crate::OutputLocatorKind::Path);
    route.clarify_question.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_read_target"));
    assert!(context.contains("response_shape: free"));
}

#[test]
fn search_locator_reason_code_attaches_structured_context() {
    let mut route = clarify_route(crate::OutputLocatorKind::Path);
    route.route_reason =
        "path search needs a concrete scope; clarify_reason_code:missing_search_locator"
            .to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_search_locator"));
    assert!(context.contains("semantic_kind: scalar_path_only"));
}

#[test]
fn route_reason_missing_locator_context_survives_generic_locator_hint() {
    let mut route = clarify_route(crate::OutputLocatorKind::None);
    route.resolved_intent = "删除文件".to_string();
    route.route_reason =
        "missing concrete target; clarify_reason_code:missing_read_target".to_string();
    route.output_contract.locator_hint = "file".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilesystemMutationResult;
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_read_target"));
    assert!(context.contains("resolved_user_intent: 删除文件"));
    assert!(context.contains("semantic_kind: filesystem_mutation_result"));
}

#[test]
fn directory_lookup_missing_locator_records_directory_case() {
    let mut route = clarify_route(crate::OutputLocatorKind::Filename);
    route.clarify_question.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::DirectoryLookup;
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_directory_locator"));
}

#[test]
fn scalar_count_missing_locator_records_count_case() {
    let mut route = clarify_route(crate::OutputLocatorKind::Filename);
    route.clarify_question.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_count_target"));
    assert!(context.contains("semantic_kind: scalar_count"));
}

#[test]
fn delivery_missing_locator_attaches_structured_context() {
    let mut route = clarify_route(crate::OutputLocatorKind::Filename);
    route.clarify_question.clear();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.wants_file_delivery = true;
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_file_locator"));
    assert!(context.contains("delivery_required: true"));
}

#[test]
fn unresolved_file_delivery_reason_attaches_missing_file_context_after_contract_clear() {
    let mut route = clarify_route(crate::OutputLocatorKind::None);
    route.clarify_question.clear();
    route.route_reason = "unresolved_file_delivery_requires_clarify".to_string();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route.wants_file_delivery = false;

    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");

    assert!(context.contains("clarify_case: missing_file_locator"));
    assert!(context.contains("delivery_required: false"));
}

#[test]
fn structured_missing_locator_context_skips_fuzzy_candidates() {
    let route = clarify_route(crate::OutputLocatorKind::Filename);
    let candidates = vec!["/tmp/a/Cargo.toml".to_string()];

    assert!(
        structured_missing_locator_clarify_context(&route, &candidates)
            .is_some_and(|context| context.contains("clarify_case: fuzzy_locator_candidates"))
    );
}

#[test]
fn fuzzy_locator_candidates_attach_structured_context() {
    let route = clarify_route(crate::OutputLocatorKind::Filename);
    let candidates = vec![
        "/tmp/a/Cargo.toml".to_string(),
        "/tmp/b/Cargo.toml".to_string(),
    ];
    let context = structured_missing_locator_clarify_context(&route, &candidates)
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: fuzzy_locator_candidates"));
    assert!(context.contains("candidate_1: /tmp/a/Cargo.toml"));
    assert!(context.contains("candidate_2: /tmp/b/Cargo.toml"));
}

#[test]
fn path_scoped_clarify_without_locator_suppresses_recent_execution_context() {
    let route = clarify_route(crate::OutputLocatorKind::Path);
    assert!(should_suppress_recent_execution_in_clarify_context(
        &route,
        &[],
    ));
}

#[test]
fn filename_clarify_without_locator_reuses_specific_router_question() {
    let route = clarify_route(crate::OutputLocatorKind::Filename);
    assert!(should_reuse_route_clarify_question(
        &route,
        crate::post_route_policy::ClarifyReasonKind::MissingPathScopedLocator,
        &[],
    ));
}

#[test]
fn route_reason_text_can_reuse_router_question_when_structured_locator_exists() {
    let mut route = clarify_route(crate::OutputLocatorKind::Filename);
    route.output_contract.locator_hint = "README.md".to_string();
    assert!(should_reuse_route_clarify_question(
        &route,
        crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        &[],
    ));
}

#[test]
fn clarify_with_fuzzy_candidates_keeps_recent_context_available() {
    let route = clarify_route(crate::OutputLocatorKind::Filename);
    assert!(!should_suppress_recent_execution_in_clarify_context(
        &route,
        &["/tmp/a".to_string()],
    ));
}
