//! ITF trace replay runner (Approach 1).
//!
//! Replays Apalache-generated ITF traces against a Rust `Driver`,
//! comparing state after each step.
//!
//! # Example
//!
//! ```
//! use tla_connect::replay_trace_str;
//! # use tla_connect::{Driver, State, ExtractState, Step, DriverError, switch};
//! # use serde::Deserialize;
//! #
//! # #[derive(Debug, PartialEq, Deserialize)]
//! # struct S { counter: i64 }
//! # struct D { v: i64 }
//! # impl State for S {}
//! # impl ExtractState<D> for S {
//! #     fn from_driver(d: &D) -> Result<Self, DriverError> { Ok(S { counter: d.v }) }
//! # }
//! # impl Driver for D {
//! #     type State = S;
//! #     fn step(&mut self, step: &Step) -> Result<(), DriverError> {
//! #         switch!(step { "init" => { self.v = 0; Ok(()) }, })
//! #     }
//! # }
//!
//! let trace = r##"{"#meta":{},"vars":["counter"],"states":[{"#meta":{"index":0},"counter":{"#bigint":"0"},"action_taken":"init"}]}"##;
//! replay_trace_str(|| D { v: 0 }, trace).unwrap();
//! ```

use crate::driver::{Driver, ExtractState, State, Step};
use crate::error::{Error, ReplayError, StepContext, StepError};
use serde::Deserialize;
use std::borrow::Borrow;
use std::time::Instant;
use tracing::{debug, info};

/// Statistics from trace replay.
///
/// The `duration` field includes framework overhead (trace parsing, state
/// comparison, progress callbacks) in addition to driver execution time.
#[must_use]
#[derive(Debug, Clone, Default)]
pub struct ReplayStats {
    pub traces_replayed: usize,
    pub total_states: usize,
    pub duration: std::time::Duration,
}

/// Progress callback for replay operations.
pub type ReplayProgressFn = Box<dyn Fn(ReplayProgress) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct ReplayProgress {
    pub trace_index: usize,
    pub total_traces: usize,
    pub state_index: usize,
    pub total_states: usize,
    pub action: String,
}

/// Replay multiple ITF traces against a Driver.
///
/// For each trace, for each state transition:
/// 1. Resolve the action name (from ITF metadata, `action_taken` field, or default)
/// 2. Extract `nondet_picks` from the ITF state
/// 3. Call `driver.step(&step)`
/// 4. Compare spec state with driver state using `State::from_spec`
/// 5. If divergent, print a unified diff and fail
#[must_use = "returns a Result that should be checked for replay failures"]
pub fn replay_traces<'a, D: Driver>(
    driver_factory: impl Fn() -> D,
    traces: impl IntoIterator<Item = &'a itf::Trace<itf::Value>>,
) -> Result<ReplayStats, Error> {
    replay_traces_with_progress(driver_factory, traces, None)
}

/// Replay with progress callback, returns stats.
pub fn replay_traces_with_progress<D: Driver>(
    driver_factory: impl Fn() -> D,
    traces: impl IntoIterator<Item = impl Borrow<itf::Trace<itf::Value>>>,
    progress: Option<ReplayProgressFn>,
) -> Result<ReplayStats, Error> {
    let start = Instant::now();
    let traces: Vec<_> = traces.into_iter().collect();
    let total_traces = traces.len();

    info!(trace_count = total_traces, "Replaying ITF traces");

    let mut stats = ReplayStats::default();

    for (trace_idx, trace) in traces.iter().enumerate() {
        let trace = trace.borrow();

        debug!(
            trace = trace_idx,
            states = trace.states.len(),
            "Replaying trace"
        );

        let mut driver = driver_factory();
        let states = replay_single_trace(
            &mut driver,
            trace,
            trace_idx,
            total_traces,
            &progress,
        )?;

        stats.total_states += states;
        stats.traces_replayed += 1;
        debug!(trace = trace_idx, "Trace replay successful");
    }

    stats.duration = start.elapsed();
    info!(
        trace_count = total_traces,
        "All traces replayed successfully"
    );
    Ok(stats)
}

