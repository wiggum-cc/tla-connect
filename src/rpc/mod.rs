//! Apalache JSON-RPC client for interactive symbolic testing (Approach 2).
//!
//! Communicates with a running Apalache explorer server to perform
//! step-by-step symbolic execution of TLA+ specs, interleaved with
//! Rust implementation execution.

pub mod client;
pub mod types;

pub use client::{ApalacheRpcClient, RetryConfig};
pub use types::{SpecParameters, TransitionStatus};

use crate::driver::{Driver, ExtractState, State, Step};
use crate::error::{Error, RpcError, StepContext, StepError};
use rand::prelude::*;
use rand::SeedableRng;
use std::path::Path;
use std::time::Instant;
use tracing::{debug, info};

/// Statistics from interactive testing.
///
/// The `duration` field includes framework overhead (RPC communication, state
/// comparison, progress callbacks) in addition to driver execution time.
#[must_use]
#[derive(Debug, Clone, Default)]
pub struct InteractiveStats {
    pub runs_completed: usize,
    pub total_steps: usize,
    pub deadlocks_hit: usize,
    pub duration: std::time::Duration,
}

/// Progress callback for interactive testing.
pub type InteractiveProgressFn = Box<dyn Fn(InteractiveProgress) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct InteractiveProgress {
    pub run_index: usize,
    pub total_runs: usize,
    pub step_index: usize,
    pub action: String,
}

/// Configuration for interactive symbolic testing.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
    pub constants: serde_json::Value,

    /// Random seed for reproducible test runs.
    /// If None, uses entropy from the system.
    pub seed: Option<u64>,
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
            seed: None,
        }
    }
}

crate::builder::impl_builder!(InteractiveConfig, InteractiveConfigBuilder {
    required { spec: std::path::PathBuf }
    optional { aux_files: Vec<std::path::PathBuf>, init: String, next: String,
               max_steps: usize, num_runs: usize, constants: serde_json::Value }
    optional_or { seed: u64 }
});

fn collect_spec_sources(spec: &Path, aux_files: &[std::path::PathBuf]) -> Result<Vec<String>, Error> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;

    let spec = spec
        .canonicalize()
        .map_err(|_| RpcError::SpecNotFound(spec.to_path_buf()))?;

    let mut sources = Vec::new();

    let main_content = std::fs::read(&spec).map_err(|e| RpcError::SpecRead {
        path: spec.clone(),
        reason: e.to_string(),
    })?;
    sources.push(engine.encode(&main_content));

    if aux_files.is_empty() {
        let spec_dir = spec
            .parent()
            .ok_or_else(|| RpcError::SpecNotFound(spec.clone()))?;
        for entry in std::fs::read_dir(spec_dir).map_err(|e| RpcError::SpecRead {
            path: spec_dir.to_path_buf(),
            reason: e.to_string(),
        })? {
            let entry = entry.map_err(|e| RpcError::SpecRead {
                path: spec_dir.to_path_buf(),
                reason: e.to_string(),
            })?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("tla") && path != spec {
                let content = std::fs::read(&path).map_err(|e| RpcError::SpecRead {
                    path: path.clone(),
                    reason: e.to_string(),
                })?;
                sources.push(engine.encode(&content));
            }
        }
    } else {
        for aux in aux_files {
            let content = std::fs::read(aux).map_err(|e| RpcError::SpecRead {
                path: aux.clone(),
                reason: e.to_string(),
            })?;
            sources.push(engine.encode(&content));
        }
    }

    Ok(sources)
}

fn extract_last_state(trace_json: &serde_json::Value) -> Result<serde_json::Value, Error> {
    let states = trace_json
        .get("states")
        .and_then(|s| s.as_array())
        .ok_or(RpcError::MissingStates)?;

    states.last().cloned().ok_or_else(|| RpcError::EmptyTrace.into())
}

fn json_state_to_itf(state: &serde_json::Value) -> Result<itf::Value, Error> {
    serde_json::from_value(state.clone()).map_err(|e| RpcError::StateConversion(e.to_string()).into())
}

