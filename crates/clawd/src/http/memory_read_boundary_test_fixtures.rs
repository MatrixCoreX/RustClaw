use super::super::memory_read_race_fixtures::Setup;
use claw_core::types::AuthIdentity;
use rusqlite::params;

pub(super) const OWN_CONTENT: &str = "owned content created after authentication changed";
pub(super) const OTHER_CONTENT: &str = "different owner's content must stay private";

pub(super) fn seed_late_state(setup: &Setup, identity: &AuthIdentity) {
    setup.late_private_content();
    setup
        .direct
        .execute(
            "UPDATE memories SET chat_id=?1,content=?2 WHERE memory_id='post-auth-private'",
            params![identity.chat_id, OWN_CONTENT],
        )
        .unwrap();
    for (label, principal, key, content) in [
        (
            "own",
            &setup.fixture.principal,
            &setup.sibling_key,
            OWN_CONTENT,
        ),
        (
            "other",
            &setup.other_principal,
            &setup.other_key,
            OTHER_CONTENT,
        ),
    ] {
        setup
            .direct
            .execute(
                "INSERT INTO user_preferences(
                memory_id,user_id,chat_id,user_key,principal_id,scope_kind,scope_ref,
                pref_key,pref_value,confidence,source,updated_at,updated_at_ts,
                origin,row_revision,legacy_scope_inferred)
             VALUES (?1,1,?2,?3,?4,'principal',?4,'format',?5,1,'user','102',102,
                'user_confirmed',1,0)",
                params![
                    format!("memory_boundary_preference_{label}"),
                    identity.chat_id,
                    key,
                    principal,
                    content
                ],
            )
            .unwrap();
        setup
            .direct
            .execute(
                "INSERT INTO memory_facts(
                memory_id,user_id,chat_id,user_key,principal_id,scope_kind,scope_ref,
                namespace,fact_key,fact_value,fact_text,source_kind,source_ref,
                created_at_ts,updated_at_ts,status,origin,row_revision,legacy_scope_inferred,
                source_refs_json,evidence_refs_json,trust_tier,sensitivity,
                last_verified_at_ts,modified_at_ts)
             VALUES (?1,1,?2,?3,?4,'principal',?4,'user_profile','language',?5,?5,
                'user_message','fixture',102,102,'active','user_confirmed',1,0,
                '[]','[]','user_confirmed','normal',102,102)",
                params![
                    format!("memory_boundary_fact_{label}"),
                    identity.chat_id,
                    key,
                    principal,
                    content
                ],
            )
            .unwrap();
        setup.direct.execute(
            "INSERT INTO memory_runtime_settings(
                setting_key,setting_scope,principal_id,use_mode,generate_mode,
                external_context_policy,revision,policy_digest,updated_at)
             VALUES (?1,'principal',?2,'enabled','disabled','exclude',7,'sha256:late-boundary','102')
             ON CONFLICT(setting_key) DO UPDATE SET use_mode='enabled',generate_mode='disabled',
                external_context_policy='exclude',revision=revision+1,updated_at='102'",
            params![format!("principal:{principal}"), principal],
        ).unwrap();
    }
}

pub(super) fn assert_normal_data(route: &str, data: &serde_json::Value, target: &str) {
    assert!(!data.is_null());
    let rendered = data.to_string();
    assert!(
        !rendered.contains(OTHER_CONTENT),
        "another owner's source leaked: {data}"
    );
    match route {
        "/memory" => assert_eq!(data["counts"]["recent"], 1),
        "/memory/settings" => {
            assert_eq!(data["target_principal_id"], target);
            assert_eq!(data["requested"]["generate_mode"], "disabled");
            assert_eq!(data["external_context_policy"], "exclude");
        }
        "/memory/recent"
        | "/memory/preferences"
        | "/memory/facts"
        | "/memory/items"
        | "/memory/export"
        | "/memory/export/markdown" => {
            assert!(
                rendered.contains(OWN_CONTENT),
                "normal owned source missing: {data}"
            );
        }
        "/memory/clear/preview" => assert_eq!(data["transcript_rows"], 1),
        "/memory/vector/status" => assert_eq!(data["remote_consent"], "exclude"),
        "/memory/remote-disclosure" => assert_eq!(data["consent_state"], "exclude"),
        _ => panic!("unrecognized boundary route"),
    }
}
