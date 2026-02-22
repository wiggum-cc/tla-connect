//! Snapshot tests for TLA+ output generation.
//!
//! These tests verify that NDJSON traces are correctly converted to TLA+ modules.

use std::io::Write;
use tla_connect::ndjson_to_tla_module;

fn write_trace(dir: &tempfile::TempDir, filename: &str, lines: &[&str]) -> std::path::PathBuf {
    let path = dir.path().join(filename);
    let mut file = std::fs::File::create(&path).unwrap();
    for line in lines {
        writeln!(file, "{}", line).unwrap();
    }
    path
}

#[test]
fn test_simple_trace_to_tla() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_trace(
        &dir,
        "simple.ndjson",
        &[
            r#"{"action": "init", "counter": 0}"#,
            r#"{"action": "increment", "counter": 1}"#,
            r#"{"action": "increment", "counter": 2}"#,
        ],
    );

    let (tla_module, count) = ndjson_to_tla_module(&trace_path).unwrap();

    assert_eq!(count, 3);
    assert!(tla_module.contains("---- MODULE TraceData ----"));
    assert!(tla_module.contains("EXTENDS Integers, Sequences"));
    assert!(tla_module.contains("TraceLog =="));
    assert!(tla_module.contains("TraceActions =="));
    assert!(tla_module.contains(r#""init""#));
    assert!(tla_module.contains(r#""increment""#));
    assert!(tla_module.contains("action |-> \"init\""));
    assert!(tla_module.contains("counter |-> 0"));
    assert!(tla_module.contains("counter |-> 1"));
    assert!(tla_module.contains("counter |-> 2"));
    assert!(tla_module.contains("===="));
}

#[test]
fn test_nested_objects_to_tla() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_trace(
        &dir,
        "nested.ndjson",
        &[
            r#"{"action": "init", "state": {"x": 0, "y": 0}}"#,
            r#"{"action": "move", "state": {"x": 1, "y": 2}}"#,
        ],
    );

    let (tla_module, count) = ndjson_to_tla_module(&trace_path).unwrap();

    assert_eq!(count, 2);
    assert!(tla_module.contains("state |-> [x |-> 0, y |-> 0]"));
    assert!(tla_module.contains("state |-> [x |-> 1, y |-> 2]"));
}

#[test]
fn test_array_values_to_tla() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_trace(
        &dir,
        "arrays.ndjson",
        &[
            r#"{"action": "init", "items": []}"#,
            r#"{"action": "push", "items": [1, 2, 3]}"#,
        ],
    );

    let (tla_module, count) = ndjson_to_tla_module(&trace_path).unwrap();

    assert_eq!(count, 2);
    assert!(tla_module.contains("items |-> <<>>"));
    assert!(tla_module.contains("items |-> <<1, 2, 3>>"));
}

#[test]
fn test_boolean_values_to_tla() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_trace(
        &dir,
        "bools.ndjson",
        &[r#"{"action": "init", "enabled": true, "ready": false}"#],
    );

    let (tla_module, count) = ndjson_to_tla_module(&trace_path).unwrap();

    assert_eq!(count, 1);
    assert!(tla_module.contains("enabled |-> TRUE"));
    assert!(tla_module.contains("ready |-> FALSE"));
}

#[test]
fn test_string_escaping_to_tla() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_trace(
        &dir,
        "strings.ndjson",
        &[r#"{"action": "log", "message": "hello\nworld\t\"quoted\""}"#],
    );

    let (tla_module, count) = ndjson_to_tla_module(&trace_path).unwrap();

    assert_eq!(count, 1);
    assert!(tla_module.contains(r#"message |-> "hello\nworld\t\"quoted\"""#));
}

#[test]
fn test_large_integers_to_tla() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_trace(
        &dir,
        "bigints.ndjson",
        &[r#"{"action": "init", "big": 9007199254740992}"#],
    );

    let (tla_module, count) = ndjson_to_tla_module(&trace_path).unwrap();

    assert_eq!(count, 1);
    assert!(tla_module.contains("big |-> 9007199254740992"));
}

#[test]
fn test_snowcat_type_annotation() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_trace(
        &dir,
        "typed.ndjson",
        &[r#"{"action": "init", "count": 0, "name": "test", "active": true}"#],
    );

    let (tla_module, _) = ndjson_to_tla_module(&trace_path).unwrap();

    assert!(tla_module.contains("\\* @type: () => Seq("));
    assert!(tla_module.contains("Int"));
    assert!(tla_module.contains("Str"));
    assert!(tla_module.contains("Bool"));
}

#[test]
fn test_emitter_produces_valid_ndjson() {
    use serde::Serialize;
    use tla_connect::StateEmitter;

    #[derive(Serialize)]
    struct GameState {
        score: i64,
        level: i64,
        active: bool,
    }

    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("game.ndjson");

    let mut emitter = StateEmitter::new(&trace_path).unwrap();
    emitter
        .emit("start", &GameState { score: 0, level: 1, active: true })
        .unwrap();
    emitter
        .emit("score", &GameState { score: 100, level: 1, active: true })
        .unwrap();
    emitter
        .emit("levelup", &GameState { score: 100, level: 2, active: true })
        .unwrap();
    emitter
        .emit("end", &GameState { score: 250, level: 2, active: false })
        .unwrap();
    let count = emitter.finish().unwrap();

    assert_eq!(count, 4);

    let (tla_module, tla_count) = ndjson_to_tla_module(&trace_path).unwrap();

    assert_eq!(tla_count, 4);
    assert!(tla_module.contains("---- MODULE TraceData ----"));
    assert!(tla_module.contains("score |-> 0"));
    assert!(tla_module.contains("score |-> 100"));
    assert!(tla_module.contains("score |-> 250"));
    assert!(tla_module.contains("level |-> 1"));
    assert!(tla_module.contains("level |-> 2"));
    assert!(tla_module.contains("active |-> TRUE"));
    assert!(tla_module.contains("active |-> FALSE"));
}

#[test]
fn test_null_values_to_tla() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_trace(
        &dir,
        "nulls.ndjson",
        &[r#"{"action": "init", "value": null}"#],
    );

    let (tla_module, count) = ndjson_to_tla_module(&trace_path).unwrap();

    assert_eq!(count, 1);
    assert!(tla_module.contains(r#"value |-> "null""#));
}

#[test]
fn test_negative_integers_to_tla() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_trace(
        &dir,
        "negative.ndjson",
        &[r#"{"action": "init", "balance": -100}"#],
    );

    let (tla_module, count) = ndjson_to_tla_module(&trace_path).unwrap();

    assert_eq!(count, 1);
    assert!(tla_module.contains("balance |-> -100"));
}

#[test]
fn test_trace_actions_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_trace(
        &dir,
        "actions.ndjson",
        &[
            r#"{"action": "start", "x": 0}"#,
            r#"{"action": "step", "x": 1}"#,
            r#"{"action": "stop", "x": 2}"#,
        ],
    );

    let (tla_module, _) = ndjson_to_tla_module(&trace_path).unwrap();

    assert!(tla_module.contains("TraceActions == <<"));
    assert!(tla_module.contains(r#""start""#));
    assert!(tla_module.contains(r#""step""#));
    assert!(tla_module.contains(r#""stop""#));
}