fn extract_action(state: &serde_json::Value) -> String {
    state
        .get("action_taken")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string()
}

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
///    a. Shuffle transitions and find first enabled (less chatty than probing all)
///    b. Apply it, query the new state
///    c. Execute the corresponding action on the driver
///    d. Compare spec state with driver state
/// 4. Dispose the session (always, even on error)
#[must_use = "returns a Result that should be checked for test failures"]
pub async fn interactive_test<D: Driver>(
    driver_factory: impl Fn() -> D,
    client: &ApalacheRpcClient,
    config: &InteractiveConfig,
) -> Result<InteractiveStats, Error> {
    interactive_test_with_progress(driver_factory, client, config, None).await
}

/// Interactive test with progress callback, returns stats.
pub async fn interactive_test_with_progress<D: Driver>(
    driver_factory: impl Fn() -> D,
    client: &ApalacheRpcClient,
    config: &InteractiveConfig,
    progress: Option<InteractiveProgressFn>,
) -> Result<InteractiveStats, Error> {
    let start = Instant::now();
    // Spec sources are computed once; each run loads them into a fresh Apalache
    // session because the server maintains per-session symbolic state that must
    // be reset between runs.
    let sources = collect_spec_sources(&config.spec, &config.aux_files)?;

    info!(
        num_runs = config.num_runs,
        max_steps = config.max_steps,
        seed = ?config.seed,
        "Starting interactive symbolic testing"
    );

    let mut rng: Box<dyn RngCore> = match config.seed {
        Some(seed) => Box::new(rand::rngs::StdRng::seed_from_u64(seed)),
        None => Box::new(rand::rng()),
    };

    let mut stats = InteractiveStats::default();

    for run in 0..config.num_runs {
        let mut driver = driver_factory();

        // Each run needs a fresh Apalache session because the server maintains
        // per-session symbolic state that must be reset between runs.
        let load_result = client
            .load_spec(&sources, &config.init, &config.next, &[])
            .await?;

        let session = load_result.session_id.clone();

        let ctx = RunContext {
            client,
            session: &session,
            load_result: &load_result,
            config,
            run,
            total_runs: config.num_runs,
            progress: &progress,
        };

        let result = run_single_test(
            &mut driver,
            &ctx,
            &mut *rng,
            &mut stats,
        )
        .await;

        if let Err(e) = client.dispose_spec(&session).await {
            debug!(run, error = %e, "Failed to dispose spec (non-fatal)");
        }

        result?;
        stats.runs_completed += 1;
        debug!(run, "Run completed successfully");
    }

    stats.duration = start.elapsed();
    info!(
        num_runs = config.num_runs,
        "Interactive symbolic testing completed"
    );
    Ok(stats)
}

struct RunContext<'a> {
    client: &'a ApalacheRpcClient,
    session: &'a str,
    load_result: &'a types::LoadSpecResult,
    config: &'a InteractiveConfig,
    run: usize,
    total_runs: usize,
    progress: &'a Option<InteractiveProgressFn>,
}

