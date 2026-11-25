use super::build_incremental_plan_prompt;
use crate::agent_engine::{attempt_ledger::build_attempt_ledger_compact, LoopState};
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use serde_json::json;
use std::path::Path;

#[test]
fn planner_overlays_expand_high_cardinality_placeholders_once() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    let contracts = [
        (
            "single_plan_execution_prompt.md",
            [
                "__GOAL__",
                "__USER_REQUEST__",
                "__TOOL_SPEC__",
                "__SKILL_PLAYBOOKS__",
                "__RECENT_ASSISTANT_REPLIES__",
            ]
            .as_slice(),
        ),
        (
            "loop_incremental_plan_prompt.md",
            [
                "__GOAL__",
                "__USER_REQUEST__",
                "__TOOL_SPEC__",
                "__SKILL_PLAYBOOKS__",
                "__RECENT_ASSISTANT_REPLIES__",
                "__HISTORY_COMPACT__",
                "__ATTEMPT_LEDGER__",
                "__LAST_ROUND_OUTPUT__",
            ]
            .as_slice(),
        ),
    ];

    for (relative_path, placeholders) in contracts {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read prompt overlay");
        for placeholder in placeholders {
            assert_eq!(
                prompt.matches(placeholder).count(),
                1,
                "{relative_path} must expand {placeholder} exactly once"
            );
        }
    }
}

#[test]
fn text_json_planner_prompt_is_built_only_after_native_turn_declines() {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent_engine/planning.rs"),
    )
    .expect("read planner source");
    let native_turn = source
        .find("let native_turn_result = llm_gateway::run_native_model_turn_with_fallback")
        .expect("native model turn branch");
    let fallback_prompt = source
        .find("let (prompt_name, prompt_source, prompt_version, prompt_text) =")
        .expect("text JSON fallback prompt");
    let fallback_request = source
        .find("planner_mode=text_json_fallback")
        .expect("bounded fallback request marker");

    assert!(
        native_turn < fallback_prompt && fallback_prompt < fallback_request,
        "native turns must not render or log the legacy text-JSON planner prompt"
    );
}

#[test]
fn native_action_protocol_requires_capability_owned_structured_observations() {
    let overlay = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../prompts/layers/overlays/native_action_protocol.md");
    let prompt = std::fs::read_to_string(overlay).expect("read native action protocol");
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(prompt.contains("runtime-owned structured results needed"));
    assert!(prompt.contains("call the exact owning `call_capability`"));
    assert!(
        prompt.contains("never derive skill.action names or call unrelated placeholders/probes")
    );
    assert!(prompt.contains("only the loader is a valid next action"));
    assert!(prompt.contains("Runtime-owned parse/validation/inspection/transformation/computation"));
    assert!(normalized.contains("uses its capability"));
    assert!(prompt.contains("Complete supplied input/rules may be transformed directly"));
    assert!(prompt.contains("no runtime validation/evidence/effect is needed"));
    assert!(prompt.contains("An owning validator performs the whole check"));
    assert!(prompt.contains("partial raw reads cannot replace it"));
    assert!(prompt.contains("its structured result requests more evidence"));
    assert!(prompt.contains("Extract known JSON/TOML/YAML fields together"));
    assert!(prompt.contains("with the structured extractor"));
    assert!(prompt.contains("Extract known JSON/TOML/YAML fields together"));
    assert!(prompt.contains("derive counts only from complete observed collections"));
    assert!(prompt.contains("Never reconfirm a successful result"));
    assert!(prompt.contains("Never reconfirm a successful result"));
    assert!(prompt.contains("Playbook pair/compare/cross-check/combine"));
    assert!(prompt.contains("requirements persist across turns"));
    assert!(prompt.contains("smallest evidence set covering ALL required methods/results"));
    assert!(prompt.contains("explicitly removed by the user"));
    assert!(prompt.contains("structured result establishes unavailability"));
    assert!(prompt.contains("structured result establishes unavailability"));
    assert!(prompt.contains("Copy complete capability names"));
    assert!(prompt.contains("never derive skill.action names"));
    assert!(prompt.contains("specific domain contracts"));
    assert!(prompt.contains("over raw primitives"));
    assert!(prompt.contains("Pass user bounds/filter/order as machine args"));
    assert!(prompt.contains("rather than broad fetching/manual trim"));
    assert!(prompt.contains("rather than broad fetching/manual trim"));
    assert!(prompt.contains("FILE:<path>, IMAGE_FILE:<path> or VIDEO_FILE:<path> runtime token"));
    assert!(prompt.contains("do not replace it with guessed channel limitations"));
    assert!(prompt.contains("Listings prove names/metadata, not contents"));
    assert!(prompt.contains("concrete current keys/members/values/scripts/"));
    assert!(prompt.contains("Current project facts need authoritative workspace observations"));
    assert!(prompt.contains("need authoritative workspace observations"));
    assert!(prompt.contains("Current project facts need authoritative workspace observations"));
    assert!(prompt.contains("memory and project names do not suffice"));
    assert!(prompt.contains("Current project facts"));
    assert!(prompt.contains("Explicit fiction/speculation or drafts"));
    assert!(prompt.contains("free_text: content contains prose, compound answers"));
    assert!(normalized.contains("list: ordinal presentation only when allowed"));
    assert!(normalized.contains("ordinal presentation only when allowed"));
    assert!(normalized.contains("Payload-only lines use free_text, items=[] and item count zero"));
    assert!(prompt.contains("unused counts are zero"));
    assert!(prompt.contains("Explicit machine labels and scalar/object/array shapes must survive"));
    assert!(prompt.contains("scalar, object, or array shape"));
    assert!(prompt.contains("agent.subagent"));
    assert!(prompt.contains("agent.subagent_batch"));
    assert!(prompt.contains("agent.subagent_persistent"));
    assert!(prompt.contains("input_materialization=attachment_only"));
    assert!(prompt.contains("structured.visible_text"));
    assert!(normalized.contains("copy all nonempty structured.visible_text in source order"));
    assert!(normalized.contains("Retain only original line markers"));
    assert!(normalized.contains("analysis_text is not transcription"));
    assert!(prompt.contains("Empty visible_text omits that section"));
    assert!(prompt.contains("Typed instructions (raw_chars>0/typed_instruction_present=true)"));
}

