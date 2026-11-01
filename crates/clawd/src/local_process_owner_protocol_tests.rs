use serde_json::{json, Value};

use super::super::instance::{ProcessBirthIdentity, ProcessStart};
use super::*;

#[test]
fn owner_descriptor_refusal_remains_a_portable_structured_error() {
    let reply = OwnerReply {
        schema_version: 1,
        request_id: 1,
        outcome: OwnerOutcome::Error {
            error_code: ScopeFailure::InvalidDescriptor.into(),
        },
    };
    let bytes = encode_reply(&reply).unwrap();
    assert_eq!(decode_reply(&bytes).unwrap(), reply);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["outcome"]["error_code"], "invalid_descriptor");
    assert_eq!(value["outcome"].as_object().unwrap().len(), 2);
}

fn request_value() -> Value {
    json!({
        "schema_version": 1,
        "credential": {
            "job_id": Uuid::new_v4(),
            "owner": ProcessBirthIdentity {
                pid: 42,
                boot_id: Uuid::new_v4(),
                started: ProcessStart::LinuxTicks { ticks: 7 },
            },
            "nonce": Uuid::new_v4(),
        },
        "request_id": 1,
        "action": "inspect",
    })
}

fn decode(value: &Value) -> Result<OwnerRequest, OwnerFailure> {
    decode_request(&serde_json::to_vec(value).unwrap())
}

#[test]
fn owner_protocol_decodes_exact_versioned_machine_identity() {
    let value = request_value();
    let request = decode(&value).unwrap();
    assert_eq!(request.schema_version, 1);
    assert_eq!(request.request_id, 1);
    assert_eq!(request.action, OwnerAction::Inspect);
    assert_eq!(request.credential.owner.pid, 42);
    assert_eq!(
        request.credential.job_id.to_string(),
        value["credential"]["job_id"].as_str().unwrap()
    );
}

#[test]
fn owner_protocol_rejects_unknown_contracts_and_fields() {
    for version in [0, 2, u32::MAX] {
        let mut value = request_value();
        value["schema_version"] = json!(version);
        assert!(matches!(
            decode(&value),
            Err(OwnerFailure::UnsupportedSchema)
        ));
    }
    for path in [
        "",
        "/credential",
        "/credential/owner",
        "/credential/owner/started",
    ] {
        let mut value = request_value();
        value.pointer_mut(path).unwrap()["unexpected"] = json!(true);
        assert!(matches!(decode(&value), Err(OwnerFailure::InvalidFrame)));
    }
    let duplicate =
        serde_json::to_string(&request_value())
            .unwrap()
            .replacen('{', "{\"request_id\":99,", 1);
    assert!(matches!(
        decode_request(duplicate.as_bytes()),
        Err(OwnerFailure::InvalidFrame)
    ));
}

#[test]
fn owner_protocol_rejects_unbounded_malformed_or_trailing_payloads() {
    for bytes in [
        Vec::new(),
        vec![b' '; MAX_CONTROL_BYTES + 1],
        vec![0xff],
        b"null".to_vec(),
    ] {
        assert!(matches!(
            decode_request(&bytes),
            Err(OwnerFailure::InvalidFrame)
        ));
    }
    let mut trailing = serde_json::to_vec(&request_value()).unwrap();
    trailing.extend_from_slice(b"{}");
    assert!(matches!(
        decode_request(&trailing),
        Err(OwnerFailure::InvalidFrame)
    ));
}

#[test]
fn owner_protocol_rejects_invalid_credentials_before_execution() {
    for (path, invalid) in [
        ("/credential/job_id", json!(Uuid::nil())),
        ("/credential/nonce", json!(Uuid::nil())),
        (
            "/credential/nonce",
            json!("00000000-0000-4000-0000-000000000001"),
        ),
        ("/credential/owner/pid", json!(1)),
        ("/credential/owner/pid", json!(i32::MAX as u32 + 1)),
        ("/credential/owner/boot_id", json!(Uuid::nil())),
    ] {
        let mut value = request_value();
        *value.pointer_mut(path).unwrap() = invalid;
        assert!(matches!(
            decode(&value),
            Err(OwnerFailure::InvalidCredential)
        ));
    }
}

