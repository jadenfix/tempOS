//! Regression tests for `ModelPolicy` fail-closed data-class defaults.
//!
//! `ModelPolicy` is currently stored on `AgentSession` for the future model
//! router. The router must not inherit an unbounded data-class ceiling just
//! because a session JSON object omitted `model_policy` or provided a partial
//! policy. Only an explicit `null` opts out of the default ceiling.

use beater_os_core::{AgentSession, DataClass, ModelPolicy};

fn session_from_json(input: &str) -> AgentSession {
    serde_json::from_str(input)
        .unwrap_or_else(|err| panic!("agent session fixture should deserialize: {err}"))
}

#[test]
fn model_policy_default_is_bounded_to_internal_data() {
    let policy = ModelPolicy::default();
    assert!(policy.allowed_routes.is_empty());
    assert!(!policy.local_only);
    assert_eq!(policy.max_data_class, Some(DataClass::Internal));
}

#[test]
fn absent_and_partial_model_policy_keep_safe_ceiling() {
    let absent: ModelPolicy = serde_json::from_str("{}")
        .unwrap_or_else(|err| panic!("empty model policy should deserialize: {err}"));
    assert_eq!(absent.max_data_class, Some(DataClass::Internal));

    let partial: ModelPolicy =
        serde_json::from_str(r#"{"allowed_routes":["cloud/opus"],"local_only":false}"#)
            .unwrap_or_else(|err| panic!("partial model policy should deserialize: {err}"));
    assert_eq!(partial.max_data_class, Some(DataClass::Internal));
}

#[test]
fn explicit_null_opts_into_unbounded_data_ceiling() {
    let policy: ModelPolicy = serde_json::from_str(r#"{"max_data_class":null}"#)
        .unwrap_or_else(|err| panic!("explicit null should deserialize: {err}"));
    assert_eq!(policy.max_data_class, None);
}

#[test]
fn agent_session_absent_model_policy_uses_safe_default() {
    let session = session_from_json(
        r#"{
          "session_id": "sess-01",
          "created_at": "2026-07-03T18:00:00Z",
          "created_by": "user:jaden",
          "agent_id": "agent:coder-1",
          "workspace_id": "ws:beateros",
          "goal": "Fix a parser test",
          "policy_profile": "default-coding",
          "journal_root": "0000000000000000000000000000000000000000000000000000000000000000",
          "status": "running"
        }"#,
    );
    assert_eq!(
        session.model_policy.max_data_class,
        Some(DataClass::Internal)
    );
}

#[test]
fn agent_session_partial_model_policy_keeps_safe_ceiling() {
    let session = session_from_json(
        r#"{
          "session_id": "sess-01",
          "created_at": "2026-07-03T18:00:00Z",
          "created_by": "user:jaden",
          "agent_id": "agent:coder-1",
          "workspace_id": "ws:beateros",
          "goal": "Fix a parser test",
          "policy_profile": "default-coding",
          "model_policy": {
            "allowed_routes": ["cloud/opus"]
          },
          "journal_root": "0000000000000000000000000000000000000000000000000000000000000000",
          "status": "running"
        }"#,
    );
    assert_eq!(
        session.model_policy.max_data_class,
        Some(DataClass::Internal)
    );
}
