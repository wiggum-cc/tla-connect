//! ITF trace replay runner (Approach 1).
//!
//! Replays Apalache-generated ITF traces against a Rust `Driver`,
//! comparing state after each step.

use crate::driver::{Driver, State, Step};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use similar::{ChangeTag, TextDiff};
use tracing::{debug, info};

/// Replay multiple ITF traces against a Driver.
///
/// For each trace, for each state transition:
/// 1. Extract `action_taken` and `nondet_picks` from the ITF state
/// 2. Call `driver.step(&step)`
/// 3. Compare spec state with driver state using `State::from_spec`
/// 4. If divergent, print a unified diff and fail
pub fn replay_traces<D: Driver>(
    driver_factory: impl Fn() -> D,
    traces: &[itf::Trace<itf::Value>],
) -> Result<()> {
    info!(trace_count = traces.len(), "Replaying ITF traces");

    for (trace_idx, trace) in traces.iter().enumerate() {
        debug!(
            trace = trace_idx,
            states = trace.states.len(),
            "Replaying trace"
        );

        let mut driver = driver_factory();

        for (state_idx, itf_state) in trace.states.iter().enumerate() {
            let state_value = &itf_state.value;

            // Extract MBT auxiliary variables from the ITF Record
            let (action_taken, nondet_picks) =
                extract_mbt_vars(state_value).with_context(|| {
                    format!("Trace {trace_idx}, state {state_idx}: failed to extract MBT vars")
                })?;

            let step = Step {
                action_taken: action_taken.clone(),
                nondet_picks,
                state: state_value.clone(),
            };

            // Execute the step on the Rust driver
            driver.step(&step).with_context(|| {
                format!(
                    "Trace {trace_idx}, state {state_idx}: failed to execute action '{action_taken}'"
                )
            })?;

            // Compare states using ITF-aware deserialization
            let spec_state = D::State::from_spec(state_value.clone()).with_context(|| {
                format!(
                    "Trace {trace_idx}, state {state_idx}: failed to deserialize spec state"
                )
            })?;

            let driver_state = D::State::from_driver(&driver).with_context(|| {
                format!(
                    "Trace {trace_idx}, state {state_idx}: failed to extract driver state"
                )
            })?;

            if spec_state != driver_state {
                let spec_str = format!("{spec_state:#?}");
                let driver_str = format!("{driver_state:#?}");
                let diff = unified_diff(&spec_str, &driver_str);

                bail!(
                    "State mismatch at trace {trace_idx}, state {state_idx} \
                     (action: '{action_taken}'):\n\n\
                     --- spec (TLA+)\n\
                     +++ driver (Rust)\n\
                     {diff}"
                );
            }
        }

        debug!(trace = trace_idx, "Trace replay successful");
    }

    info!(
        trace_count = traces.len(),
        "All traces replayed successfully"
    );
    Ok(())
}

/// Extract `action_taken` and `nondet_picks` from an ITF state record.
fn extract_mbt_vars(state: &itf::Value) -> Result<(String, itf::Value)> {
    let itf::Value::Record(ref rec) = state else {
        bail!("Expected ITF state to be a Record, got: {state:?}");
    };

    let action_taken = rec
        .get("action_taken")
        .map(|v| String::deserialize(v.clone()))
        .transpose()
        .context("Failed to deserialize action_taken")?
        .unwrap_or_else(|| "init".to_string());

    let nondet_picks = rec
        .get("nondet_picks")
        .cloned()
        .unwrap_or(itf::Value::Tuple(vec![].into()));

    Ok((action_taken, nondet_picks))
}

/// Produce a unified diff between two debug-formatted strings.
fn unified_diff(left: &str, right: &str) -> String {
    let diff = TextDiff::from_lines(left, right);
    let mut output = String::new();

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        output.push_str(sign);
        output.push_str(change.value());
        if !change.value().ends_with('\n') {
            output.push('\n');
        }
    }

    output
}

/// Replay a single ITF trace from a JSON string against a Driver.
///
/// Convenience function for testing with inline trace data.
pub fn replay_trace_str<D: Driver>(
    driver_factory: impl Fn() -> D,
    json: &str,
) -> Result<()> {
    // Parse directly via serde_json to avoid itf::trace_from_str's
    // decode() step, which loses BigInt type info through deserialize_any.
    let trace: itf::Trace<itf::Value> =
        serde_json::from_str(json).context("Failed to parse ITF trace JSON")?;
    replay_traces(driver_factory, &[trace])
}

/// Parse ITF traces from a directory of `.itf.json` files.
pub fn load_traces_from_dir(
    dir: &std::path::Path,
) -> Result<Vec<itf::Trace<itf::Value>>> {
    let mut traces = Vec::new();

    if !dir.is_dir() {
        bail!("Not a directory: {}", dir.display());
    }

    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read directory: {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        if filename.ends_with(".itf.json") {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read: {}", path.display()))?;
            // Parse directly via serde_json to avoid itf::trace_from_str's
            // decode() step, which loses BigInt type info through deserialize_any.
            let trace: itf::Trace<itf::Value> =
                serde_json::from_str(&content)
                    .with_context(|| format!("Failed to parse: {}", path.display()))?;
            traces.push(trace);
        }
    }

    Ok(traces)
}