#[test]
fn owner_protocol_actions_do_not_accept_arbitrary_signal_or_process_selectors() {
    for action in ["kill", "spawn", "terminate_group", "", "SIGTERM"] {
        let mut value = request_value();
        value["action"] = json!(action);
        assert!(matches!(decode(&value), Err(OwnerFailure::InvalidFrame)));
    }
    let mut value = request_value();
    value["pid"] = json!(42);
    assert!(matches!(decode(&value), Err(OwnerFailure::InvalidFrame)));
    value.as_object_mut().unwrap().remove("pid");
    value["action"] = json!("cancel");
    assert_eq!(decode(&value).unwrap().action, OwnerAction::Cancel);
    value["request_id"] = json!(0);
    assert!(matches!(decode(&value), Err(OwnerFailure::SequenceGap)));
}

#[test]
fn owner_protocol_accepts_both_platform_birth_contracts() {
    let mut value = request_value();
    value["credential"]["owner"]["started"] = json!({
        "kind": "mac_os_microseconds", "seconds": 7, "microseconds": 999_999,
    });
    assert!(decode(&value).is_ok());
    value["credential"]["owner"]["started"]["microseconds"] = json!(1_000_000);
    assert!(matches!(
        decode(&value),
        Err(OwnerFailure::InvalidCredential)
    ));
}

#[test]
fn owner_protocol_replies_never_echo_credentials_or_process_selectors() {
    fn assert_public_fields(value: &Value) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    assert!(![
                        "credential",
                        "nonce",
                        "owner",
                        "pid",
                        "process_group",
                        "job_id"
                    ]
                    .contains(&key.as_str()));
                    assert_public_fields(value);
                }
            }
            Value::Array(values) => values.iter().for_each(assert_public_fields),
            _ => {}
        }
    }
    let credential = decode(&request_value()).unwrap().credential;
    for outcome in [
        OwnerOutcome::Ok {
            snapshot: OwnerSnapshot {
                phase: OwnerPhase::Cancelling,
                completion: None,
                root_exit: Some(ChildExit {
                    code: Some(0),
                    signal: None,
                }),
            },
        },
        OwnerOutcome::Error {
            error_code: OwnerFailure::OwnershipLost,
        },
    ] {
        let reply = OwnerReply {
            schema_version: 1,
            request_id: 7,
            outcome,
        };
        let value: Value = serde_json::from_slice(&encode_reply(&reply).unwrap()).unwrap();
        assert_eq!(value["request_id"], 7);
        assert_eq!(value.as_object().unwrap().len(), 3);
        assert_public_fields(&value);
        for secret in [
            credential.nonce,
            credential.job_id,
            credential.owner.boot_id,
        ] {
            assert!(!serde_json::to_string(&value)
                .unwrap()
                .contains(&secret.to_string()));
            assert!(!format!("{reply:?}").contains(&secret.to_string()));
        }
    }
}

#[test]
fn owner_protocol_scope_projection_keeps_root_exit_separate_from_scope_completion() {
    let exit = ChildExit {
        code: Some(0),
        signal: None,
    };
    for complete in [false, true] {
        for cancel_requested in [false, true] {
            let projected = OwnerSnapshot::try_from(ScopeSnapshot {
                root_exit: Some(exit),
                complete,
                cancel_requested,
            })
            .unwrap();
            assert_eq!(projected.root_exit, Some(exit));
            assert_eq!(
                projected.phase,
                if complete {
                    OwnerPhase::Terminal
                } else if cancel_requested {
                    OwnerPhase::Cancelling
                } else {
                    OwnerPhase::Running
                }
            );
            assert_eq!(
                projected.completion,
                complete.then_some(if cancel_requested {
                    OwnerCompletion::Cancelled
                } else {
                    OwnerCompletion::Completed
                })
            );
        }
    }
}

#[test]
fn owner_protocol_scope_projection_refuses_missing_or_invalid_exit_proof() {
    assert_eq!(
        OwnerSnapshot::try_from(ScopeSnapshot {
            root_exit: None,
            complete: true,
            cancel_requested: false
        }),
        Err(OwnerFailure::InvalidState)
    );
    for exit in [
        ChildExit {
            code: None,
            signal: None,
        },
        ChildExit {
            code: Some(0),
            signal: Some(libc::SIGTERM),
        },
        ChildExit {
            code: Some(-1),
            signal: None,
        },
        ChildExit {
            code: Some(256),
            signal: None,
        },
        ChildExit {
            code: None,
            signal: Some(0),
        },
    ] {
        for complete in [false, true] {
            assert_eq!(
                OwnerSnapshot::try_from(ScopeSnapshot {
                    root_exit: Some(exit),
                    complete,
                    cancel_requested: false
                }),
                Err(OwnerFailure::InvalidState)
            );
        }
    }
    assert_eq!(
        OwnerSnapshot::try_from(ScopeSnapshot {
            root_exit: None,
            complete: false,
            cancel_requested: false
        })
        .unwrap()
        .phase,
        OwnerPhase::Running
    );
}

