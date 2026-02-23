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
///
/// Call [`finish()`](Self::finish) when done to flush buffered output.
/// If dropped without calling `finish()`, the destructor will attempt to
/// flush but any errors will be silently ignored.
pub struct StateEmitter {
    writer: Option<std::io::BufWriter<std::fs::File>>,
    count: usize,
    finished: bool,
}

impl StateEmitter {
    /// Create a new emitter writing to the given file path.
    #[must_use = "emitter should be used to emit states and then finished"]
    pub fn new(path: &Path) -> Result<Self, Error> {
        let file = std::fs::File::create(path).map_err(ValidationError::Io)?;
        Ok(Self {
            writer: Some(std::io::BufWriter::new(file)),
            count: 0,
            finished: false,
        })
    }

    /// Emit a state transition as an NDJSON line.
    ///
    /// The `state` value must serialize to a flat JSON object. An `"action"`
    /// field is prepended to identify the transition.
    #[must_use = "emit result should be checked for errors"]
    pub fn emit<S: Serialize>(&mut self, action: &str, state: &S) -> Result<(), Error> {
        let mut obj = serde_json::to_value(state)?;

        let map = obj.as_object_mut().ok_or_else(|| ValidationError::NonObjectState {
            found: format!("{:?}", serde_json::to_value(state).unwrap_or_default()),
        })?;

        map.insert(
            "action".to_string(),
            serde_json::Value::String(action.to_string()),
        );

        let writer = self.writer.as_mut().ok_or(ValidationError::EmitterFinished)?;
        serde_json::to_writer(&mut *writer, &obj)?;
        writer
            .write_all(b"\n")
            .map_err(ValidationError::Io)?;

        self.count += 1;
        Ok(())
    }

    /// Flush buffered output and return the number of states emitted.
    #[must_use = "finish result should be checked for errors"]
    pub fn finish(mut self) -> Result<usize, Error> {
        self.flush_inner()?;
        self.finished = true;
        Ok(self.count)
    }

    /// Get the number of states emitted so far.
    pub fn count(&self) -> usize {
        self.count
    }

    fn flush_inner(&mut self) -> Result<(), Error> {
        if let Some(ref mut writer) = self.writer {
            writer.flush().map_err(ValidationError::Io)?;
        }
        Ok(())
    }
}

impl Drop for StateEmitter {
    fn drop(&mut self) {
        if !self.finished {
            if self.count > 0 {
                tracing::warn!(
                    count = self.count,
                    "StateEmitter dropped without calling finish() — flushing buffered output"
                );
            }
            let _ = self.flush_inner();
        }
    }
}
