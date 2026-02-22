//! Typed errors for tla-connect.
//!
//! Provides structured error types instead of anyhow for better
//! library ergonomics and pattern matching.

use std::path::PathBuf;
use thiserror::Error;

/// Top-level error type for tla-connect operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// Error during ITF trace replay.
    #[error("Replay error: {0}")]
    Replay(#[from] ReplayError),

    /// Error during Apalache trace generation.
    #[error("Trace generation error: {0}")]
    TraceGen(#[from] TraceGenError),

    /// Error during trace validation.
    #[error("Trace validation error: {0}")]
    Validation(#[from] ValidationError),

    /// Error during RPC communication with Apalache server.
    #[cfg(feature = "rpc")]
    #[error("RPC error: {0}")]
    Rpc(#[from] RpcError),

    /// Error in driver step execution.
    #[error("Driver error: {0}")]
    Driver(#[from] DriverError),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Error during ITF trace replay.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ReplayError {
    /// Failed to extract MBT variables from ITF state.
    #[error("Trace {trace}, state {state}: failed to extract MBT vars: {reason}")]
    MbtVarExtraction {
        trace: usize,
        state: usize,
        reason: String,
    },

    /// Failed to execute action on driver.
    #[error("Trace {trace}, state {state}: failed to execute action '{action}': {reason}")]
    StepExecution {
        trace: usize,
        state: usize,
        action: String,
        reason: String,
    },

    /// Failed to deserialize spec state.
    #[error("Trace {trace}, state {state}: failed to deserialize spec state: {reason}")]
    SpecDeserialize {
        trace: usize,
        state: usize,
        reason: String,
    },

    /// Failed to extract driver state.
    #[error("Trace {trace}, state {state}: failed to extract driver state: {reason}")]
    DriverStateExtraction {
        trace: usize,
        state: usize,
        reason: String,
    },

    /// State mismatch between spec and driver.
    #[error("State mismatch at trace {trace}, state {state} (action: '{action}'):\n{diff}")]
    StateMismatch {
        trace: usize,
        state: usize,
        action: String,
        diff: String,
    },

    /// ITF state is not a record.
    #[error("Expected ITF state to be a Record, got: {found}")]
    InvalidStateType { found: String },

    /// Failed to parse ITF trace.
    #[error("Failed to parse ITF trace: {0}")]
    Parse(String),

    /// Directory read error.
    #[error("Failed to read directory {path}: {reason}")]
    DirectoryRead { path: PathBuf, reason: String },
}

/// Error during Apalache trace generation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TraceGenError {
    /// TLA+ spec file not found.
    #[error("TLA+ spec not found: {0}")]
    SpecNotFound(PathBuf),

    /// Failed to create temp directory.
    #[error("Failed to create temp directory: {0}")]
    TempDir(String),

    /// Apalache execution failed.
    #[error("Apalache failed (exit code: {exit_code}): {message}")]
    ApalacheExecution { exit_code: i32, message: String },

    /// Apalache binary not found or not executable.
    #[error("Failed to execute Apalache. Is it installed and on PATH? {0}")]
    ApalacheNotFound(String),

    /// No ITF traces found in output.
    #[error("No ITF traces found in Apalache output directory: {0}")]
    NoTracesFound(PathBuf),

    /// Failed to parse ITF trace file.
    #[error("Failed to parse ITF trace {path}: {reason}")]
    TraceParse { path: PathBuf, reason: String },

    /// Failed to read directory.
    #[error("Failed to read directory {path}: {reason}")]
    DirectoryRead { path: PathBuf, reason: String },
}

/// Error during trace validation (Approach 3).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ValidationError {
    /// TraceSpec file not found.
    #[error("TraceSpec not found: {0}")]
    TraceSpecNotFound(PathBuf),

    /// Trace file not found.
    #[error("Trace file not found: {0}")]
    TraceFileNotFound(PathBuf),

    /// Trace file is empty.
    #[error("Trace file is empty: {0}")]
    EmptyTrace(PathBuf),

    /// Invalid JSON in trace file.
    #[error("Invalid JSON on line {line}: {reason}")]
    InvalidJson { line: usize, reason: String },

    /// State must serialize to a JSON object.
    #[error("State must serialize to a JSON object, got: {found}")]
    NonObjectState { found: String },

    /// Inconsistent record schema across trace lines.
    #[error("Inconsistent record schema: line {line} has keys {found:?}, expected {expected:?}")]
    InconsistentSchema {
        line: usize,
        expected: Vec<String>,
        found: Vec<String>,
    },

    /// Unsupported JSON value type.
    #[error("Unsupported JSON value type at line {line}, field '{field}': {reason}")]
    UnsupportedType {
        line: usize,
        field: String,
        reason: String,
    },

    /// Float values not supported (TLA+ uses Int).
    #[error("Float value not supported at line {line}, field '{field}': {value}")]
    FloatNotSupported {
        line: usize,
        field: String,
        value: f64,
    },

    /// Failed to convert to TLA+ record.
    #[error("Failed to convert line {line} to TLA+ record: {reason}")]
    TlaConversion { line: usize, reason: String },

    /// Apalache execution failed.
    #[error("Apalache failed (exit code: {exit_code:?}): {message}")]
    ApalacheExecution {
        exit_code: Option<i32>,
        message: String,
    },

    /// Apalache binary not found.
    #[error("Failed to execute Apalache. Is it installed and on PATH? {0}")]
    ApalacheNotFound(String),

    /// Failed to create work directory.
    #[error("Failed to create work directory: {0}")]
    WorkDir(String),

    /// Failed to copy spec files.
    #[error("Failed to copy spec file {path}: {reason}")]
    FileCopy { path: PathBuf, reason: String },

    /// IO error during validation.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Error during RPC communication with Apalache server.
#[cfg(feature = "rpc")]
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RpcError {
    /// Failed to create HTTP client.
    #[error("Failed to create HTTP client: {0}")]
    ClientCreation(String),

    /// Failed to send request to Apalache server.
    #[error("Failed to send request to {url}. Is the Apalache server running? {reason}")]
    RequestFailed { url: String, reason: String },

    /// Failed to parse JSON-RPC response.
    #[error("Failed to parse JSON-RPC response: {0}")]
    ResponseParse(String),

    /// JSON-RPC error from server.
    #[error("Apalache JSON-RPC error {code}: {message}")]
    JsonRpc { code: i64, message: String },

    /// Missing result in response.
    #[error("JSON-RPC response missing 'result' field")]
    MissingResult,

    /// Failed to deserialize result.
    #[error("Failed to deserialize JSON-RPC result: {0}")]
    ResultDeserialize(String),

    /// Spec file not found.
    #[error("Spec file not found: {0}")]
    SpecNotFound(PathBuf),

    /// Failed to read spec file.
    #[error("Failed to read spec file {path}: {reason}")]
    SpecRead { path: PathBuf, reason: String },

    /// Init transition disabled.
    #[error("Run {run}: Init transition is disabled")]
    InitDisabled { run: usize },

    /// Constants unsatisfiable.
    #[error("Run {run}: Constant constraints are unsatisfiable")]
    ConstantsUnsatisfiable { run: usize },

    /// State mismatch.
    #[error("State mismatch at run {run}, step {step} (action: '{action}'):\nspec:   {spec_state}\ndriver: {driver_state}")]
    StateMismatch {
        run: usize,
        step: usize,
        action: String,
        spec_state: String,
        driver_state: String,
    },

    /// Failed to execute action.
    #[error("Run {run}, step {step}: failed to execute action '{action}': {reason}")]
    StepExecution {
        run: usize,
        step: usize,
        action: String,
        reason: String,
    },

    /// Failed to deserialize spec state.
    #[error("Run {run}, step {step}: failed to deserialize spec state: {reason}")]
    SpecDeserialize {
        run: usize,
        step: usize,
        reason: String,
    },

    /// Failed to extract driver state.
    #[error("Run {run}, step {step}: failed to extract driver state: {reason}")]
    DriverStateExtraction {
        run: usize,
        step: usize,
        reason: String,
    },

    /// Trace missing states.
    #[error("Trace has no states")]
    EmptyTrace,

    /// Trace missing states array.
    #[error("Trace missing 'states' array")]
    MissingStates,

    /// Failed to convert state.
    #[error("Failed to convert state to ITF Value: {0}")]
    StateConversion(String),
}

/// Error during driver step execution.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DriverError {
    /// Unknown action encountered.
    #[error("Unknown action: {0}")]
    UnknownAction(String),

    /// Action execution failed.
    #[error("Action '{action}' failed: {reason}")]
    ActionFailed { action: String, reason: String },

    /// State extraction failed.
    #[error("Failed to extract state: {0}")]
    StateExtraction(String),
}

/// Result type alias using tla-connect's Error.
pub type TlaResult<T> = std::result::Result<T, Error>;
