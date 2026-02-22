//! NDJSON state emitter for recording Rust execution traces (Approach 3).
//!
//! Records state transitions as newline-delimited JSON, one object per line.
//! The resulting trace file is validated against a TLA+ TraceSpec by Apalache.

use crate::error::{Error, ValidationError};
use serde::Serialize;
use std::io::Write;
use std::path::Path;

/// Records state transitions as NDJSON for Apalache trace validation.
///
/// Each call to `emit()` writes one JSON object on a new line:
/// ```json
/// {"action": "request_success", "cb_state": "Closed", "failure_count": 0}
/// ```
pub struct StateEmitter {
    writer: std::io::BufWriter<std::fs::File>,
    count: usize,
}

impl StateEmitter {
    /// Create a new emitter writing to the given file path.
    pub fn new(path: &Path) -> Result<Self, Error> {
        let file = std::fs::File::create(path).map_err(ValidationError::Io)?;
        Ok(Self {
            writer: std::io::BufWriter::new(file),
            count: 0,
        })
    }

    /// Emit a state transition as an NDJSON line.
    ///
    /// The `state` value must serialize to a flat JSON object. An `"action"`
    /// field is prepended to identify the transition.
    pub fn emit<S: Serialize>(&mut self, action: &str, state: &S) -> Result<(), Error> {
        let mut obj = serde_json::to_value(state)?;

        let map = obj.as_object_mut().ok_or_else(|| ValidationError::NonObjectState {
            found: format!("{:?}", serde_json::to_value(state).unwrap_or_default()),
        })?;

        map.insert(
            "action".to_string(),
            serde_json::Value::String(action.to_string()),
        );

        serde_json::to_writer(&mut self.writer, &obj)?;
        self.writer
            .write_all(b"\n")
            .map_err(ValidationError::Io)?;

        self.count += 1;
        Ok(())
    }

    /// Flush buffered output and return the number of states emitted.
    pub fn finish(mut self) -> Result<usize, Error> {
        self.writer.flush().map_err(ValidationError::Io)?;
        Ok(self.count)
    }

    /// Get the number of states emitted so far.
    pub fn count(&self) -> usize {
        self.count
    }
}
