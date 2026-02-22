//! NDJSON state emitter for recording Rust execution traces (Approach 3).
//!
//! Records state transitions as newline-delimited JSON, one object per line.
//! The resulting trace file is validated against a TLA+ TraceSpec by TLC.

use anyhow::{Context, Result};
use serde::Serialize;
use std::io::Write;
use std::path::Path;

/// Records state transitions as NDJSON for TLC trace validation.
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
    pub fn new(path: &Path) -> Result<Self> {
        let file = std::fs::File::create(path)
            .with_context(|| format!("Failed to create trace file: {}", path.display()))?;
        Ok(Self {
            writer: std::io::BufWriter::new(file),
            count: 0,
        })
    }

    /// Emit a state transition as an NDJSON line.
    ///
    /// The `state` value is serialized as a flat JSON object with an
    /// `"action"` field prepended.
    pub fn emit<S: Serialize>(&mut self, action: &str, state: &S) -> Result<()> {
        // Serialize the state to a JSON Value
        let mut obj = serde_json::to_value(state).context("Failed to serialize state")?;

        // Inject the action field
        if let Some(map) = obj.as_object_mut() {
            map.insert(
                "action".to_string(),
                serde_json::Value::String(action.to_string()),
            );
        }

        // Write as a single line
        serde_json::to_writer(&mut self.writer, &obj)
            .context("Failed to write NDJSON line")?;
        self.writer
            .write_all(b"\n")
            .context("Failed to write newline")?;

        self.count += 1;
        Ok(())
    }

    /// Flush buffered output and return the number of states emitted.
    pub fn finish(mut self) -> Result<usize> {
        self.writer.flush().context("Failed to flush trace file")?;
        Ok(self.count)
    }

    /// Get the number of states emitted so far.
    pub fn count(&self) -> usize {
        self.count
    }
}
