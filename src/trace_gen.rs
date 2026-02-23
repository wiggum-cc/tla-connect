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

    /// Timeout for the Apalache subprocess. If None, no timeout is applied.
    pub timeout: Option<std::time::Duration>,
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
            timeout: None,
        }
    }
}

crate::builder::impl_builder!(ApalacheConfig, ApalacheConfigBuilder {
    required { spec: PathBuf }
    optional { inv: String, max_traces: usize, max_length: usize,
               mode: ApalacheMode, apalache_bin: String, keep_outputs: bool }
    optional_or { view: String, cinit: String, out_dir: PathBuf, timeout: std::time::Duration }
});

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
            temp.keep()
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
            if config.keep_outputs {
                let path = tmp.keep();
                (path, None)
            } else {
                let path = tmp.path().to_path_buf();
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

    let output = crate::util::run_with_timeout(&mut cmd, config.timeout)
        .map_err(TraceGenError::from)?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let exit_code = output.status.code().unwrap_or(-1);
    if exit_code != 0 && exit_code != 12 {
        return Err(TraceGenError::from(crate::error::ApalacheError::Execution {
            exit_code: Some(exit_code),
            message: format!("stdout: {stdout}\nstderr: {stderr}"),
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walkdir_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let files = walkdir(tmp.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn walkdir_with_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "").unwrap();

        let files = walkdir(tmp.path()).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn walkdir_recursive() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(tmp.path().join("top.txt"), "").unwrap();
        std::fs::write(sub.join("nested.txt"), "").unwrap();

        let files = walkdir(tmp.path()).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn walkdir_nonexistent_returns_empty() {
        let files = walkdir(Path::new("/nonexistent/dir")).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn builder_missing_spec_returns_error() {
        let result = ApalacheConfig::builder().build();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("spec"));
    }

    #[test]
    fn builder_with_spec_succeeds() {
        let config = ApalacheConfig::builder()
            .spec("test.tla")
            .build()
            .unwrap();
        assert_eq!(config.spec, PathBuf::from("test.tla"));
        assert_eq!(config.inv, "TraceComplete");
        assert_eq!(config.max_traces, 100);
    }

    #[test]
    fn config_from_str() {
        let config: ApalacheConfig = "test.tla".into();
        assert_eq!(config.spec, PathBuf::from("test.tla"));
    }
}

/// Simple recursive directory walker.
fn walkdir(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return Ok(files);
    }
    for entry in std::fs::read_dir(dir).map_err(|e| TraceGenError::from(crate::error::DirectoryReadError {
        path: dir.to_path_buf(),
        reason: e.to_string(),
    }))? {
        let entry = entry.map_err(|e| TraceGenError::from(crate::error::DirectoryReadError {
            path: dir.to_path_buf(),
            reason: e.to_string(),
        }))?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(walkdir(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}