#[test]
fn native_action_protocol_requires_structured_session_alias_persistence() {
    let prompt = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../prompts/layers/overlays/native_action_protocol.md"),
    )
    .expect("read native action protocol");

    assert!(prompt.contains("requires `session.bind_alias` before replying"));
    assert!(prompt.contains("Effective request and active work"));
    assert!(prompt.contains("limits the eventual answer, never cancels execution"));
    assert!(prompt.contains("not persist bindings"));
    assert!(prompt
        .contains("An acknowledgement, basename, preference, scalar fact or equal alias/target"));
    assert!(prompt.contains("Respond does not persist bindings"));
    assert!(prompt.contains("Ordinary contextual facts need no mutation"));
}

#[test]
fn native_action_protocol_clarifies_ambiguous_targets_and_keeps_plans_stable() {
    let prompt = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../prompts/layers/overlays/native_action_protocol.md"),
    )
    .expect("read native action protocol");
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(prompt.contains("distinct active targets are equally compatible"));
    assert!(prompt.contains("missing_slot=target_ref"));
    assert!(prompt.contains("Judge meaning across languages, never fixed phrases"));
    assert!(prompt.contains("not discovery, bookkeeping or transport"));
    assert!(prompt.contains("stable IDs, never invent IDs"));
    assert!(
        normalized.contains("Batch ready plan bookkeeping FIRST with the ready material action")
    );
    assert!(normalized.contains("avoiding a bookkeeping-only model turn"));
    assert!(normalized.contains("Respond stays standalone"));
    assert!(prompt.contains(
        "If continuation/completion is also requested, deliver both under continue/amend"
    ));
}

#[test]
fn native_turn_context_rechecks_open_playbook_obligations_at_decision_boundary() {
    let overlay = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../prompts/layers/overlays/native_turn_context.md");
    let prompt = std::fs::read_to_string(overlay).expect("read native turn context");
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let audit = prompt
        .find("Before deciding, audit the selected capability playbooks")
        .expect("playbook obligation audit");
    let decision = prompt
        .find("Decide the next protocol outcome for this turn")
        .expect("decision boundary");

    assert!(audit < decision);
    assert!(prompt.contains("pair, compare, cross-check, or combine evidence"));
    assert!(normalized.contains("every still-relevant source has a current-loop observation"));
    assert!(prompt.contains("user explicitly narrowed the"));
    assert!(prompt.contains("structured observation established"));
    assert!(prompt.contains("`completed_action_result_reused`"));
    assert!(normalized.contains("never propose the same completed action again"));
}

