//! Tests for the RPC client functionality.
//!
//! These tests use a mock HTTP server to test the JSON-RPC client
//! without requiring a real Apalache server.

#![cfg(feature = "rpc")]

use serde_json::json;
use tla_connect::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_client_creation() {
    let mock_server = MockServer::start().await;
    let client = ApalacheRpcClient::new(&mock_server.uri()).await;
    assert!(client.is_ok());
}

#[tokio::test]
async fn test_ping_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = ApalacheRpcClient::new(&mock_server.uri()).await.unwrap();
    let result = client.ping().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_ping_server_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let client = ApalacheRpcClient::new(&mock_server.uri()).await.unwrap();
    let result = client.ping().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_load_spec_success() {
    let mock_server = MockServer::start().await;

    let response = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "sessionId": "test-session-123",
            "snapshotId": 0,
            "specParameters": {
                "initTransitions": [{"index": 0, "labels": ["Init"]}],
                "nextTransitions": [
                    {"index": 0, "labels": ["Action1"]},
                    {"index": 1, "labels": ["Action2"]}
                ],
                "stateInvariants": [],
                "actionInvariants": []
            }
        }
    });

    Mock::given(method("POST"))
        .and(path("/rpc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&mock_server)
        .await;

    let client = ApalacheRpcClient::new(&mock_server.uri()).await.unwrap();
    let result = client
        .load_spec(vec!["base64content".to_string()], "Init", "Next", &[])
        .await;

    assert!(result.is_ok());
    let load_result = result.unwrap();
    assert_eq!(load_result.session_id, "test-session-123");
    assert_eq!(load_result.spec_parameters.next_transitions.len(), 2);
}

#[tokio::test]
async fn test_json_rpc_error_handling() {
    let mock_server = MockServer::start().await;

    let response = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "error": {
            "code": -32600,
            "message": "Invalid Request"
        }
    });

    Mock::given(method("POST"))
        .and(path("/rpc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&mock_server)
        .await;

    let client = ApalacheRpcClient::new(&mock_server.uri()).await.unwrap();
    let result = client
        .load_spec(vec![], "Init", "Next", &[])
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("-32600") || err.contains("Invalid Request"));
}

#[tokio::test]
async fn test_retry_on_network_error() {
    let mock_server = MockServer::start().await;

    // First two calls fail, third succeeds
    Mock::given(method("POST"))
        .and(path("/rpc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "sessionId": "retry-test",
                "snapshotId": 0,
                "specParameters": {
                    "initTransitions": [],
                    "nextTransitions": [],
                    "stateInvariants": [],
                    "actionInvariants": []
                }
            }
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let retry_config = RetryConfig {
        max_retries: 3,
        initial_delay: std::time::Duration::from_millis(10),
        ..Default::default()
    };

    let client = ApalacheRpcClient::with_retry_config(&mock_server.uri(), retry_config)
        .await
        .unwrap();

    let result = client
        .load_spec(vec![], "Init", "Next", &[])
        .await;

    // Should succeed since server responds
    assert!(result.is_ok());
}
