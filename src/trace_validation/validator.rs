//! Apalache-based trace validation (Approach 3).
//!
//! Validates that a recorded NDJSON trace is a valid behavior of a TLA+
//! specification by running Apalache on a TraceSpec.

use crate::error::{Error, ValidationError};
use std::collections::BTreeSet;
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
pub fn validate_trace(config: &TraceValidatorConfig, trace_file: &Path) -> Result<TraceResult, Error> {
    let trace_spec = config
        .trace_spec
        .canonicalize()
        .map_err(|_| ValidationError::TraceSpecNotFound(config.trace_spec.clone()))?;

    let trace_file = trace_file
        .canonicalize()
        .map_err(|_| ValidationError::TraceFileNotFound(trace_file.to_path_buf()))?;

    let spec_dir = trace_spec
        .parent()
        .ok_or_else(|| ValidationError::TraceSpecNotFound(trace_spec.clone()))?;

    let spec_filename = trace_spec
        .file_name()
        .ok_or_else(|| ValidationError::TraceSpecNotFound(trace_spec.clone()))?;

    info!(
        spec = %trace_spec.display(),
        trace = %trace_file.display(),
        "Validating trace with Apalache"
    );

    let (trace_data, trace_len) = ndjson_to_tla_module(&trace_file)?;

    let work_dir = tempfile::Builder::new()
        .prefix("tla_trace_")
        .tempdir()
        .map_err(|e| ValidationError::WorkDir(e.to_string()))?;
    let spec_subdir = work_dir.path().join("spec");
    let out_subdir = work_dir.path().join("out");
    std::fs::create_dir_all(&spec_subdir).map_err(ValidationError::Io)?;
    std::fs::create_dir_all(&out_subdir).map_err(ValidationError::Io)?;

    for entry in std::fs::read_dir(spec_dir).map_err(ValidationError::Io)? {
        let entry = entry.map_err(ValidationError::Io)?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("tla") {
            let dest = spec_subdir.join(entry.file_name());
            std::fs::copy(&path, &dest).map_err(|e| ValidationError::FileCopy {
                path: path.clone(),
                reason: e.to_string(),
            })?;
        }
    }

    let trace_data_path = spec_subdir.join("TraceData.tla");
    std::fs::write(&trace_data_path, &trace_data).map_err(ValidationError::Io)?;

    debug!(
        "Generated TraceData.tla ({} bytes, {} trace entries)",
        trace_data.len(),
        trace_len
    );

    let length = trace_len.saturating_sub(1);

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
        .map_err(|e| ValidationError::ApalacheNotFound(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    debug!("Apalache stdout:\n{}", stdout);
    if !stderr.is_empty() {
        debug!("Apalache stderr:\n{}", stderr);
    }

    parse_apalache_output(&stdout, &stderr, output.status.code())
}

fn parse_apalache_output(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
) -> Result<TraceResult, Error> {
    match exit_code {
        Some(12) => {
            info!("Trace validated successfully (Apalache violated TraceFinished)");
            Ok(TraceResult::Valid)
        }

        Some(0) => Ok(TraceResult::Invalid {
            reason: "Apalache completed without violating TraceFinished — \
                     the trace could not be fully replayed against the spec"
                .to_string(),
        }),

        _ => {
            let error_lines: Vec<&str> = stdout
                .lines()
                .filter(|l| l.contains("Error") || l.contains("error"))
                .chain(stderr.lines().filter(|l| !l.is_empty()))
                .collect();

            Err(ValidationError::ApalacheExecution {
                exit_code,
                message: error_lines.join("\n"),
            }
            .into())
        }
    }
}

/// Convert an NDJSON trace file to a TLA+ module defining `TraceLog`.
fn ndjson_to_tla_module(trace_file: &Path) -> Result<(String, usize), Error> {
    let content = std::fs::read_to_string(trace_file).map_err(ValidationError::Io)?;

    let mut json_objects = Vec::new();
    let mut records = Vec::new();
    let mut expected_keys: Option<BTreeSet<String>> = None;

    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let line_num = i + 1;

        let obj: serde_json::Value = serde_json::from_str(line).map_err(|e| {
            ValidationError::InvalidJson {
                line: line_num,
                reason: e.to_string(),
            }
        })?;

        let obj_map = obj.as_object().ok_or_else(|| ValidationError::NonObjectState {
            found: format!("line {line_num}: {}", obj),
        })?;

        let current_keys: BTreeSet<String> = obj_map.keys().cloned().collect();

        if let Some(ref expected) = expected_keys {
            if &current_keys != expected {
                return Err(ValidationError::InconsistentSchema {
                    line: line_num,
                    expected: expected.iter().cloned().collect(),
                    found: current_keys.into_iter().collect(),
                }
                .into());
            }
        } else {
            expected_keys = Some(current_keys);
        }

        validate_json_types(&obj, line_num)?;

        let record = json_obj_to_tla_record(&obj, line_num)?;
        json_objects.push(obj);
        records.push(record);
    }

    if records.is_empty() {
        return Err(ValidationError::EmptyTrace(trace_file.to_path_buf()).into());
    }

    let record_type = infer_snowcat_record_type(&json_objects[0])?;

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

    out.push_str("\\* @type: () => Seq(Str);\n");
    out.push_str("TraceActions == <<\n");
    for (i, action) in actions.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        out.push_str(&format!("  \"{}\"", escape_tla_string(action)));
    }
    out.push_str("\n>>\n\n====\n");
    Ok((out, count))
}

