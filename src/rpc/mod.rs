//! Apalache JSON-RPC client for interactive symbolic testing (Approach 2).
//!
//! Communicates with a running Apalache explorer server to perform
//! step-by-step symbolic execution of TLA+ specs, interleaved with
//! Rust implementation execution.
//!
//! The explorer API is stateful: `loadSpec` returns a session with transition
//! descriptors. The test loop probes transitions via `assumeTransition`,
//! applies them with `nextStep`, and reads the resulting state via `query`.
//! `rollback` undoes assumptions to explore alternative transitions.

pub mod client;
pub mod types;

pub use client::ApalacheRpcClient;
pub use types::{SpecParameters, TransitionStatus};

use crate::driver::{Driver, State, Step};
use anyhow::{bail, Context, Result};
use rand::prelude::*;
use std::path::Path;
use tracing::{debug, info};

/// Configuration for interactive symbolic testing.
#[derive(Debug, Clone)]
pub struct InteractiveConfig {
    /// Path to the TLA+ spec file (main module).
    pub spec: std::path::PathBuf,

    /// Additional TLA+ files to include (e.g. modules the spec extends).
    /// If empty, all `.tla` files in the spec's directory are included.
    pub aux_files: Vec<std::path::PathBuf>,

    /// Name of the Init predicate (default: "Init").
    pub init: String,

    /// Name of the Next relation (default: "Next").
    pub next: String,

    /// Maximum steps per run.
    pub max_steps: usize,

    /// Number of test runs to execute.
    pub num_runs: usize,

    /// Constants to set via `assumeState` before init.
    /// The Apalache explorer API has no `--cinit` flag, so constants
    /// must be constrained explicitly. Keys are constant names, values
    /// are ITF-encoded (e.g. `{"#bigint": "5"}` for integers).
    pub constants: serde_json::Value,
}

impl Default for InteractiveConfig {
    fn default() -> Self {
        Self {
            spec: std::path::PathBuf::new(),
            aux_files: Vec::new(),
            init: "Init".into(),
            next: "Next".into(),
            max_steps: 100,
            num_runs: 50,
            constants: serde_json::Value::Object(serde_json::Map::new()),
        }
    }
}

/// Collect and base64-encode spec files for loading into Apalache.
///
/// The main spec file comes first. If `aux_files` is empty, all other `.tla`
/// files in the spec's parent directory are included.
fn collect_spec_sources(spec: &Path, aux_files: &[std::path::PathBuf]) -> Result<Vec<String>> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;

    let spec = spec
        .canonicalize()
        .with_context(|| format!("Spec file not found: {}", spec.display()))?;

    let mut sources = Vec::new();

    // Main spec file first
    let main_content = std::fs::read(&spec)
        .with_context(|| format!("Failed to read spec: {}", spec.display()))?;
    sources.push(engine.encode(&main_content));

    if aux_files.is_empty() {
        // Auto-discover: include all .tla files in the same directory
        let spec_dir = spec
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Spec has no parent directory"))?;
        for entry in std::fs::read_dir(spec_dir)
            .with_context(|| format!("Failed to read spec dir: {}", spec_dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("tla") && path != spec {
                let content = std::fs::read(&path)
                    .with_context(|| format!("Failed to read: {}", path.display()))?;
                sources.push(engine.encode(&content));
            }
        }
    } else {
        for aux in aux_files {
            let content = std::fs::read(aux)
                .with_context(|| format!("Failed to read aux file: {}", aux.display()))?;
            sources.push(engine.encode(&content));
        }
    }

    Ok(sources)
}

/// Extract the last state from an ITF trace JSON value.
fn extract_last_state(trace_json: &serde_json::Value) -> Result<serde_json::Value> {
    let states = trace_json
        .get("states")
        .and_then(|s| s.as_array())
        .context("Trace missing 'states' array")?;

    states
        .last()
        .cloned()
        .context("Trace has no states")
}

/// Convert a JSON state (from Apalache query) to an ITF Value for the Driver.
fn json_state_to_itf(state: &serde_json::Value) -> Result<itf::Value> {
    // Parse directly via serde_json to avoid itf::from_value's lossy
    // double-deserialization which converts BigInt to String.
    serde_json::from_value(state.clone()).context("Failed to convert state to ITF Value")
}

