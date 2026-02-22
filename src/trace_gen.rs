//! Apalache trace generation (Approach 1).
//!
//! Invokes Apalache CLI to generate ITF traces from TLA+ specs via bounded
//! model checking or random simulation.
//!
//! # Example
//!
//! ```ignore
//! use tla_connect::{generate_traces, ApalacheConfig, ApalacheMode};
//!
//! let config = ApalacheConfig::builder()
//!     .spec("specs/Counter.tla")
//!     .inv("TraceComplete")
//!     .max_traces(10)
//!     .mode(ApalacheMode::Simulate)
//!     .build();
//!
//! let generated = generate_traces(&config)?;
//! println!("Generated {} traces", generated.traces.len());
//! ```

use crate::error::{Error, TraceGenError};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Configuration for Apalache trace generation.
#[derive(Debug, Clone)]
#[non_exhaustive]
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

impl ApalacheConfig {
    pub fn builder() -> ApalacheConfigBuilder {
        ApalacheConfigBuilder::default()
    }
}

#[derive(Default)]
pub struct ApalacheConfigBuilder {
    spec: Option<PathBuf>,
    inv: Option<String>,
    max_traces: Option<usize>,
    max_length: Option<usize>,
    view: Option<String>,
    cinit: Option<String>,
    mode: Option<ApalacheMode>,
    apalache_bin: Option<String>,
    out_dir: Option<PathBuf>,
    keep_outputs: Option<bool>,
}

impl ApalacheConfigBuilder {
    pub fn spec(mut self, path: impl Into<PathBuf>) -> Self {
        self.spec = Some(path.into());
        self
    }

    pub fn inv(mut self, inv: impl Into<String>) -> Self {
        self.inv = Some(inv.into());
        self
    }

    pub fn max_traces(mut self, n: usize) -> Self {
        self.max_traces = Some(n);
        self
    }

    pub fn max_length(mut self, n: usize) -> Self {
        self.max_length = Some(n);
        self
    }

    pub fn view(mut self, view: impl Into<String>) -> Self {
        self.view = Some(view.into());
        self
    }

    pub fn cinit(mut self, cinit: impl Into<String>) -> Self {
        self.cinit = Some(cinit.into());
        self
    }

    pub fn mode(mut self, mode: ApalacheMode) -> Self {
        self.mode = Some(mode);
        self
    }

    pub fn apalache_bin(mut self, bin: impl Into<String>) -> Self {
        self.apalache_bin = Some(bin.into());
        self
    }

    pub fn out_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.out_dir = Some(dir.into());
        self
    }

    pub fn keep_outputs(mut self, keep: bool) -> Self {
        self.keep_outputs = Some(keep);
        self
    }

    pub fn build(self) -> ApalacheConfig {
        let defaults = ApalacheConfig::default();
        ApalacheConfig {
            spec: self.spec.unwrap_or(defaults.spec),
            inv: self.inv.unwrap_or(defaults.inv),
            max_traces: self.max_traces.unwrap_or(defaults.max_traces),
            max_length: self.max_length.unwrap_or(defaults.max_length),
            view: self.view.or(defaults.view),
            cinit: self.cinit.or(defaults.cinit),
            mode: self.mode.unwrap_or(defaults.mode),
            apalache_bin: self.apalache_bin.unwrap_or(defaults.apalache_bin),
            out_dir: self.out_dir.or(defaults.out_dir),
            keep_outputs: self.keep_outputs.unwrap_or(defaults.keep_outputs),
        }
    }
}

/// Apalache execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
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
#[non_exhaustive]
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
#[must_use = "contains generated traces that should be used for replay"]
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

impl From<PathBuf> for ApalacheConfig {
    fn from(spec: PathBuf) -> Self {
        Self {
            spec,
            ..Default::default()
        }
    }
}

impl From<&str> for ApalacheConfig {
    fn from(spec: &str) -> Self {
        Self {
            spec: PathBuf::from(spec),
            ..Default::default()
        }
    }
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
