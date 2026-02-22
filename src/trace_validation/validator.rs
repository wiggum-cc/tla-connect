//! Apalache-based trace validation (Approach 3).
//!
//! Validates that a recorded NDJSON trace is a valid behavior of a TLA+
//! specification by running Apalache on a TraceSpec.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Result of trace validation.
#[derive(Debug)]
pub enum TraceResult {
    /// The trace is a valid behavior of the specification.
    /// Apalache violated the `TraceFinished` invariant (meaning the full trace
    /// was successfully replayed).
    Valid,

    /// The trace is NOT a valid behavior of the specification.
    Invalid {
        /// Human-readable reason for the failure.
        reason: String,
    },
}

/// Configuration for Apalache-based trace validation.
#[derive(Debug, Clone)]
pub struct TraceValidatorConfig {
    /// Path to the TLA+ TraceSpec file.
    pub trace_spec: PathBuf,

    /// INIT predicate name in the TraceSpec (default: "TraceInit").
    pub init: String,

    /// NEXT predicate name in the TraceSpec (default: "TraceNext").
    pub next: String,

    /// Invariant that is violated when the trace is fully consumed
    /// (default: "TraceFinished").
    pub inv: String,

    /// Constant initialization predicate (default: "TraceConstInit").
    pub cinit: String,

    /// Path to the Apalache binary (default: "apalache-mc").
    pub apalache_bin: String,
}

impl Default for TraceValidatorConfig {
    fn default() -> Self {
        Self {
            trace_spec: PathBuf::new(),
            init: "TraceInit".into(),
            next: "TraceNext".into(),
            inv: "TraceFinished".into(),
            cinit: "TraceConstInit".into(),
            apalache_bin: "apalache-mc".into(),
        }
    }
}

/// Validates Rust execution traces against TLA+ specs using Apalache.
///
/// Uses the "inverted invariant" technique: the TraceSpec defines a
/// `TraceFinished` invariant that is violated when the entire trace has
/// been consumed. If Apalache reports a violation, the trace is valid.
///
/// Workflow:
/// 1. Reads the NDJSON trace file
/// 2. Generates a `TraceData.tla` module embedding the trace as TLA+ records
/// 3. Copies spec files + TraceData.tla to a temp work directory
/// 4. Runs `apalache-mc check` with the appropriate flags
pub fn validate_trace(
    config: &TraceValidatorConfig,
    trace_file: &Path,
) -> Result<TraceResult> {
    let trace_spec = config
        .trace_spec
        .canonicalize()
        .with_context(|| format!("TraceSpec not found: {}", config.trace_spec.display()))?;

    let trace_file = trace_file
        .canonicalize()
        .with_context(|| format!("Trace file not found: {}", trace_file.display()))?;

    let spec_dir = trace_spec
        .parent()
        .ok_or_else(|| anyhow::anyhow!("TraceSpec has no parent directory"))?;

    let spec_filename = trace_spec
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("TraceSpec has no filename"))?;

    info!(
        spec = %trace_spec.display(),
        trace = %trace_file.display(),
        "Validating trace with Apalache"
    );

    // Generate TraceData.tla from the NDJSON trace
    let (trace_data, trace_len) = ndjson_to_tla_module(&trace_file)
        .context("Failed to convert NDJSON trace to TLA+ module")?;

    // Create temp work directory with separate spec and output subdirs.
    // Apalache creates <out-dir>/<spec-name>/ which would conflict with the
    // spec file if both lived in the same directory.
    let work_dir = tempfile::Builder::new()
        .prefix("tla_trace_")
        .tempdir()
        .context("Failed to create temp work directory")?;
    let spec_subdir = work_dir.path().join("spec");
    let out_subdir = work_dir.path().join("out");
    std::fs::create_dir_all(&spec_subdir).context("Failed to create spec subdir")?;
    std::fs::create_dir_all(&out_subdir).context("Failed to create out subdir")?;

    // Copy all .tla files from spec dir to work dir
    for entry in std::fs::read_dir(spec_dir)
        .with_context(|| format!("Failed to read spec dir: {}", spec_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("tla") {
            let dest = spec_subdir.join(entry.file_name());
            std::fs::copy(&path, &dest).with_context(|| {
                format!("Failed to copy {} to work dir", path.display())
            })?;
        }
    }

    // Write TraceData.tla to spec subdir
    let trace_data_path = spec_subdir.join("TraceData.tla");
    std::fs::write(&trace_data_path, &trace_data)
        .context("Failed to write TraceData.tla")?;

    debug!(
        "Generated TraceData.tla ({} bytes, {} trace entries)",
        trace_data.len(),
        trace_len
    );

    // Apalache needs --length = trace_len - 1 (number of Next transitions)
    let length = trace_len.saturating_sub(1);

    // Build Apalache command
    let mut cmd = std::process::Command::new(&config.apalache_bin);
    cmd.arg("check")
        .arg(format!("--init={}", config.init))
        .arg(format!("--next={}", config.next))
        .arg(format!("--inv={}", config.inv))
        .arg(format!("--cinit={}", config.cinit))
        .arg(format!("--length={length}"))
        .arg(format!("--out-dir={}", out_subdir.display()))
        .arg(spec_subdir.join(spec_filename));

    debug!("Apalache command: {:?}", cmd);

    let output = cmd
        .output()
        .context("Failed to execute Apalache. Is it installed and on PATH?")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    debug!("Apalache stdout:\n{}", stdout);
    if !stderr.is_empty() {
        debug!("Apalache stderr:\n{}", stderr);
    }

    parse_apalache_output(&stdout, &stderr, output.status.code())
}