/// Validate JSON types are supported (reject floats, nested structures).
fn validate_json_types(value: &serde_json::Value, line: usize) -> Result<(), Error> {
    let obj = value.as_object().ok_or_else(|| ValidationError::NonObjectState {
        found: format!("{value}"),
    })?;

    for (key, val) in obj {
        match val {
            serde_json::Value::Number(n) => {
                if n.is_f64() && !n.is_i64() && !n.is_u64() {
                    return Err(ValidationError::FloatNotSupported {
                        line,
                        field: key.clone(),
                        value: n.as_f64().unwrap_or(0.0),
                    }
                    .into());
                }
            }
            serde_json::Value::Array(arr) => {
                for (idx, elem) in arr.iter().enumerate() {
                    if let serde_json::Value::Number(n) = elem {
                        if n.is_f64() && !n.is_i64() && !n.is_u64() {
                            return Err(ValidationError::FloatNotSupported {
                                line,
                                field: format!("{key}[{idx}]"),
                                value: n.as_f64().unwrap_or(0.0),
                            }
                            .into());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn infer_snowcat_record_type(value: &serde_json::Value) -> Result<String, Error> {
    let obj = value.as_object().ok_or_else(|| ValidationError::NonObjectState {
        found: format!("{value}"),
    })?;

    let mut fields = Vec::new();
    for (key, val) in obj {
        let ty = infer_snowcat_type(val);
        fields.push(format!("{key}: {ty}"));
    }

    Ok(format!("{{{}}}", fields.join(", ")))
}

fn infer_snowcat_type(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Bool(_) => "Bool".to_string(),
        serde_json::Value::Number(_) => "Int".to_string(),
        serde_json::Value::String(_) => "Str".to_string(),
        serde_json::Value::Array(arr) => {
            if let Some(first) = arr.first() {
                format!("Seq({})", infer_snowcat_type(first))
            } else {
                "Seq(Int)".to_string()
            }
        }
        serde_json::Value::Object(obj) => {
            let fields: Vec<String> = obj
                .iter()
                .map(|(k, v)| format!("{k}: {}", infer_snowcat_type(v)))
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
        serde_json::Value::Null => "Str".to_string(),
    }
}

fn json_obj_to_tla_record(value: &serde_json::Value, line: usize) -> Result<String, Error> {
    let obj = value.as_object().ok_or_else(|| ValidationError::TlaConversion {
        line,
        reason: format!("Expected JSON object, got: {value}"),
    })?;

    let mut fields = Vec::new();

    for (key, val) in obj {
        let tla_val = json_to_tla_value(val, line, key)?;
        fields.push(format!("{key} |-> {tla_val}"));
    }

    Ok(format!("[{}]", fields.join(", ")))
}

fn json_to_tla_value(value: &serde_json::Value, line: usize, field: &str) -> Result<String, Error> {
    match value {
        serde_json::Value::Null => Ok("\"null\"".to_string()),
        serde_json::Value::Bool(b) => Ok(if *b { "TRUE" } else { "FALSE" }.to_string()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.to_string())
            } else if let Some(u) = n.as_u64() {
                Ok(u.to_string())
            } else {
                Err(ValidationError::FloatNotSupported {
                    line,
                    field: field.to_string(),
                    value: n.as_f64().unwrap_or(0.0),
                }
                .into())
            }
        }
        serde_json::Value::String(s) => Ok(format!("\"{}\"", escape_tla_string(s))),
        serde_json::Value::Array(arr) => {
            let elems: Result<Vec<String>, Error> = arr
                .iter()
                .enumerate()
                .map(|(i, v)| json_to_tla_value(v, line, &format!("{field}[{i}]")))
                .collect();
            Ok(format!("<<{}>>", elems?.join(", ")))
        }
        serde_json::Value::Object(_) => {
            json_obj_to_tla_record(value, line)
        }
    }
}

/// Escape a string for use in a TLA+ string literal.
fn escape_tla_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}
