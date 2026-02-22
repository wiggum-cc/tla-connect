//! HTTP client for Apalache's JSON-RPC explorer server (Approach 2).
//!
//! Communicates with a running Apalache server to perform interactive
//! symbolic execution of TLA+ specs.

use super::types::*;
use crate::error::{Error, RpcError};
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::debug;

/// Client for Apalache's JSON-RPC explorer server.
///
/// The server must be started separately:
/// ```bash
/// apalache-mc server --port=8822 --server-type=explorer
/// ```
pub struct ApalacheRpcClient {
    url: String,
    client: reqwest::Client,
    request_id: AtomicU64,
}

impl ApalacheRpcClient {
    /// Create a new client. `url` should be e.g. `http://localhost:8822`.
    /// The `/rpc` path is appended automatically.
    pub async fn new(url: &str) -> Result<Self, Error> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| RpcError::ClientCreation(e.to_string()))?;

        let rpc_url = format!("{}/rpc", url.trim_end_matches('/'));

        Ok(Self {
            url: rpc_url,
            client,
            request_id: AtomicU64::new(1),
        })
    }

    /// Load a TLA+ specification into the server.
    ///
    /// `sources` should be base64-encoded contents of each `.tla` file.
    pub async fn load_spec(
        &self,
        sources: Vec<String>,
        init: &str,
        next: &str,
        invariants: &[&str],
    ) -> Result<LoadSpecResult, Error> {
        let params = LoadSpecParams {
            sources,
            init: init.to_string(),
            next: next.to_string(),
            invariants: invariants.iter().map(|s| s.to_string()).collect(),
        };

        let result: LoadSpecResult = self.call("loadSpec", params).await?;
        debug!(
            session_id = %result.session_id,
            init_transitions = result.spec_parameters.init_transitions.len(),
            next_transitions = result.spec_parameters.next_transitions.len(),
            "Loaded TLA+ spec"
        );
        Ok(result)
    }

    /// Check whether a transition is enabled from the current symbolic state.
    pub async fn assume_transition(
        &self,
        session_id: &str,
        transition_id: u32,
        check_enabled: bool,
    ) -> Result<AssumeTransitionResult, Error> {
        let params = AssumeTransitionParams {
            session_id: session_id.to_string(),
            transition_id,
            check_enabled,
        };
        self.call("assumeTransition", params).await
    }

    /// Advance to the next state after a transition has been assumed.
    pub async fn next_step(&self, session_id: &str) -> Result<NextStepResult, Error> {
        let params = NextStepParams {
            session_id: session_id.to_string(),
        };
        self.call("nextStep", params).await
    }

    /// Roll back to a previously saved snapshot.
    pub async fn rollback(&self, session_id: &str, snapshot_id: u64) -> Result<RollbackResult, Error> {
        let params = RollbackParams {
            session_id: session_id.to_string(),
            snapshot_id,
        };
        self.call("rollback", params).await
    }

    /// Constrain state variables/constants with equality constraints.
    pub async fn assume_state(
        &self,
        session_id: &str,
        equalities: serde_json::Value,
        check_enabled: bool,
    ) -> Result<AssumeStateResult, Error> {
        let params = AssumeStateParams {
            session_id: session_id.to_string(),
            equalities,
            check_enabled,
        };
        self.call("assumeState", params).await
    }

    /// Query the current trace from the symbolic execution.
    pub async fn query_trace(&self, session_id: &str) -> Result<QueryResult, Error> {
        let params = QueryParams {
            session_id: session_id.to_string(),
            kinds: vec!["TRACE".to_string()],
        };
        self.call("query", params).await
    }

    /// Dispose of the loaded specification and free server resources.
    pub async fn dispose_spec(&self, session_id: &str) -> Result<DisposeSpecResult, Error> {
        let params = DisposeSpecParams {
            session_id: session_id.to_string(),
        };
        self.call("disposeSpec", params).await
    }

    /// Send a JSON-RPC request and parse the response.
    async fn call<P: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, Error> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);
        let request = JsonRpcRequest::new(id, method, params);

        debug!(method = method, id = id, "Sending JSON-RPC request");

        let response = self
            .client
            .post(&self.url)
            .json(&request)
            .send()
            .await
            .map_err(|e| RpcError::RequestFailed {
                url: self.url.clone(),
                reason: e.to_string(),
            })?;

        let rpc_response: JsonRpcResponse = response
            .json()
            .await
            .map_err(|e| RpcError::ResponseParse(e.to_string()))?;

        if let Some(error) = rpc_response.error {
            return Err(RpcError::JsonRpc {
                code: error.code,
                message: error.message,
            }
            .into());
        }

        let result_value = rpc_response.result.ok_or(RpcError::MissingResult)?;

        serde_json::from_value(result_value)
            .map_err(|e| RpcError::ResultDeserialize(e.to_string()).into())
    }
}