/// Extract `action_taken` from a JSON state.
fn extract_action(state: &serde_json::Value) -> String {
    state
        .get("action_taken")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Extract `nondet_picks` from an ITF Value record.
fn extract_nondet(state: &itf::Value) -> itf::Value {
    if let itf::Value::Record(ref rec) = state {
        if let Some(val) = rec.get("nondet_picks") {
            return val.clone();
        }
    }
    itf::Value::Tuple(vec![].into())
}

/// Run interactive symbolic testing (Approach 2).
///
/// For each run:
/// 1. Load spec into the Apalache server
/// 2. Apply an init transition, query the state, execute on driver
/// 3. For each subsequent step:
///    a. Probe all next transitions to find enabled ones
///    b. Pick one randomly
///    c. Apply it, query the new state
///    d. Execute the corresponding action on the driver
///    e. Compare spec state with driver state
/// 4. Dispose the session
pub async fn interactive_test<D: Driver>(
    driver_factory: impl Fn() -> D,
    client: &ApalacheRpcClient,
    config: &InteractiveConfig,
) -> Result<()> {
    let sources = collect_spec_sources(&config.spec, &config.aux_files)
        .context("Failed to collect spec sources")?;

    info!(
        num_runs = config.num_runs,
        max_steps = config.max_steps,
        "Starting interactive symbolic testing"
    );

    let mut rng = rand::rng();

    for run in 0..config.num_runs {
        let mut driver = driver_factory();

        // Load spec (creates a fresh session per run)
        let load_result = client
            .load_spec(sources.clone(), &config.init, &config.next, &[])
            .await
            .context("Failed to load spec")?;

        let session = &load_result.session_id;
        let next_transitions = &load_result.spec_parameters.next_transitions;

        // Constrain constants via assumeState (explorer has no --cinit)
        if config.constants.is_object()
            && !config
                .constants
                .as_object()
                .map_or(true, |m| m.is_empty())
        {
            let result = client
                .assume_state(session, config.constants.clone(), true)
                .await
                .context("Failed to constrain constants via assumeState")?;

            if result.status != TransitionStatus::Enabled {
                bail!("Run {run}: Constant constraints are unsatisfiable");
            }

            debug!(run, "Constants constrained via assumeState");
        }

        // Apply init transition
        let init_idx = load_result
            .spec_parameters
            .init_transitions
            .first()
            .map(|t| t.index)
            .unwrap_or(0);

        let assume_result = client
            .assume_transition(session, init_idx, true)
            .await
            .context("Failed to assume init transition")?;

        if assume_result.status != TransitionStatus::Enabled {
            bail!("Run {run}: Init transition is disabled");
        }

        let step_result = client
            .next_step(session)
            .await
            .context("Failed to apply init step")?;

        let mut current_snapshot = step_result.snapshot_id;

        // Query initial state
        let query = client.query_trace(session).await?;
        let trace = query.trace.context("No trace returned")?;
        let init_state_json = extract_last_state(&trace)?;
        let init_itf = json_state_to_itf(&init_state_json)?;

        // Execute init on driver
        let init_step = Step {
            action_taken: "init".to_string(),
            nondet_picks: itf::Value::Tuple(vec![].into()),
            state: init_itf.clone(),
        };
        driver.step(&init_step).context("Failed to execute init")?;

        // Compare initial state
        compare_states::<D>(&driver, &init_itf, run, 0, "init")?;

        // Step loop
        for step_idx in 1..config.max_steps {
            // Probe all next transitions to find enabled ones
            let mut enabled = Vec::new();

            for t in next_transitions {
                let result = client
                    .assume_transition(session, t.index, true)
                    .await
                    .with_context(|| {
                        format!(
                            "Run {run}, step {step_idx}: assumeTransition({}) failed",
                            t.index
                        )
                    })?;

                if result.status == TransitionStatus::Enabled {
                    enabled.push(t.index);
                }

                // Rollback the assumption regardless (we're just probing)
                client
                    .rollback(session, current_snapshot)
                    .await
                    .with_context(|| {
                        format!(
                            "Run {run}, step {step_idx}: rollback after probe failed"
                        )
                    })?;
            }

            if enabled.is_empty() {
                debug!(run, step = step_idx, "No enabled transitions (deadlock)");
                break;
            }

            // Pick a random enabled transition
            let chosen = *enabled.choose(&mut rng).expect("enabled is non-empty");

            // Apply the chosen transition
            let assume_result = client
                .assume_transition(session, chosen, true)
                .await?;

            if assume_result.status != TransitionStatus::Enabled {
                // Shouldn't happen since we just probed it, but handle gracefully
                client.rollback(session, current_snapshot).await?;
                debug!(
                    run,
                    step = step_idx,
                    transition = chosen,
                    "Transition became disabled between probe and apply"
                );
                break;
            }

            let step_result = client.next_step(session).await.with_context(|| {
                format!("Run {run}, step {step_idx}: nextStep failed")
            })?;

            current_snapshot = step_result.snapshot_id;

            // Query the new state
            let query = client.query_trace(session).await?;
            let trace = query.trace.context("No trace returned")?;
            let state_json = extract_last_state(&trace)?;
            let state_itf = json_state_to_itf(&state_json)?;
            let action_taken = extract_action(&state_json);

            // Execute action on driver
            let step = Step {
                action_taken: action_taken.clone(),
                nondet_picks: extract_nondet(&state_itf),
                state: state_itf.clone(),
            };

            driver.step(&step).with_context(|| {
                format!(
                    "Run {run}, step {step_idx}: failed to execute action '{action_taken}'"
                )
            })?;

            compare_states::<D>(&driver, &state_itf, run, step_idx, &action_taken)?;
        }

        // Cleanup session
        if let Err(e) = client.dispose_spec(session).await {
            debug!(run, error = %e, "Failed to dispose spec (non-fatal)");
        }

        debug!(run, "Run completed successfully");
    }

    info!(
        num_runs = config.num_runs,
        "Interactive symbolic testing completed"
    );
    Ok(())
}

/// Compare spec state with driver state, failing with a diff on mismatch.
fn compare_states<D: Driver>(
    driver: &D,
    spec_itf_state: &itf::Value,
    run: usize,
    step: usize,
    action: &str,
) -> Result<()> {
    let spec_state = D::State::from_spec(spec_itf_state.clone()).with_context(|| {
        format!("Run {run}, step {step}: failed to deserialize spec state")
    })?;

    let driver_state = D::State::from_driver(driver).with_context(|| {
        format!("Run {run}, step {step}: failed to extract driver state")
    })?;

    if spec_state != driver_state {
        bail!(
            "State mismatch at run {run}, step {step} (action: '{action}'):\n\
             spec:   {spec_state:?}\n\
             driver: {driver_state:?}"
        );
    }

    Ok(())
}