/// Parse Apalache output to determine if the trace was valid.
///
/// The "inverted invariant" technique means:
/// - Exit code 12 (invariant violated) = trace was fully consumed = VALID
/// - Exit code 0 (no violation) = trace could not be replayed = INVALID
/// - Other exit codes = errors
fn parse_apalache_output(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
) -> Result<TraceResult> {
    match exit_code {
        // Invariant violation found → trace is valid
        Some(12) => {
            info!("Trace validated successfully (Apalache violated TraceFinished)");
            Ok(TraceResult::Valid)
        }

        // No violation → trace could not be fully replayed
        Some(0) => Ok(TraceResult::Invalid {
            reason: "Apalache completed without violating TraceFinished — \
                     the trace could not be fully replayed against the spec"
                .to_string(),
        }),

        // Errors
        _ => {
            let error_lines: Vec<&str> = stdout
                .lines()
                .filter(|l| l.contains("Error") || l.contains("error"))
                .chain(stderr.lines().filter(|l| !l.is_empty()))
                .collect();

            bail!(
                "Apalache failed (exit code: {:?}):\n{}",
                exit_code,
                error_lines.join("\n")
            );
        }
    }
}

/// Convert an NDJSON trace file to a TLA+ module defining `TraceLog`.
///
/// Returns (module_content, trace_entry_count).
fn ndjson_to_tla_module(trace_file: &Path) -> Result<(String, usize)> {
    let content = std::fs::read_to_string(trace_file)
        .with_context(|| format!("Failed to read trace file: {}", trace_file.display()))?;

    let mut json_objects = Vec::new();
    let mut records = Vec::new();

    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let obj: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("Invalid JSON on line {}", i + 1))?;

        let record = json_obj_to_tla_record(&obj)
            .with_context(|| format!("Failed to convert line {} to TLA+ record", i + 1))?;
        json_objects.push(obj);
        records.push(record);
    }

    if records.is_empty() {
        bail!("Trace file is empty: {}", trace_file.display());
    }

    // Infer Apalache Snowcat type from the first JSON record
    let record_type = infer_snowcat_record_type(&json_objects[0])?;

    // Extract action names as a separate sequence to avoid record field
    // access in the TraceSpec (which causes Apalache Snowcat type issues).
    let actions: Vec<String> = json_objects
        .iter()
        .map(|obj| {
            obj.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string()
        })
        .collect();

    let count = records.len();
    let mut out = String::new();
    out.push_str("---- MODULE TraceData ----\n");
    out.push_str("EXTENDS Integers, Sequences\n\n");

    // TraceLog: full record sequence (for state comparison if needed)
    out.push_str(&format!("\\* @type: () => Seq({record_type});\n"));
    out.push_str("TraceLog == <<\n");
    for (i, record) in records.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        out.push_str("  ");
        out.push_str(record);
    }
    out.push_str("\n>>\n\n");

    // TraceActions: action name sequence (avoids record field access)
    out.push_str("\\* @type: () => Seq(Str);\n");
    out.push_str("TraceActions == <<\n");
    for (i, action) in actions.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        out.push_str(&format!("  \"{action}\""));
    }
    out.push_str("\n>>\n\n====\n");
    Ok((out, count))
}

/// Infer an Apalache Snowcat type string from a JSON object.
///
/// `{"action": "init", "count": 5}` → `{action: Str, count: Int}`
fn infer_snowcat_record_type(value: &serde_json::Value) -> Result<String> {
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Expected JSON object for type inference"))?;

    let mut fields = Vec::new();
    for (key, val) in obj {
        let ty = infer_snowcat_type(val);
        fields.push(format!("{key}: {ty}"));
    }

    Ok(format!("{{{}}}", fields.join(", ")))
}

/// Infer a Snowcat type from a JSON value.
fn infer_snowcat_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Bool(_) => "Bool",
        serde_json::Value::Number(_) => "Int",
        serde_json::Value::String(_) => "Str",
        serde_json::Value::Array(_) => "Seq(Int)", // conservative default
        serde_json::Value::Null | serde_json::Value::Object(_) => "Str",
    }
}

/// Convert a JSON object to a TLA+ record literal.
///
/// `{"action": "init", "count": 5}` → `[action |-> "init", count |-> 5]`
fn json_obj_to_tla_record(value: &serde_json::Value) -> Result<String> {
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Expected JSON object, got: {value}"))?;

    let mut fields = Vec::new();

    for (key, val) in obj {
        let tla_val = json_to_tla_value(val)?;
        fields.push(format!("{key} |-> {tla_val}"));
    }

    Ok(format!("[{}]", fields.join(", ")))
}

/// Convert a JSON value to a TLA+ expression.
fn json_to_tla_value(value: &serde_json::Value) -> Result<String> {
    match value {
        serde_json::Value::Null => Ok("\"null\"".to_string()),
        serde_json::Value::Bool(b) => Ok(if *b {
            "TRUE".to_string()
        } else {
            "FALSE".to_string()
        }),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::String(s) => Ok(format!(
            "\"{}\"",
            s.replace('\\', "\\\\").replace('"', "\\\"")
        )),
        serde_json::Value::Array(arr) => {
            let elems: Result<Vec<String>> = arr.iter().map(json_to_tla_value).collect();
            Ok(format!("<<{}>>", elems?.join(", ")))
        }
        serde_json::Value::Object(_) => json_obj_to_tla_record(value),
    }
}
