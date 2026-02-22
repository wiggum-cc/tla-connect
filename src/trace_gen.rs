//! Apalache trace generation (Approach 1).
//!
//! Invokes Apalache CLI to generate ITF traces from TLA+ specs via bounded
//! model checking or random simulation.

use crate::error::{Error, TraceGenError};
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
    /// If None, a temp directory is created and owned by the returned `GeneratedTraces`.
    pub out_dir: Option<PathBuf>,

    /// Whether to keep the output directory after `GeneratedTraces` is dropped.
    /// Only relevant when `out_dir` is None (temp directory).
    pub keep_outputs: bool,
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
            keep_outputs: false,
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

/// Result of trace generation, owning the output directory.
///
/// The temp directory (if created) is cleaned up when this struct is dropped,
/// unless `keep_outputs` was set in the config.
pub struct GeneratedTraces {
    /// The generated ITF traces.
    pub traces: Vec<itf::Trace<itf::Value>>,

    /// Path to the output directory containing raw Apalache output.
    pub out_dir: PathBuf,

    /// Owned temp directory (cleaned up on drop unless persisted).
    _temp: Option<tempfile::TempDir>,
}

impl GeneratedTraces {
    /// Persist the output directory, preventing cleanup on drop.
    ///
    /// Returns the path to the persisted directory.
    pub fn persist(mut self) -> PathBuf {
        if let Some(temp) = self._temp.take() {
            let path = temp.path().to_path_buf();
            // Prevent cleanup by forgetting the TempDir
            std::mem::forget(temp);
            path
        } else {
            self.out_dir.clone()
        }
    }
}

/// Generate ITF traces by invoking Apalache on a TLA+ spec.
///
/// Returns a `GeneratedTraces` struct containing the parsed traces and
/// owning the output directory (cleaned up on drop unless persisted).
pub fn generate_traces(config: &ApalacheConfig) -> Result<GeneratedTraces, Error> {
    let (out_dir, temp) = match &config.out_dir {
        Some(dir) => (dir.clone(), None),
        None => {
            let tmp = tempfile::tempdir()
                .map_err(|e| TraceGenError::TempDir(e.to_string()))?;
            let path = tmp.path().to_path_buf();
            if config.keep_outputs {
                // Prevent cleanup by forgetting the TempDir
                std::mem::forget(tmp);
                (path, None)
            } else {
                (path, Some(tmp))
            }
        }
    };

    let spec_path = config
        .spec
        .canonicalize()
        .map_err(|_| TraceGenError::SpecNotFound(config.spec.clone()))?;

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
        .arg::<&std::path::Path>(&spec_path);

    info!(
        mode = ?config.mode,
        spec = %spec_path.display(),
        inv = %config.inv,
        "Running Apalache trace generation"
    );
    debug!("Command: {:?}", cmd);

    let output = cmd
        .output()
        .map_err(|e| TraceGenError::ApalacheNotFound(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let exit_code = output.status.code().unwrap_or(-1);
    if exit_code != 0 && exit_code != 12 {
        return Err(TraceGenError::ApalacheExecution {
            exit_code,
            message: format!("stdout: {stdout}\nstderr: {stderr}"),
        }
        .into());
    }

    let traces = collect_itf_traces(&out_dir)?;

    Ok(GeneratedTraces {
        traces,
        out_dir,
        _temp: temp,
    })
}

/// Collect all `.itf.json` files from an Apalache output directory.
fn collect_itf_traces(out_dir: &Path) -> Result<Vec<itf::Trace<itf::Value>>, Error> {
    let mut traces = Vec::new();

    for path in walkdir(out_dir)? {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if filename.ends_with(".itf.json") {
            debug!(path = %path.display(), "Found ITF trace file");
            let content = std::fs::read_to_string(&path).map_err(|e| TraceGenError::TraceParse {
                path: path.clone(),
                reason: e.to_string(),
            })?;
            let trace: itf::Trace<itf::Value> =
                serde_json::from_str(&content).map_err(|e| TraceGenError::TraceParse {
                    path: path.clone(),
                    reason: e.to_string(),
                })?;
            traces.push(trace);
        }
    }

    if traces.is_empty() {
        return Err(TraceGenError::NoTracesFound(out_dir.to_path_buf()).into());
    }

    info!(count = traces.len(), "Collected ITF traces");
    Ok(traces)
}

/// Simple recursive directory walker.
fn walkdir(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return Ok(files);
    }
    for entry in std::fs::read_dir(dir).map_err(|e| TraceGenError::DirectoryRead {
        path: dir.to_path_buf(),
        reason: e.to_string(),
    })? {
        let entry = entry.map_err(|e| TraceGenError::DirectoryRead {
            path: dir.to_path_buf(),
            reason: e.to_string(),
        })?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(walkdir(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}
