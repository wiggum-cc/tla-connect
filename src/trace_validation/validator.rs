//! Apalache-based trace validation (Approach 3).
//!
//! Validates that a recorded NDJSON trace is a valid behavior of a TLA+
//! specification by running Apalache on a TraceSpec.

use crate::error::{Error, ValidationError};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Result of trace validation.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "trace validation result should be checked"]
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
#[non_exhaustive]
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

impl TraceValidatorConfig {
    pub fn builder() -> TraceValidatorConfigBuilder {
        TraceValidatorConfigBuilder::default()
    }
}

#[derive(Default)]
pub struct TraceValidatorConfigBuilder {
    trace_spec: Option<PathBuf>,
    init: Option<String>,
    next: Option<String>,
    inv: Option<String>,
    cinit: Option<String>,
    apalache_bin: Option<String>,
}

impl TraceValidatorConfigBuilder {
    pub fn trace_spec(mut self, path: impl Into<PathBuf>) -> Self {
        self.trace_spec = Some(path.into());
        self
    }

    pub fn init(mut self, init: impl Into<String>) -> Self {
        self.init = Some(init.into());
        self
    }

    pub fn next(mut self, next: impl Into<String>) -> Self {
        self.next = Some(next.into());
        self
    }

    pub fn inv(mut self, inv: impl Into<String>) -> Self {
        self.inv = Some(inv.into());
        self
    }

    pub fn cinit(mut self, cinit: impl Into<String>) -> Self {
        self.cinit = Some(cinit.into());
        self
    }

    pub fn apalache_bin(mut self, bin: impl Into<String>) -> Self {
        self.apalache_bin = Some(bin.into());
        self
    }

    pub fn build(self) -> Result<TraceValidatorConfig, crate::error::BuilderError> {
        let defaults = TraceValidatorConfig::default();
        let trace_spec = self.trace_spec.ok_or(crate::error::BuilderError::MissingRequiredField {
            builder: "TraceValidatorConfigBuilder",
            field: "trace_spec",
        })?;
        Ok(TraceValidatorConfig {
            trace_spec,
            init: self.init.unwrap_or(defaults.init),
            next: self.next.unwrap_or(defaults.next),
            inv: self.inv.unwrap_or(defaults.inv),
            cinit: self.cinit.unwrap_or(defaults.cinit),
            apalache_bin: self.apalache_bin.unwrap_or(defaults.apalache_bin),
        })
    }
}

/// Validates Rust execution traces against TLA+ specs using Apalache.
///
/// Uses the "inverted invariant" technique: the TraceSpec defines a
/// `TraceFinished` invariant that is violated when the entire trace has
/// been consumed. If Apalache reports a violation, the trace is valid.
#[must_use = "validation result should be checked"]
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
        .map_err(|e| ValidationError::from(crate::error::ApalacheError::NotFound(e.to_string())))?;

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

            Err(ValidationError::from(crate::error::ApalacheError::Execution {
                exit_code,
                message: error_lines.join("\n"),
            })
            .into())
        }
    }
}

/// Convert an NDJSON trace file to a TLA+ module defining `TraceLog`.
#[doc(hidden)]
pub fn ndjson_to_tla_module(trace_file: &Path) -> Result<(String, usize), Error> {
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
        validate_json_value(val, line, key)?;
    }
    Ok(())
}

