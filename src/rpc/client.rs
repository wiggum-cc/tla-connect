//! HTTP client for Apalache's JSON-RPC explorer server (Approach 2).
//!
//! Communicates with a running Apalache server to perform interactive
//! symbolic execution of TLA+ specs.

use super::types::*;
use crate::error::{Error, RpcError};
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::debug;

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 = no retries).
    pub max_retries: u32,
    /// Initial delay between retries.
    pub initial_delay: std::time::Duration,
    /// Backoff multiplier for exponential backoff.
    pub backoff_multiplier: f64,
    /// Maximum delay between retries.
    pub max_delay: std::time::Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: std::time::Duration::from_millis(100),
            backoff_multiplier: 2.0,
            max_delay: std::time::Duration::from_secs(5),
        }
    }
}

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
    retry_config: RetryConfig,
}

impl ApalacheRpcClient {
    /// Create a new client. `url` should be e.g. `http://localhost:8822`.
    /// The `/rpc` path is appended automatically.
    #[must_use = "returns a Result containing the client"]
    pub async fn new(url: &str) -> Result<Self, Error> {
        Self::with_retry_config(url, RetryConfig::default()).await
    }

    /// Create a new client with custom retry configuration.
    #[must_use = "returns a Result containing the client"]
    pub async fn with_retry_config(url: &str, retry_config: RetryConfig) -> Result<Self, Error> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| RpcError::ClientCreation(e.to_string()))?;

        let rpc_url = format!("{}/rpc", url.trim_end_matches('/'));

        Ok(Self {
            url: rpc_url,
            client,
            request_id: AtomicU64::new(1),
            retry_config,
        })
    }

    /// Check if the Apalache server is reachable.
    ///
    /// Sends a GET request to the server's base URL to verify connectivity.
    /// Returns `Ok(())` if the server responds with any 2xx or 4xx status
    /// (a 4xx still indicates the server is running and reachable, just that
    /// the endpoint may not support GET). Returns an error for 5xx responses
    /// or connection failures.
    pub async fn ping(&self) -> Result<(), Error> {
        let response = self
            .client
            .get(self.url.trim_end_matches("/rpc"))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| RpcError::RequestFailed {
                url: self.url.clone(),
                reason: e.to_string(),
            })?;

        if response.status().is_success() || response.status().is_client_error() {
            Ok(())
        } else {
            Err(RpcError::RequestFailed {
                url: self.url.clone(),
                reason: format!("Server returned status {}", response.status()),
            }
            .into())
        }
    }

    /// Load a TLA+ specification into the server.
    ///
    /// `sources` should be base64-encoded contents of each `.tla` file.
    /// This method uses retry logic for transient network failures.
    #[must_use = "returns a Result containing the load result with session ID"]
    pub async fn load_spec(
        &self,
        sources: &[String],
        init: &str,
        next: &str,
        invariants: &[&str],
    ) -> Result<LoadSpecResult, Error> {
        let params = LoadSpecParams {
            sources: sources.to_vec(),
            init: init.to_string(),
            next: next.to_string(),
            invariants: invariants.iter().map(|s| s.to_string()).collect(),
        };

        let result: LoadSpecResult = self.call_with_retry("loadSpec", params).await?;
        debug!(
            session_id = %result.session_id,
            init_transitions = result.spec_parameters.init_transitions.len(),
            next_transitions = result.spec_parameters.next_transitions.len(),
            "Loaded TLA+ spec"
        );
        Ok(result)
    }

    /// Check whether a transition is enabled from the current symbolic state.
    #[must_use = "returns a Result that should be checked"]
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
    #[must_use = "returns a Result that should be checked"]
    pub async fn next_step(&self, session_id: &str) -> Result<NextStepResult, Error> {
        let params = NextStepParams {
            session_id: session_id.to_string(),
        };
        self.call("nextStep", params).await
    }

    /// Roll back to a previously saved snapshot.
    #[must_use = "returns a Result that should be checked"]
    pub async fn rollback(&self, session_id: &str, snapshot_id: u64) -> Result<RollbackResult, Error> {
        let params = RollbackParams {
            session_id: session_id.to_string(),
            snapshot_id,
        };
        self.call("rollback", params).await
    }

    /// Constrain state variables/constants with equality constraints.
    #[must_use = "returns a Result that should be checked"]
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
    #[must_use = "returns a Result that should be checked"]
    pub async fn query_trace(&self, session_id: &str) -> Result<QueryResult, Error> {
        let params = QueryParams {
            session_id: session_id.to_string(),
            kinds: vec!["TRACE".to_string()],
        };
        self.call("query", params).await
    }

    /// Dispose of the loaded specification and free server resources.
    #[must_use = "returns a Result that should be checked"]
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

    /// Send a JSON-RPC request with retry logic using the client's retry config.
    async fn call_with_retry<P: serde::Serialize + Clone, R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, Error> {
        let retry_config = &self.retry_config;
        let mut attempts = 0;
        let mut delay = retry_config.initial_delay;

        loop {
            match self.call(method, params.clone()).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    attempts += 1;
                    if attempts > retry_config.max_retries {
                        return Err(e);
                    }

                    if !is_retryable_error(&e) {
                        return Err(e);
                    }

                    debug!(
                        method = method,
                        attempt = attempts,
                        delay_ms = delay.as_millis() as u64,
                        "Retrying RPC call"
                    );

                    tokio::time::sleep(delay).await;
                    delay = std::cmp::min(
                        std::time::Duration::from_secs_f64(
                            delay.as_secs_f64() * retry_config.backoff_multiplier,
                        ),
                        retry_config.max_delay,
                    );
                }
            }
        }
    }
}

fn is_retryable_error(err: &Error) -> bool {
    matches!(err, Error::Rpc(RpcError::RequestFailed { .. }))
}
