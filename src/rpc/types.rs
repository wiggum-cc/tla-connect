//! JSON-RPC request/response types for Apalache's explorer server.
//!
//! Apalache v0.52+ exposes a JSON-RPC server at `/rpc` when started with
//! `--server-type=explorer`. Sources are base64-encoded.

use serde::{Deserialize, Serialize};

/// JSON-RPC 2.0 request envelope.
#[derive(Debug, Serialize)]
pub struct JsonRpcRequest<P: Serialize> {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    pub params: P,
}

impl<P: Serialize> JsonRpcRequest<P> {
    pub fn new(id: u64, method: impl Into<String>, params: P) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 response envelope.
#[derive(Debug, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

// ---------------------------------------------------------------------------
// loadSpec
// ---------------------------------------------------------------------------

/// Parameters for `loadSpec`.
#[derive(Debug, Clone, Serialize)]
pub struct LoadSpecParams {
    /// Base64-encoded TLA+ source files.
    pub sources: Vec<String>,

    /// Name of the Init predicate.
    pub init: String,

    /// Name of the Next relation.
    pub next: String,

    /// Invariant names (optional).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub invariants: Vec<String>,
}

/// Result of `loadSpec`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSpecResult {
    pub session_id: String,
    pub snapshot_id: u64,
    pub spec_parameters: SpecParameters,
}

/// Spec metadata returned by `loadSpec`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecParameters {
    pub init_transitions: Vec<Transition>,
    pub next_transitions: Vec<Transition>,
    #[serde(default)]
    pub state_invariants: Vec<serde_json::Value>,
    #[serde(default)]
    pub action_invariants: Vec<serde_json::Value>,
}

/// A transition descriptor.
#[derive(Debug, Clone, Deserialize)]
pub struct Transition {
    pub index: u32,
    #[serde(default)]
    pub labels: Vec<String>,
}

// ---------------------------------------------------------------------------
// assumeTransition
// ---------------------------------------------------------------------------

/// Parameters for `assumeTransition`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssumeTransitionParams {
    pub session_id: String,
    pub transition_id: u32,
    pub check_enabled: bool,
}

/// Result of `assumeTransition`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssumeTransitionResult {
    pub session_id: String,
    pub snapshot_id: u64,
    pub transition_id: u32,
    pub status: TransitionStatus,
}

/// Whether a transition was enabled or disabled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransitionStatus {
    Enabled,
    Disabled,
}

// ---------------------------------------------------------------------------
// nextStep
// ---------------------------------------------------------------------------

/// Parameters for `nextStep`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NextStepParams {
    pub session_id: String,
}

/// Result of `nextStep`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextStepResult {
    pub session_id: String,
    pub snapshot_id: u64,
    pub new_step_no: u64,
}

// ---------------------------------------------------------------------------
// rollback
// ---------------------------------------------------------------------------

/// Parameters for `rollback`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackParams {
    pub session_id: String,
    pub snapshot_id: u64,
}

/// Result of `rollback`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackResult {
    pub session_id: String,
    pub snapshot_id: u64,
}

// ---------------------------------------------------------------------------
// assumeState
// ---------------------------------------------------------------------------

/// Parameters for `assumeState`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssumeStateParams {
    pub session_id: String,
    /// Equality constraints as `{varName: itfValue}`.
    pub equalities: serde_json::Value,
    pub check_enabled: bool,
}

/// Result of `assumeState`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssumeStateResult {
    pub session_id: String,
    pub snapshot_id: u64,
    pub status: TransitionStatus,
}

// ---------------------------------------------------------------------------
// query
// ---------------------------------------------------------------------------

/// Parameters for `query`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryParams {
    pub session_id: String,
    pub kinds: Vec<String>,
}

/// Result of `query`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub session_id: String,
    /// ITF trace (present when `kinds` includes `"TRACE"`).
    pub trace: Option<serde_json::Value>,
    pub operator_value: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// disposeSpec
// ---------------------------------------------------------------------------

/// Parameters for `disposeSpec`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisposeSpecParams {
    pub session_id: String,
}

/// Result of `disposeSpec`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisposeSpecResult {
    pub session_id: String,
}