/// Recursively validate a JSON value, rejecting floats at any depth.
fn validate_json_value(value: &serde_json::Value, line: usize, field: &str) -> Result<(), Error> {
    match value {
        serde_json::Value::Number(n) => {
            if n.is_f64() && !n.is_i64() && !n.is_u64() {
                return Err(ValidationError::FloatNotSupported {
                    line,
                    field: field.to_string(),
                    value: n.as_f64().unwrap_or(0.0),
                }
                .into());
            }
        }
        serde_json::Value::Array(arr) => {
            for (idx, elem) in arr.iter().enumerate() {
                validate_json_value(elem, line, &format!("{field}[{idx}]"))?;
            }
        }
        serde_json::Value::Object(obj) => {
            for (key, val) in obj {
                validate_json_value(val, line, &format!("{field}.{key}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn infer_snowcat_record_type(value: &serde_json::Value) -> Result<String, Error> {
    let obj = value.as_object().ok_or_else(|| ValidationError::NonObjectState {
        found: format!("{value}"),
    })?;

    let sorted: BTreeMap<_, _> = obj.iter().collect();
    let mut fields = Vec::new();
    for (key, val) in &sorted {
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
            let sorted: BTreeMap<_, _> = obj.iter().collect();
            let fields: Vec<String> = sorted
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

    let sorted: BTreeMap<_, _> = obj.iter().collect();
    let mut fields = Vec::new();

    for (key, val) in &sorted {
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

impl From<PathBuf> for TraceValidatorConfig {
    fn from(trace_spec: PathBuf) -> Self {
        Self {
            trace_spec,
            ..Default::default()
        }
    }
}

impl From<&str> for TraceValidatorConfig {
    fn from(trace_spec: &str) -> Self {
        Self {
            trace_spec: PathBuf::from(trace_spec),
            ..Default::default()
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn escape_tla_string_plain() {
        assert_eq!(escape_tla_string("hello"), "hello");
    }

    #[test]
    fn escape_tla_string_special_chars() {
        assert_eq!(escape_tla_string("a\\b"), "a\\\\b");
        assert_eq!(escape_tla_string("a\"b"), "a\\\"b");
        assert_eq!(escape_tla_string("a\nb"), "a\\nb");
        assert_eq!(escape_tla_string("a\rb"), "a\\rb");
        assert_eq!(escape_tla_string("a\tb"), "a\\tb");
    }

    #[test]
    fn escape_tla_string_control_char() {
        let result = escape_tla_string("a\x01b");
        assert_eq!(result, "a\\u0001b");
    }

    #[test]
    fn json_to_tla_value_null() {
        let val = json!(null);
        assert_eq!(json_to_tla_value(&val, 1, "f").unwrap(), "\"null\"");
    }

    #[test]
    fn json_to_tla_value_bool() {
        assert_eq!(json_to_tla_value(&json!(true), 1, "f").unwrap(), "TRUE");
        assert_eq!(json_to_tla_value(&json!(false), 1, "f").unwrap(), "FALSE");
    }

    #[test]
    fn json_to_tla_value_int() {
        assert_eq!(json_to_tla_value(&json!(42), 1, "f").unwrap(), "42");
        assert_eq!(json_to_tla_value(&json!(-7), 1, "f").unwrap(), "-7");
    }

    #[test]
    fn json_to_tla_value_string() {
        assert_eq!(json_to_tla_value(&json!("hello"), 1, "f").unwrap(), "\"hello\"");
    }

    #[test]
    fn json_to_tla_value_array() {
        assert_eq!(json_to_tla_value(&json!([1, 2, 3]), 1, "f").unwrap(), "<<1, 2, 3>>");
        assert_eq!(json_to_tla_value(&json!([]), 1, "f").unwrap(), "<<>>");
    }

    #[test]
    fn json_to_tla_value_float_rejected() {
        assert!(json_to_tla_value(&json!(3.14), 1, "f").is_err());
    }

    #[test]
    fn validate_json_types_nested_float() {
        // Float nested in array of arrays should be rejected
        let val = json!({"data": [[3.14]]});
        assert!(validate_json_types(&val, 1).is_err());
    }

    #[test]
    fn validate_json_types_nested_object_float() {
        // Float nested in object should be rejected
        let val = json!({"outer": {"inner": 3.14}});
        assert!(validate_json_types(&val, 1).is_err());
    }

    #[test]
    fn validate_json_types_valid() {
        let val = json!({"a": 1, "b": "str", "c": true, "d": [1, 2]});
        assert!(validate_json_types(&val, 1).is_ok());
    }

    #[test]
    fn infer_snowcat_type_primitives() {
        assert_eq!(infer_snowcat_type(&json!(true)), "Bool");
        assert_eq!(infer_snowcat_type(&json!(42)), "Int");
        assert_eq!(infer_snowcat_type(&json!("hi")), "Str");
        assert_eq!(infer_snowcat_type(&json!(null)), "Str");
    }

    #[test]
    fn infer_snowcat_type_array() {
        assert_eq!(infer_snowcat_type(&json!([1, 2])), "Seq(Int)");
        assert_eq!(infer_snowcat_type(&json!([])), "Seq(Int)");
    }

    #[test]
    fn json_obj_to_tla_record_sorted() {
        let val = json!({"z": 1, "a": 2});
        let record = json_obj_to_tla_record(&val, 1).unwrap();
        // Fields should be sorted alphabetically
        assert!(record.starts_with("[a |->"));
    }

    #[test]
    fn builder_missing_required_field() {
        let result = TraceValidatorConfig::builder().build();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("trace_spec"));
    }
}
