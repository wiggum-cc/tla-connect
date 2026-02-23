//! Tests for ITF trace replay functionality.

use serde::Deserialize;
use tla_connect::*;

#[derive(Debug, PartialEq, Deserialize)]
struct TestState {
    counter: i64,
}

impl State for TestState {}

impl ExtractState<TestDriver> for TestState {
    fn from_driver(driver: &TestDriver) -> Result<Self, DriverError> {
        Ok(TestState {
            counter: driver.value,
        })
    }
}

struct TestDriver {
    value: i64,
}

impl Default for TestDriver {
    fn default() -> Self {
        Self { value: 0 }
    }
}

impl Driver for TestDriver {
    type State = TestState;

    fn step(&mut self, step: &Step) -> Result<(), DriverError> {
        switch!(step {
            "init" => {
                self.value = 0;
                Ok(())
            },
            "increment" => {
                self.value += 1;
                Ok(())
            },
            "decrement" => {
                self.value -= 1;
                Ok(())
            },
        })
    }
}

#[test]
fn test_replay_simple_trace() {
    let trace_json = r###"{
        "#meta": {"format": "ITF", "format-description": "ITF trace"},
        "vars": ["counter", "action_taken"],
        "states": [
            {"#meta": {"index": 0}, "counter": {"#bigint": "0"}, "action_taken": "init"},
            {"#meta": {"index": 1}, "counter": {"#bigint": "1"}, "action_taken": "increment"},
            {"#meta": {"index": 2}, "counter": {"#bigint": "2"}, "action_taken": "increment"},
            {"#meta": {"index": 3}, "counter": {"#bigint": "1"}, "action_taken": "decrement"}
        ]
    }"###;

    let result = replay_trace_str(TestDriver::default, trace_json);
    assert!(result.is_ok(), "Replay failed: {:?}", result.err());
}

#[test]
fn test_replay_state_mismatch() {
    let trace_json = r###"{
        "#meta": {"format": "ITF"},
        "vars": ["counter", "action_taken"],
        "states": [
            {"#meta": {"index": 0}, "counter": {"#bigint": "0"}, "action_taken": "init"},
            {"#meta": {"index": 1}, "counter": {"#bigint": "5"}, "action_taken": "increment"}
        ]
    }"###;

    let result = replay_trace_str(TestDriver::default, trace_json);
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("State mismatch"),
        "Expected state mismatch error, got: {err_str}"
    );
}

#[test]
fn test_replay_unknown_action() {
    let trace_json = r###"{
        "#meta": {"format": "ITF"},
        "vars": ["counter", "action_taken"],
        "states": [
            {"#meta": {"index": 0}, "counter": {"#bigint": "0"}, "action_taken": "init"},
            {"#meta": {"index": 1}, "counter": {"#bigint": "0"}, "action_taken": "unknown_action"}
        ]
    }"###;

    let result = replay_trace_str(TestDriver::default, trace_json);
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("Unknown action") || err_str.contains("unknown_action"),
        "Expected unknown action error, got: {err_str}"
    );
}

#[test]
fn test_replay_empty_trace() {
    let trace_json = r###"{
        "#meta": {"format": "ITF"},
        "vars": ["counter", "action_taken"],
        "states": []
    }"###;

    let result = replay_trace_str(TestDriver::default, trace_json);
    assert!(result.is_ok());
}

#[test]
fn test_replay_with_nondet_picks() {
    #[derive(Debug, PartialEq, Deserialize)]
    struct StateWithValue {
        counter: i64,
    }

    impl State for StateWithValue {}

    impl ExtractState<DriverWithNondet> for StateWithValue {
        fn from_driver(driver: &DriverWithNondet) -> Result<Self, DriverError> {
            Ok(StateWithValue {
                counter: driver.value,
            })
        }
    }

    struct DriverWithNondet {
        value: i64,
    }

    impl Driver for DriverWithNondet {
        type State = StateWithValue;

        fn step(&mut self, step: &Step) -> Result<(), DriverError> {
            switch!(step {
                "init" => {
                    self.value = 0;
                    Ok(())
                },
                "add" => {
                    if let itf::Value::Record(ref rec) = step.nondet_picks {
                        if let Some(itf::Value::BigInt(amount)) = rec.get("amount") {
                            self.value += amount.to_string().parse::<i64>().unwrap_or(0);
                        }
                    }
                    Ok(())
                },
            })
        }
    }

    let trace_json = r###"{
        "#meta": {"format": "ITF"},
        "vars": ["counter", "action_taken", "nondet_picks"],
        "states": [
            {"#meta": {"index": 0}, "counter": {"#bigint": "0"}, "action_taken": "init", "nondet_picks": {}},
            {"#meta": {"index": 1}, "counter": {"#bigint": "5"}, "action_taken": "add", "nondet_picks": {"amount": {"#bigint": "5"}}}
        ]
    }"###;

    let result = replay_trace_str(|| DriverWithNondet { value: 0 }, trace_json);
    assert!(result.is_ok(), "Replay failed: {:?}", result.err());
}