async fn run_single_test<D: Driver>(
    driver: &mut D,
    ctx: &RunContext<'_>,
    rng: &mut dyn RngCore,
    stats: &mut InteractiveStats,
) -> Result<(), Error> {
    let next_transitions = &ctx.load_result.spec_parameters.next_transitions;

    if ctx.config.constants.is_object()
        && !ctx.config
            .constants
            .as_object()
            .is_none_or(|m| m.is_empty())
    {
        let result = ctx.client
            .assume_state(ctx.session, ctx.config.constants.clone(), true)
            .await?;

        if result.status != TransitionStatus::Enabled {
            return Err(RpcError::ConstantsUnsatisfiable { run: ctx.run }.into());
        }

        debug!(run = ctx.run, "Constants constrained via assumeState");
    }

    let init_idx = ctx.load_result
        .spec_parameters
        .init_transitions
        .first()
        .map(|t| t.index)
        .unwrap_or(0);

    let assume_result = ctx.client.assume_transition(ctx.session, init_idx, true).await?;

    if assume_result.status != TransitionStatus::Enabled {
        return Err(RpcError::InitDisabled { run: ctx.run }.into());
    }

    let step_result = ctx.client.next_step(ctx.session).await?;
    let mut current_snapshot = step_result.snapshot_id;

    let query = ctx.client.query_trace(ctx.session).await?;
    let trace = query.trace.ok_or(RpcError::MissingStates)?;
    let init_state_json = extract_last_state(&trace)?;
    let init_itf = json_state_to_itf(&init_state_json)?;

    if let Some(ref cb) = ctx.progress {
        cb(InteractiveProgress {
            run_index: ctx.run,
            total_runs: ctx.total_runs,
            step_index: 0,
            action: "init".to_string(),
        });
    }

    let init_step = Step {
        action_taken: "init".to_string(),
        nondet_picks: itf::Value::Tuple(vec![].into()),
        state: init_itf.clone(),
    };

    let step_ctx = StepContext::Rpc { run: ctx.run, step: 0 };
    driver.step(&init_step).map_err(|e| StepError::StepExecution {
        context: step_ctx.clone(),
        action: "init".to_string(),
        reason: e.to_string(),
    })?;

    compare_states::<D>(driver, &init_itf, ctx.run, 0)?;
    stats.total_steps += 1;

    for step_idx in 1..ctx.config.max_steps {
        let mut indices: Vec<u32> = next_transitions.iter().map(|t| t.index).collect();
        indices.shuffle(rng);

        let mut chosen = None;
        for idx in indices {
            let result = ctx.client.assume_transition(ctx.session, idx, true).await?;

            if result.status == TransitionStatus::Enabled {
                chosen = Some(idx);
                break;
            }

            ctx.client.rollback(ctx.session, current_snapshot).await?;
        }

        let Some(_chosen_idx) = chosen else {
            debug!(run = ctx.run, step = step_idx, "No enabled transitions (deadlock)");
            stats.deadlocks_hit += 1;
            break;
        };

        let step_result = ctx.client.next_step(ctx.session).await?;
        current_snapshot = step_result.snapshot_id;

        let query = ctx.client.query_trace(ctx.session).await?;
        let trace = query.trace.ok_or(RpcError::MissingStates)?;
        let state_json = extract_last_state(&trace)?;
        let state_itf = json_state_to_itf(&state_json)?;
        let action_taken = extract_action(&state_json);

        if let Some(ref cb) = ctx.progress {
            cb(InteractiveProgress {
                run_index: ctx.run,
                total_runs: ctx.total_runs,
                step_index: step_idx,
                action: action_taken.clone(),
            });
        }

        let step = Step {
            action_taken: action_taken.clone(),
            nondet_picks: extract_nondet(&state_itf),
            state: state_itf.clone(),
        };

        let step_ctx = StepContext::Rpc { run: ctx.run, step: step_idx };
        driver.step(&step).map_err(|e| StepError::StepExecution {
            context: step_ctx.clone(),
            action: action_taken.clone(),
            reason: e.to_string(),
        })?;

        compare_states::<D>(driver, &state_itf, ctx.run, step_idx)?;
        stats.total_steps += 1;
    }

    Ok(())
}

impl From<std::path::PathBuf> for InteractiveConfig {
    fn from(spec: std::path::PathBuf) -> Self {
        Self {
            spec,
            ..Default::default()
        }
    }
}

impl From<&str> for InteractiveConfig {
    fn from(spec: &str) -> Self {
        Self {
            spec: std::path::PathBuf::from(spec),
            ..Default::default()
        }
    }
}

fn compare_states<D: Driver>(
    driver: &D,
    spec_itf_state: &itf::Value,
    run: usize,
    step: usize,
) -> Result<(), Error> {
    let ctx = StepContext::Rpc { run, step };

    let spec_state = D::State::from_spec(spec_itf_state).map_err(|e| StepError::SpecDeserialize {
        context: ctx.clone(),
        reason: e.to_string(),
    })?;

    let driver_state = <D::State as ExtractState<D>>::from_driver(driver).map_err(|e| StepError::DriverStateExtraction {
        context: ctx.clone(),
        reason: e.to_string(),
    })?;

    if spec_state != driver_state {
        let diff = crate::driver::format_state_mismatch(&spec_state, &driver_state);

        return Err(StepError::StateMismatch {
            context: ctx,
            action: "".to_string(),
            diff,
        }
        .into());
    }

    Ok(())
}
