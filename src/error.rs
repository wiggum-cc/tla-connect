//! Typed errors for tla-connect.
//!
//! Provides structured error types instead of anyhow for better
//! library ergonomics and pattern matching.

#[cfg(any(feature = "replay", feature = "trace-gen", feature = "trace-validation", feature = "rpc"))]
use std::path::PathBuf;
use thiserror::Error;

/// Shared error for Apalache CLI execution failures.
///
/// Used by both `TraceGenError` and `ValidationError` to avoid duplication.
#[cfg(any(feature = "trace-gen", feature = "trace-validation"))]
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ApalacheError {
    /// Apalache execution failed with a non-zero exit code.
    #[error("Apalache failed (exit code: {exit_code:?}): {message}")]
    Execution { exit_code: Option<i32>, message: String },

    /// Apalache binary not found or not executable.
    #[error("Failed to execute Apalache. Is it installed and on PATH? {0}")]
    NotFound(String),

    /// Apalache timed out after the specified duration.
    #[error("Apalache timed out after {duration:?}")]
    Timeout { duration: std::time::Duration },
}

/// Shared error for directory read failures.
///
/// Used by both `ReplayError` and `TraceGenError` to avoid duplication.
#[cfg(any(feature = "replay", feature = "trace-gen"))]
#[derive(Debug, Error)]
#[error("Failed to read directory {path}: {reason}")]
pub struct DirectoryReadError {
    pub path: PathBuf,
    pub reason: String,
}

/// Context for a step-level error, identifying where in a test run the error occurred.
#[cfg(any(feature = "replay", feature = "rpc"))]
#[derive(Debug, Clone)]
pub enum StepContext {
    Replay { trace: usize, state: usize },
    Rpc { run: usize, step: usize },
}

#[cfg(any(feature = "replay", feature = "rpc"))]
impl std::fmt::Display for StepContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepContext::Replay { trace, state } => write!(f, "Trace {trace}, state {state}"),
            StepContext::Rpc { run, step } => write!(f, "Run {run}, step {step}"),
        }
    }
}

/// Shared error for step-level failures during replay or interactive testing.
///
/// Consolidates the duplicated error variants from `ReplayError` and `RpcError`.
#[cfg(any(feature = "replay", feature = "rpc"))]
#[derive(Debug, Error)]
pub enum StepError {
    /// Failed to execute action on driver.
    #[error("{context}: failed to execute action '{action}': {reason}")]
    StepExecution { context: StepContext, action: String, reason: String },

    /// Failed to deserialize spec state.
    #[error("{context}: failed to deserialize spec state: {reason}")]
    SpecDeserialize { context: StepContext, reason: String },

    /// Failed to extract driver state.
    #[error("{context}: failed to extract driver state: {reason}")]
    DriverStateExtraction { context: StepContext, reason: String },

    /// State mismatch between spec and driver.
    #[error("State mismatch at {context} (action: '{action}'):\n{diff}")]
    StateMismatch { context: StepContext, action: String, diff: String },
}

/// Top-level error type for tla-connect operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// Error during ITF trace replay.
    #[cfg(feature = "replay")]
    #[error("Replay error: {0}")]
    Replay(#[from] ReplayError),

    /// Error during Apalache trace generation.
    #[cfg(feature = "trace-gen")]
    #[error("Trace generation error: {0}")]
    TraceGen(#[from] TraceGenError),

    /// Error during trace validation.
    #[cfg(feature = "trace-validation")]
    #[error("Trace validation error: {0}")]
    Validation(#[from] ValidationError),

    /// Error during RPC communication with Apalache server.
    #[cfg(feature = "rpc")]
    #[error("RPC error: {0}")]
    Rpc(#[from] RpcError),

    /// Error during a step (shared between replay and RPC).
    #[cfg(any(feature = "replay", feature = "rpc"))]
    #[error("Step error: {0}")]
    Step(#[from] StepError),

    /// Error in driver step execution.
    #[error("Driver error: {0}")]
    Driver(#[from] DriverError),

    /// Error during configuration building.
    #[error("Builder error: {0}")]
    Builder(#[from] BuilderError),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Error during ITF trace replay.
#[cfg(feature = "replay")]
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

    /// ITF state is not a record.
    #[error("Expected ITF state to be a Record, got: {found}")]
    InvalidStateType { found: String },

    /// Failed to parse ITF trace.
    #[error("Failed to parse ITF trace: {0}")]
    Parse(String),

    /// Directory read error.
    #[error(transparent)]
    DirectoryRead(#[from] DirectoryReadError),
}

/// Error during Apalache trace generation.
#[cfg(feature = "trace-gen")]
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TraceGenError {
    /// TLA+ spec file not found.
    #[error("TLA+ spec not found: {0}")]
    SpecNotFound(PathBuf),

    /// Failed to create temp directory.
    #[error("Failed to create temp directory: {0}")]
    TempDir(String),

    /// Apalache CLI error.
    #[error(transparent)]
    Apalache(#[from] ApalacheError),

    /// No ITF traces found in output.
    #[error("No ITF traces found in Apalache output directory: {0}")]
    NoTracesFound(PathBuf),

    /// Failed to parse ITF trace file.
    #[error("Failed to parse ITF trace {path}: {reason}")]
    TraceParse { path: PathBuf, reason: String },

    /// Directory read error.
    #[error(transparent)]
    DirectoryRead(#[from] DirectoryReadError),
}

/// Error during trace validation (Approach 3).
#[cfg(feature = "trace-validation")]
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

    /// Apalache CLI error.
    #[error(transparent)]
    Apalache(#[from] ApalacheError),

    /// Failed to create work directory.
    #[error("Failed to create work directory: {0}")]
    WorkDir(String),

    /// Failed to copy spec files.
    #[error("Failed to copy spec file {path}: {reason}")]
    FileCopy { path: PathBuf, reason: String },

    /// IO error during validation.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Cannot emit after StateEmitter has been finished.
    #[error("Cannot emit after StateEmitter has been finished")]
    EmitterFinished,

    /// Inconsistent array element types.
    #[error("Inconsistent array element types at field '{field}': expected {expected}, got {found}")]
    InconsistentArrayType {
        field: String,
        expected: String,
        found: String,
    },
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

/// Error during configuration building.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BuilderError {
    /// A required field was not set.
    #[error("Required field '{field}' was not set on {builder}")]
    MissingRequiredField { builder: &'static str, field: &'static str },
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
