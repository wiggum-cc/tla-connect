//! End-to-end tests against a real Apalache binary.

use serde::Deserialize;
use std::path::Path;
use tla_connect::*;

#[derive(Debug, PartialEq, Deserialize)]
struct EmptyState {}

impl State for EmptyState {
    fn from_spec(_: &itf::Value) -> Result<Self, DriverError> {
        Ok(Self {})
    }
}

struct NoVarsDriver;

impl ExtractState<NoVarsDriver> for EmptyState {
    fn from_driver(_: &NoVarsDriver) -> Result<Self, DriverError> {
        Ok(EmptyState {})
    }
}

impl Driver for NoVarsDriver {
    type State = EmptyState;

    fn step(&mut self, step: &Step) -> Result<(), DriverError> {
        switch!(step {
            "init" => Ok(()),
            "unknown" => Ok(()),
        })
    }
}

#[test]
fn test_generate_traces_and_replay_with_real_apalache() {
    let config = ApalacheConfig::builder()
        .spec(Path::new("tests/specs/no_vars_replay.tla"))
        .inv("BadInv")
        .mode(ApalacheMode::Check)
        .max_traces(1usize)
        .max_length(2usize)
        .build()
        .unwrap();

    let generated = generate_traces(&config).expect("generate_traces should run Apalache successfully");
    assert!(!generated.traces.is_empty(), "expected at least one ITF trace from Apalache");

    let stats = replay_traces(|| NoVarsDriver, &generated.traces)
        .expect("replay_traces should accept real Apalache ITF output");

    assert!(stats.traces_replayed >= 1);
    assert!(stats.total_states >= 1);
}

#[test]
fn test_validate_trace_with_real_apalache() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("trace.ndjson");

    std::fs::write(&trace_path, "{\"action\":\"init\"}\n{\"action\":\"step\"}\n").unwrap();

    let config = TraceValidatorConfig::builder()
        .trace_spec(Path::new("tests/specs/trace_spec_no_rows.tla"))
        .build()
        .unwrap();

    let result = validate_trace(&config, &trace_path)
        .expect("validate_trace should run Apalache and return a TraceResult");

    match result {
        TraceResult::Valid => {}
        TraceResult::Invalid { reason } => {
            panic!("expected valid trace, got invalid: {reason}");
        }
        _ => {
            panic!("unexpected TraceResult variant");
        }
    }
}