#[test]
fn planner_prompts_do_not_treat_historical_delivery_as_current_execution() {
    let prompt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers");
    let native = std::fs::read_to_string(
        prompt_root
            .join("overlays")
            .join("native_action_protocol.md"),
    )
    .expect("read native action protocol");
    let common = std::fs::read_to_string(prompt_root.join("base/execution/common_rules.md"))
        .expect("read shared execution rules");
    let incremental = std::fs::read_to_string(
        prompt_root
            .join("overlays")
            .join("loop_incremental_plan_prompt.md"),
    )
    .expect("read incremental planner prompt");
    let native = native.split_whitespace().collect::<Vec<_>>().join(" ");
    let incremental = incremental.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(native.contains("Success requires current-loop observations"));
    assert!(native.contains("not history/recent replies/old delivery tokens"));
    assert!(common.contains("not current-task execution evidence"));
    assert!(incremental.contains("scoped to the current task loop"));
    assert!(incremental.contains("do not complete a new executable user turn"));
}

#[test]
fn planner_prompts_do_not_inherit_expanding_modifiers_across_independent_tasks() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for relative_path in [
        "native_action_protocol.md",
        "single_plan_execution_prompt.md",
        "loop_incremental_plan_prompt.md",
    ] {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read planner overlay");
        let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");

        assert!(
            normalized.contains(
                "Optional arguments that expand an operation into additional analysis, conversion, extraction, transformation"
            ),
            "{relative_path} must bind operation-expanding modifiers to the current request"
        );
        assert!(
            normalized.contains(
                "A standalone new locator or share request inherits no prior transformation"
            ),
            "{relative_path} must not inherit transformations across independent tasks"
        );
        assert!(
            normalized.contains("semantically continues or revises that same operation"),
            "{relative_path} must preserve genuine continuations"
        );
        assert!(
            normalized.contains("_runtime_argument_evidence")
                && normalized.contains("exact verbatim quote from the current user request")
                && normalized
                    .contains("runtime verifies and removes this envelope before dispatch"),
            "{relative_path} must explain runtime-verifiable current-request evidence"
        );
    }
}

#[test]
fn planner_common_rules_do_not_probe_workspace_for_capability_credentials() {
    let prompt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers");
    let common = std::fs::read_to_string(prompt_root.join("base/execution/common_rules.md"))
        .expect("read shared execution rules");

    assert!(common.contains("Do not search or read workspace files, environment files"));
    assert!(common.contains("Invoke the matching capability with the requested arguments first"));
    assert!(common.contains("structured missing-credential or missing-dependency result"));
    assert!(common.contains("preserve those names exactly in the visible result"));
    assert!(common.contains("including identity and dimension fields"));
    assert!(common.contains("does not substitute for its field label"));
    assert!(common.contains("maximum item/entity count applies to the complete visible answer"));
}

#[test]
fn planner_overlays_select_subagents_through_capabilities_only() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for relative_path in [
        "single_plan_execution_prompt.md",
        "loop_incremental_plan_prompt.md",
        "plan_repair_prompt.md",
        "agent_tool_spec.md",
    ] {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read planner overlay");
        assert!(
            prompt.contains("agent.subagent"),
            "{relative_path} must expose the registry capability"
        );
        assert!(
            prompt.contains("agent.subagent_batch"),
            "{relative_path} must expose the bounded batch capability"
        );
        assert!(
            !prompt.contains("\"tool\":\"subagent\""),
            "{relative_path} must not revive the retired direct-tool protocol"
        );
    }
}

#[test]
fn planner_tool_spec_is_domain_neutral_and_prefers_owned_capabilities() {
    let overlay = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../prompts/layers/overlays/agent_tool_spec.md");
    let prompt = std::fs::read_to_string(overlay).expect("read agent tool spec");
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(normalized.contains("description and semantic tags own the requested"));
    assert!(normalized.contains("Use raw filesystem, HTTP, database, or shell primitives"));
    assert!(normalized.contains("Do not replace a catalog-owned domain operation"));
    assert!(normalized.contains(
        "Do not call a command or shell capability merely to echo, log, or restate the user request"
    ));
    assert!(normalized.contains("A successful capability result is material evidence"));
    assert!(normalized.contains("Batch only independent read-only actions"));
    for retired_heading in [
        "### image_vision",
        "### config_basic",
        "### photo_organize",
        "Skill behavior notes",
    ] {
        assert!(
            !prompt.contains(retired_heading),
            "global tool protocol must not contain {retired_heading}"
        );
    }
}