/// Replay a single ITF trace against a Driver.
///
/// Internal helper used by both sequential and parallel replay.
fn replay_single_trace<D: Driver>(
    driver: &mut D,
    trace: &itf::Trace<itf::Value>,
    trace_idx: usize,
    total_traces: usize,
    progress: &Option<ReplayProgressFn>,
) -> Result<usize, Error> {
    let total_states = trace.states.len();

    for (state_idx, itf_state) in trace.states.iter().enumerate() {
        let state_value = &itf_state.value;

        let (action_taken, nondet_picks) =
            extract_mbt_vars(state_value, &itf_state.meta).map_err(|reason| ReplayError::MbtVarExtraction {
                trace: trace_idx,
                state: state_idx,
                reason,
            })?;

        if let Some(ref cb) = progress {
            cb(ReplayProgress {
                trace_index: trace_idx,
                total_traces,
                state_index: state_idx,
                total_states,
                action: action_taken.clone(),
            });
        }

        let step = Step {
            action_taken: action_taken.clone(),
            nondet_picks,
            state: state_value.clone(),
        };

        let ctx = StepContext::Replay { trace: trace_idx, state: state_idx };

        driver
            .step(&step)
            .map_err(|e| StepError::StepExecution {
                context: ctx.clone(),
                action: action_taken.clone(),
                reason: e.to_string(),
            })?;

        let spec_state =
            D::State::from_spec(state_value).map_err(|e| StepError::SpecDeserialize {
                context: ctx.clone(),
                reason: e.to_string(),
            })?;

        let driver_state =
            <D::State as ExtractState<D>>::from_driver(driver).map_err(|e| StepError::DriverStateExtraction {
                context: ctx.clone(),
                reason: e.to_string(),
            })?;

        if spec_state != driver_state {
            let diff = crate::driver::format_state_mismatch(&spec_state, &driver_state);

            return Err(StepError::StateMismatch {
                context: ctx,
                action: action_taken,
                diff,
            }
            .into());
        }
    }

    Ok(trace.states.len())
}

/// Extract `action_taken` and `nondet_picks` from an ITF state.
///
/// Action resolution priority:
/// 1. ITF state metadata (`#meta`) fields: `"action"`, `"label"`, or `"transition"`
/// 2. Explicit `action_taken` field in the state record
/// 3. Default: `"init"` for state index 0, `"unknown"` otherwise
fn extract_mbt_vars(
    state: &itf::Value,
    meta: &itf::state::Meta,
) -> Result<(String, itf::Value), String> {
    let itf::Value::Record(ref rec) = state else {
        return Err(format!("Expected ITF state to be a Record, got: {state:?}"));
    };

    // Priority 1: check ITF metadata for action label
    let action_from_meta = ["action", "label", "transition"]
        .iter()
        .find_map(|key| meta.other.get(*key).cloned());

    let action_taken = if let Some(action) = action_from_meta {
        action
    } else if let Some(action) = rec.get("action_taken") {
        // Priority 2: explicit action_taken field in state
        String::deserialize(action.clone())
            .map_err(|e| format!("Failed to deserialize action_taken: {e}"))?
    } else {
        // Priority 3: default based on state index
        let is_init = meta.index.map_or(true, |i| i == 0);
        if is_init { "init" } else { "unknown" }.to_string()
    };

    let nondet_picks = rec
        .get("nondet_picks")
        .cloned()
        .unwrap_or(itf::Value::Tuple(vec![].into()));

    Ok((action_taken, nondet_picks))
}

/// Replay a single ITF trace from a JSON string against a Driver.
///
/// Convenience function for testing with inline trace data.
#[must_use = "returns a Result that should be checked for replay failures"]
pub fn replay_trace_str<D: Driver>(driver_factory: impl Fn() -> D, json: &str) -> Result<ReplayStats, Error> {
    let trace: itf::Trace<itf::Value> =
        serde_json::from_str(json).map_err(|e| ReplayError::Parse(e.to_string()))?;
    replay_traces(driver_factory, &[trace])
}

