//! Tests for trace validation functionality (Approach 3).

use serde::Serialize;
use std::io::Write;
use tla_connect::*;

#[derive(Serialize)]
struct SimpleState {
    counter: i64,
}

#[test]
fn test_emitter_creates_ndjson() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.ndjson");

    let mut emitter = StateEmitter::new(&path).unwrap();
    emitter.emit("init", &SimpleState { counter: 0 }).unwrap();
    emitter
        .emit("increment", &SimpleState { counter: 1 })
        .unwrap();
    let count = emitter.finish().unwrap();

    assert_eq!(count, 2);

    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);

    // Verify JSON structure
    let line1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(line1["action"], "init");
    assert_eq!(line1["counter"], 0);

    let line2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(line2["action"], "increment");
    assert_eq!(line2["counter"], 1);
}

#[test]
fn test_emitter_rejects_non_object() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.ndjson");

    let mut emitter = StateEmitter::new(&path).unwrap();

    // Arrays should be rejected
    let result = emitter.emit("test", &vec![1, 2, 3]);
    assert!(result.is_err());

    // Primitives should be rejected
    let result = emitter.emit("test", &42i64);
    assert!(result.is_err());
}

#[test]
fn test_emitter_handles_special_characters() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.ndjson");

    #[derive(Serialize)]
    struct StateWithString {
        message: String,
    }

    let mut emitter = StateEmitter::new(&path).unwrap();
    emitter
        .emit(
            "test",
            &StateWithString {
                message: "hello\nworld\t\"quoted\"".to_string(),
            },
        )
        .unwrap();
    emitter.finish().unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
    assert_eq!(parsed["message"], "hello\nworld\t\"quoted\"");
}

#[test]
fn test_validation_rejects_empty_trace() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("empty.ndjson");

    std::fs::write(&trace_path, "").unwrap();

    let config = TraceValidatorConfig::builder()
        .trace_spec(dir.path().join("nonexistent.tla"))
        .build();

    let result = validate_trace(&config, &trace_path);
    assert!(result.is_err());
}

#[test]
fn test_validation_rejects_inconsistent_schema() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("inconsistent.ndjson");

    let mut file = std::fs::File::create(&trace_path).unwrap();
    writeln!(file, r#"{{"action": "init", "counter": 0}}"#).unwrap();
    writeln!(file, r#"{{"action": "step", "different_field": 1}}"#).unwrap();

    // We need a dummy trace spec to get past the file existence check
    let spec_path = dir.path().join("Spec.tla");
    std::fs::write(&spec_path, "---- MODULE Spec ----\n====").unwrap();

    let config = TraceValidatorConfig::builder()
        .trace_spec(spec_path)
        .build();

    let result = validate_trace(&config, &trace_path);
    assert!(result.is_err());

    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("Inconsistent") || err_str.contains("schema"),
        "Expected schema error, got: {err_str}"
    );
}

#[test]
fn test_validation_rejects_float_values() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("floats.ndjson");

    let mut file = std::fs::File::create(&trace_path).unwrap();
    writeln!(file, r#"{{"action": "init", "value": 3.14}}"#).unwrap();

    let spec_path = dir.path().join("Spec.tla");
    std::fs::write(&spec_path, "---- MODULE Spec ----\n====").unwrap();

    let config = TraceValidatorConfig::builder()
        .trace_spec(spec_path)
        .build();

    let result = validate_trace(&config, &trace_path);
    assert!(result.is_err());

    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("Float") || err_str.contains("float"),
        "Expected float error, got: {err_str}"
    );
}

#[test]
fn test_validation_handles_nested_objects() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("nested.ndjson");

    let mut file = std::fs::File::create(&trace_path).unwrap();
    writeln!(
        file,
        r#"{{"action": "init", "state": {{"x": 1, "y": 2}}}}"#
    )
    .unwrap();
    writeln!(
        file,
        r#"{{"action": "step", "state": {{"x": 2, "y": 3}}}}"#
    )
    .unwrap();

    let spec_path = dir.path().join("Spec.tla");
    std::fs::write(&spec_path, "---- MODULE Spec ----\n====").unwrap();

    let config = TraceValidatorConfig::builder()
        .trace_spec(spec_path)
        .build();

    // This should fail because apalache isn't available, but it should
    // successfully parse the nested structure first
    let result = validate_trace(&config, &trace_path);
    // The error should be about Apalache, not about parsing
    if let Err(e) = result {
        let err_str = e.to_string();
        assert!(
            err_str.contains("Apalache") || err_str.contains("apalache"),
            "Expected Apalache error (not parse error), got: {err_str}"
        );
    }
}