#[test]
fn owner_protocol_preserves_each_scope_failure_without_reclassifying_it() {
    for (scope, expected) in [
        (ScopeFailure::Unsupported, OwnerFailure::Unsupported),
        (
            ScopeFailure::OwnerContextInUse,
            OwnerFailure::OwnerContextInUse,
        ),
        (
            ScopeFailure::InvalidSignalDisposition,
            OwnerFailure::InvalidSignalDisposition,
        ),
        (ScopeFailure::SpawnFailed, OwnerFailure::SpawnFailed),
        (
            ScopeFailure::ProbeUnavailable,
            OwnerFailure::ProbeUnavailable,
        ),
        (ScopeFailure::OwnershipLost, OwnerFailure::OwnershipLost),
        (ScopeFailure::SignalFailed, OwnerFailure::SignalFailed),
        (ScopeFailure::LimitExceeded, OwnerFailure::LimitExceeded),
    ] {
        assert_eq!(OwnerFailure::from(scope), expected);
    }
}

#[test]
fn owner_protocol_reply_decoder_refuses_inconsistent_terminal_and_pending_states() {
    for snapshot in [
        OwnerSnapshot {
            phase: OwnerPhase::Terminal,
            completion: Some(OwnerCompletion::Completed),
            root_exit: None,
        },
        OwnerSnapshot {
            phase: OwnerPhase::Running,
            completion: Some(OwnerCompletion::Completed),
            root_exit: Some(ChildExit {
                code: Some(0),
                signal: None,
            }),
        },
    ] {
        let reply = OwnerReply {
            schema_version: 1,
            request_id: 1,
            outcome: OwnerOutcome::Ok { snapshot },
        };
        let bytes = serde_json::to_vec(&reply).unwrap();
        assert_eq!(decode_reply(&bytes), Err(OwnerFailure::InvalidState));
        assert_eq!(encode_reply(&reply), Err(OwnerFailure::InvalidState));
    }
}

#[test]
fn owner_protocol_reply_roundtrip_is_bounded_versioned_and_closed() {
    let reply = OwnerReply {
        schema_version: 1,
        request_id: 3,
        outcome: OwnerOutcome::Ok {
            snapshot: OwnerSnapshot {
                phase: OwnerPhase::Terminal,
                completion: Some(OwnerCompletion::Completed),
                root_exit: Some(ChildExit {
                    code: Some(0),
                    signal: None,
                }),
            },
        },
    };
    assert_eq!(decode_reply(&encode_reply(&reply).unwrap()), Ok(reply));
    for path in [
        vec![],
        vec!["outcome"],
        vec!["outcome", "snapshot"],
        vec!["outcome", "snapshot", "root_exit"],
    ] {
        let mut value = serde_json::to_value(reply).unwrap();
        let mut object = &mut value;
        for field in path {
            object = &mut object[field];
        }
        object["unexpected"] = Value::Bool(true);
        assert_eq!(
            decode_reply(&serde_json::to_vec(&value).unwrap()),
            Err(OwnerFailure::InvalidFrame)
        );
    }
    for bytes in [
        Vec::new(),
        vec![b'x'; MAX_CONTROL_BYTES + 1],
        b"{\"schema_version\":".to_vec(),
    ] {
        assert_eq!(decode_reply(&bytes), Err(OwnerFailure::InvalidFrame));
    }
    let unsupported = OwnerReply {
        schema_version: 2,
        ..reply
    };
    assert_eq!(
        decode_reply(&serde_json::to_vec(&unsupported).unwrap()),
        Err(OwnerFailure::UnsupportedSchema)
    );
    assert_eq!(
        encode_reply(&unsupported),
        Err(OwnerFailure::UnsupportedSchema)
    );
    let invalid_sequence = OwnerReply {
        request_id: 0,
        ..reply
    };
    assert_eq!(
        decode_reply(&serde_json::to_vec(&invalid_sequence).unwrap()),
        Err(OwnerFailure::InvalidFrame)
    );
    assert_eq!(
        encode_reply(&invalid_sequence),
        Err(OwnerFailure::InvalidFrame)
    );
}