/// Parse ITF traces from a directory of `.itf.json` files.
///
/// Traces are sorted by file path for deterministic ordering.
#[must_use = "returns traces that should be used for replay"]
pub fn load_traces_from_dir(dir: &std::path::Path) -> Result<Vec<itf::Trace<itf::Value>>, Error> {
    if !dir.is_dir() {
        return Err(ReplayError::from(crate::error::DirectoryReadError {
            path: dir.to_path_buf(),
            reason: "Not a directory".to_string(),
        })
        .into());
    }

    let mut entries: Vec<(std::path::PathBuf, itf::Trace<itf::Value>)> = Vec::new();

    for entry in std::fs::read_dir(dir).map_err(|e| ReplayError::from(crate::error::DirectoryReadError {
        path: dir.to_path_buf(),
        reason: e.to_string(),
    }))? {
        let entry = entry.map_err(|e| ReplayError::from(crate::error::DirectoryReadError {
            path: dir.to_path_buf(),
            reason: e.to_string(),
        }))?;
        let path = entry.path();
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        if filename.ends_with(".itf.json") {
            let content = std::fs::read_to_string(&path).map_err(|e| ReplayError::Parse(format!(
                "Failed to read {}: {e}",
                path.display()
            )))?;
            let trace: itf::Trace<itf::Value> = serde_json::from_str(&content)
                .map_err(|e| ReplayError::Parse(format!("Failed to parse {}: {e}", path.display())))?;
            entries.push((path, trace));
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries.into_iter().map(|(_, t)| t).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_at(index: u64) -> itf::state::Meta {
        itf::state::Meta {
            index: Some(index),
            other: std::collections::BTreeMap::new(),
        }
    }

    fn meta_with_action(index: u64, key: &str, value: &str) -> itf::state::Meta {
        let mut other = std::collections::BTreeMap::new();
        other.insert(key.to_string(), value.to_string());
        itf::state::Meta {
            index: Some(index),
            other,
        }
    }

    #[test]
    fn extract_mbt_vars_from_record() {
        let state = itf::Value::Record(
            vec![
                ("action_taken".to_string(), itf::Value::String("increment".into())),
                ("nondet_picks".to_string(), itf::Value::Record(
                    vec![("amount".to_string(), itf::Value::Number(5))].into_iter().collect(),
                )),
                ("counter".to_string(), itf::Value::Number(42)),
            ]
            .into_iter()
            .collect(),
        );

        let (action, nondet) = extract_mbt_vars(&state, &meta_at(1)).unwrap();
        assert_eq!(action, "increment");
        assert!(matches!(nondet, itf::Value::Record(_)));
    }

    #[test]
    fn extract_mbt_vars_defaults_init_at_index_0() {
        let state = itf::Value::Record(
            vec![("counter".to_string(), itf::Value::Number(0))]
                .into_iter()
                .collect(),
        );

        let (action, _) = extract_mbt_vars(&state, &meta_at(0)).unwrap();
        assert_eq!(action, "init");
    }

    #[test]
    fn extract_mbt_vars_defaults_unknown_at_nonzero_index() {
        let state = itf::Value::Record(
            vec![("counter".to_string(), itf::Value::Number(1))]
                .into_iter()
                .collect(),
        );

        let (action, _) = extract_mbt_vars(&state, &meta_at(3)).unwrap();
        assert_eq!(action, "unknown");
    }

    #[test]
    fn extract_mbt_vars_prefers_meta_over_state_field() {
        let state = itf::Value::Record(
            vec![
                ("action_taken".to_string(), itf::Value::String("from_state".into())),
                ("counter".to_string(), itf::Value::Number(1)),
            ]
            .into_iter()
            .collect(),
        );

        let (action, _) = extract_mbt_vars(&state, &meta_with_action(1, "action", "from_meta")).unwrap();
        assert_eq!(action, "from_meta");
    }

    #[test]
    fn extract_mbt_vars_uses_label_key() {
        let state = itf::Value::Record(
            vec![("counter".to_string(), itf::Value::Number(1))]
                .into_iter()
                .collect(),
        );

        let (action, _) = extract_mbt_vars(&state, &meta_with_action(1, "label", "increment")).unwrap();
        assert_eq!(action, "increment");
    }

    #[test]
    fn extract_mbt_vars_uses_transition_key() {
        let state = itf::Value::Record(
            vec![("counter".to_string(), itf::Value::Number(1))]
                .into_iter()
                .collect(),
        );

        let (action, _) = extract_mbt_vars(&state, &meta_with_action(1, "transition", "decrement")).unwrap();
        assert_eq!(action, "decrement");
    }

    #[test]
    fn extract_mbt_vars_rejects_non_record() {
        let state = itf::Value::Number(42);
        assert!(extract_mbt_vars(&state, &meta_at(0)).is_err());
    }

    #[test]
    fn unified_diff_identical() {
        let result = crate::driver::unified_diff("hello\nworld\n", "hello\nworld\n");
        assert!(!result.contains('+'));
        assert!(!result.contains('-'));
    }

    #[test]
    fn unified_diff_different() {
        let result = crate::driver::unified_diff("hello\n", "world\n");
        assert!(result.contains("-hello"));
        assert!(result.contains("+world"));
    }
}

/// Replay traces in parallel using rayon.
///
/// Each trace is replayed independently in its own thread.
/// Returns on first error encountered.
#[cfg(feature = "parallel")]
pub fn replay_traces_parallel<D: Driver + Send>(
    driver_factory: impl Fn() -> D + Sync,
    traces: &[itf::Trace<itf::Value>],
) -> Result<ReplayStats, Error> {
    use rayon::prelude::*;

    let start = std::time::Instant::now();
    let total_traces = traces.len();

    let results: Result<Vec<(usize, usize)>, Error> = traces
        .par_iter()
        .enumerate()
        .map(|(trace_idx, trace)| {
            let mut driver = driver_factory();
            let states = replay_single_trace(&mut driver, trace, trace_idx, total_traces, &None)?;
            Ok((1, states))
        })
        .collect();

    let stats_vec = results?;
    let (traces_replayed, total_states) = stats_vec
        .iter()
        .fold((0, 0), |acc, x| (acc.0 + x.0, acc.1 + x.1));

    Ok(ReplayStats {
        traces_replayed,
        total_states,
        duration: start.elapsed(),
    })
}