#[test]
fn answer_verifier_distinguishes_listing_metadata_from_file_contents() {
    let overlay = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../prompts/layers/overlays/answer_verifier_prompt.md");
    let prompt = std::fs::read_to_string(overlay).expect("read answer verifier prompt");

    assert!(prompt.contains("Listings prove names/metadata"));
    assert!(prompt.contains("not content"));
    assert!(prompt.contains("requested generic type-level purposes may be inferred approximately"));
    assert!(prompt.contains("Current keys/members/values/scripts/schemas need content"));
}

#[test]
fn answer_verifier_does_not_invent_configurable_token_mappings() {
    let overlay = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../prompts/layers/overlays/answer_verifier_prompt.md");
    let prompt = std::fs::read_to_string(overlay).expect("read answer verifier prompt");

    assert!(prompt.contains("Never invent mappings"));
    assert!(prompt.contains("array position"));
    assert!(prompt.contains("successful allowed replacement tokens are evidence"));
    assert!(prompt.contains("do not excuse contradiction of explicit mappings"));
}

#[test]
fn answer_verifier_accepts_grounded_required_input_clarification() {
    let overlay = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../prompts/layers/overlays/answer_verifier_prompt.md");
    let prompt = std::fs::read_to_string(overlay).expect("read answer verifier prompt");

    assert!(prompt.contains("one concise clarification identifying it suffices"));
    assert!(prompt.contains("schema plus one concise clarification identifying it suffices"));
    assert!(prompt.contains("never demand an invalid call"));
    assert!(prompt.contains("pass=true, gaps=[], retry=false"));
}

#[test]
fn planner_and_verifier_stop_at_external_user_action_blockers() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    let native = std::fs::read_to_string(overlays.join("native_action_protocol.md"))
        .expect("read native action protocol");
    let verifier = std::fs::read_to_string(overlays.join("answer_verifier_prompt.md"))
        .expect("read answer verifier prompt");
    let native = native.split_whitespace().collect::<Vec<_>>().join(" ");
    let verifier = verifier.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(native.contains("new external action"));
    assert!(native.contains("terminal closeout for the current attempt"));
    assert!(native.contains("not a side reply"));
    assert!(verifier.contains("not an immediate retry"));
    assert!(verifier.contains("future user input"));
    assert!(verifier.contains("`pass=false` and `should_retry=false`"));
    assert!(verifier.contains("set `blocked=true`"));
}

#[test]
fn drafting_prompts_apply_explicit_replacements_without_unrelated_file_lookup() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for relative_path in [
        "single_plan_execution_prompt.md",
        "loop_incremental_plan_prompt.md",
    ] {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read planner overlay");
        assert!(prompt.contains("Old-to-new corrections"), "{relative_path}");
        assert!(prompt.contains("old value is absent"), "{relative_path}");
        assert!(prompt.contains("alias mutations"), "{relative_path}");
        assert!(
            prompt.contains("unfinished file mutation"),
            "{relative_path}"
        );
    }

    let response = std::fs::read_to_string(overlays.join("chat_response_prompt.md"))
        .expect("read chat response overlay");
    assert!(response.contains("Old-to-new corrections"));
    assert!(response.contains("standalone correction with no active draft"));

    let verifier = std::fs::read_to_string(overlays.join("answer_verifier_prompt.md"))
        .expect("read answer verifier overlay");
    assert!(verifier.contains("Old-to-new corrections"));
    assert!(verifier.contains("asking where it appeared is incomplete"));
    assert!(verifier.contains("Rewrite directly, without invented edits"));
}

#[test]
fn planner_and_verifier_reject_truncated_search_as_exhaustive_inventory() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    let single = std::fs::read_to_string(overlays.join("single_plan_execution_prompt.md"))
        .expect("read single plan prompt");
    let incremental = std::fs::read_to_string(overlays.join("loop_incremental_plan_prompt.md"))
        .expect("read incremental plan prompt");
    let verifier = std::fs::read_to_string(overlays.join("answer_verifier_prompt.md"))
        .expect("read answer verifier prompt");

    assert!(single.contains("complete direct-child inventories"));
    assert!(single.contains("bounded recursive search"));
    assert!(incremental.contains("truncated=true"));
    assert!(incremental.contains("direct listing capability"));
    assert!(verifier.contains("truncated=true"));
    assert!(verifier.contains("nontruncated inventory"));
}

#[test]
fn final_synthesis_prompts_preserve_named_machine_field_shapes() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for relative_path in [
        "observed_answer_fallback_prompt.md",
        "answer_verifier_prompt.md",
    ] {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read synthesis prompt");
        assert!(
            prompt.contains("explicitly names machine fields"),
            "{relative_path} must recognize explicit machine-field delivery"
        );
        assert!(
            prompt.contains("scalar, object, or array shape"),
            "{relative_path} must preserve structured value shapes"
        );
        assert!(
            prompt.contains("nested scalar"),
            "{relative_path} must reject collection flattening"
        );
    }
}

