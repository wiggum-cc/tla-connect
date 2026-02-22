//! Apalache trace generation (Approach 1).
//!
//! Invokes Apalache CLI to generate ITF traces from TLA+ specs via bounded
//! model checking or random simulation.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Configuration for Apalache trace generation.
#[derive(Debug, Clone)]
pub struct ApalacheConfig {
    /// Path to the TLA+ spec file.
    pub spec: PathBuf,

    /// Invariant to violate for trace generation (e.g., "TraceComplete").
    /// Counterexamples to this invariant become test traces.
    pub inv: String,

    /// Maximum number of traces to generate.
    pub max_traces: usize,

    /// Maximum trace length (number of steps).
    pub max_length: usize,

    /// View operator for trace diversity (optional).
    /// Ensures generated traces differ in this projection.
    pub view: Option<String>,

    /// Constant initialization predicate (optional, e.g., "ConstInit").
    /// Used with `--cinit` to set CONSTANTS from a TLA+ predicate.
    pub cinit: Option<String>,

    /// Apalache execution mode.
    pub mode: ApalacheMode,

    /// Path to the Apalache binary (default: "apalache-mc").
    pub apalache_bin: String,

    /// Output directory override (default: temp directory).
    pub out_dir: Option<PathBuf>,
}

impl Default for ApalacheConfig {
    fn default() -> Self {
        Self {
            spec: PathBuf::new(),
            inv: "TraceComplete".into(),
            max_traces: 100,
            max_length: 50,
            view: None,
            cinit: None,
            mode: ApalacheMode::Simulate,
            apalache_bin: "apalache-mc".into(),
            out_dir: None,
        }
    }
}

/// Apalache execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApalacheMode {
    /// Bounded model checking (systematic exploration).
    /// Uses `apalache-mc check`. Exhaustive up to `max_length` steps.
    Check,

    /// Random simulation (like Quint `run`).
    /// Uses `apalache-mc simulate`. Faster but not exhaustive.
    Simulate,
}

/// Generate ITF traces by invoking Apalache on a TLA+ spec.
///
/// Returns parsed ITF traces that can be replayed against a `Driver`.
pub fn generate_traces(
    config: &ApalacheConfig,
) -> Result<Vec<itf::Trace<itf::Value>>> {
    let out_dir = match &config.out_dir {
        Some(dir) => dir.clone(),
        None => {
            let tmp = tempfile::tempdir().context("Failed to create temp directory")?;
            // Leak the tempdir so it persists until process exit
            let path = tmp.path().to_path_buf();
            std::mem::forget(tmp);
            path
        }
    };

    let spec_path = config
        .spec
        .canonicalize()
        .with_context(|| format!("TLA+ spec not found: {}", config.spec.display()))?;

    let mut cmd = std::process::Command::new(&config.apalache_bin);

    match config.mode {
        ApalacheMode::Simulate => {
            cmd.arg("simulate")
                .arg(format!("--inv={}", config.inv))
                .arg(format!("--max-run={}", config.max_traces))
                .arg(format!("--length={}", config.max_length));
        }
        ApalacheMode::Check => {
            cmd.arg("check")
                .arg(format!("--inv={}", config.inv))
                .arg(format!("--max-error={}", config.max_traces))
                .arg(format!("--length={}", config.max_length));
        }
    }

    if let Some(ref cinit) = config.cinit {
        cmd.arg(format!("--cinit={cinit}"));
    }

    if let Some(ref view) = config.view {
        cmd.arg(format!("--view={view}"));
    }

    cmd.arg(format!("--out-dir={}", out_dir.display()))
        .arg(&spec_path);

    info!(
        mode = ?config.mode,
        spec = %spec_path.display(),
        inv = %config.inv,
        "Running Apalache trace generation"
    );
    debug!("Command: {:?}", cmd);

    let output = cmd
        .output()
        .context("Failed to execute Apalache. Is it installed and on PATH?")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // For simulate mode, exit code 12 (invariant violation) is expected — that's
    // how we get traces. For check mode, same applies.
    // Only fail on truly unexpected exit codes (not 0 or 12).
    let exit_code = output.status.code().unwrap_or(-1);
    if exit_code != 0 && exit_code != 12 {
        bail!(
            "Apalache failed (exit code: {exit_code}):\nstdout: {stdout}\nstderr: {stderr}"
        );
    }

    // Collect ITF files from the output directory
    collect_itf_traces(&out_dir)
}

/// Collect all `.itf.json` files from an Apalache output directory.
///
/// Apalache writes traces to `<out_dir>/<spec_name>/<timestamp>/` depending
/// on version.
fn collect_itf_traces(
    out_dir: &Path,
) -> Result<Vec<itf::Trace<itf::Value>>> {
    let mut traces = Vec::new();

    // Walk the output directory recursively looking for .itf.json files
    for path in walkdir(out_dir)? {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if filename.ends_with(".itf.json") {
            debug!(path = %path.display(), "Found ITF trace file");
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read ITF file: {}", path.display()))?;
            // Parse directly via serde_json to avoid itf::trace_from_str's
            // decode() step, which loses BigInt type info through deserialize_any.
            let trace: itf::Trace<itf::Value> =
                serde_json::from_str(&content).with_context(|| {
                    format!("Failed to parse ITF trace: {}", path.display())
                })?;
            traces.push(trace);
        }
    }

    if traces.is_empty() {
        bail!(
            "No ITF traces found in Apalache output directory: {}",
            out_dir.display()
        );
    }

    info!(count = traces.len(), "Collected ITF traces");
    Ok(traces)
}

/// Simple recursive directory walker (avoids adding walkdir as dependency).
fn walkdir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return Ok(files);
    }
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read directory: {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(walkdir(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}