#[test]
fn planner_overlays_require_runtime_observation_for_policy_projections() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for relative_path in [
        "single_plan_execution_prompt.md",
        "loop_incremental_plan_prompt.md",
    ] {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read prompt overlay");
        assert!(
            prompt.contains(
                "Current permission, policy, risk, confirmation, sandbox, or approval projections are runtime observations"
            ),
            "{relative_path} must require a runtime-owned policy projection"
        );
        assert!(
            prompt.contains("never replace it with a guessed `respond`"),
            "{relative_path} must reject planner-invented permission decisions"
        );
        assert!(
            prompt.contains("Prefer a matching dedicated read-only preview/preflight capability"),
            "{relative_path} must prefer the runtime-owned preview contract"
        );
        assert!(
            prompt.contains(
                "Never simulate a no-mutation preview by running a mutating shell command"
            ),
            "{relative_path} must reject side-effecting preview simulation"
        );
    }
}

#[test]
fn planner_overlays_keep_independent_reads_out_of_shell_pipelines() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for relative_path in [
        "single_plan_execution_prompt.md",
        "loop_incremental_plan_prompt.md",
    ] {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read prompt overlay");
        assert!(
            prompt.contains("Independent known targets are not cross-step dependencies"),
            "{relative_path} must distinguish independent reads from dependent execution"
        );
        assert!(
            prompt.contains("Do not collapse independent reads into `run_cmd`"),
            "{relative_path} must keep known read targets on dedicated capabilities"
        );
        assert!(
            prompt.contains(
                "This exception never applies to independent targets whose paths or selectors are already known"
            ),
            "{relative_path} must close the shell dependency exception"
        );
    }
}

#[test]
fn planner_overlays_reserve_scalar_shape_for_one_atomic_result() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for relative_path in [
        "single_plan_execution_prompt.md",
        "loop_incremental_plan_prompt.md",
    ] {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read prompt overlay");
        assert!(
            prompt.contains(
                "`response_shape=\"scalar\"` is valid only when the complete final answer is exactly one atomic value"
            ),
            "{relative_path} must keep scalar projection atomic"
        );
        assert!(
            prompt.contains(
                "compound result and must use `free` or `strict` so every requested deliverable survives final"
            ),
            "{relative_path} must preserve compound final answers"
        );
    }
}

#[test]
fn incremental_prompt_carries_structured_failed_attempt_for_planner_repair() {
    let mut loop_state = LoopState::new();
    let err = crate::skills::structured_skill_error_from_parts(
        "fs_basic",
        "missing_required_field",
        "missing_required_field",
        None,
        Some(json!({
            "error_code": "missing_required_field",
            "missing_evidence_fields": ["path"],
            "message_key": "clawd.skill.missing_required_field"
        })),
    );
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(err),
        started_at: 100,
        finished_at: 110,
    });

    let attempt_ledger = build_attempt_ledger_compact(&loop_state);
    let prompt = build_incremental_plan_prompt(
        "ledger=__ATTEMPT_LEDGER__\nlast=__LAST_ROUND_OUTPUT__\nround=__ROUND__",
        "read project file",
        "read project file",
        "turn_analysis",
        "tool_spec",
        "skill_playbooks",
        "",
        "auto",
        "zh-CN",
        "agent-runtime",
        2,
        "history",
        &attempt_ledger,
        "last round failed",
        "linux",
        "bash",
        "/workspace",
    );

    assert!(prompt.contains(&format!("ledger={attempt_ledger}")));
    let ledger: serde_json::Value = serde_json::from_str(&attempt_ledger).unwrap();
    let record = &ledger[0];
    assert_eq!(record["tool_or_skill"], "fs_basic");
    assert_eq!(record["status"], "error");
    assert_eq!(record["error_code"], "missing_required_field");
    assert_eq!(record["missing_evidence"], json!(["path"]));
    assert_eq!(record["recovery_action"], "collect_missing_evidence");
    assert_eq!(
        record["repair_signal"]["repair_envelope"]["repair_class"],
        "loop_bounded_recovery"
    );
    assert_eq!(
        record["repair_signal"]["repair_envelope"]["next_recovery_kind"],
        "wait_background"
    );
    assert!(record["forbidden_repeat_signature"]
        .as_str()
        .is_some_and(|v| !v.is_empty()));
    assert!(prompt.contains("round=2"));
}
