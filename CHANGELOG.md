# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Feature flags**: `replay`, `trace-gen`, `trace-validation`, `rpc`, `parallel`, `full`
- **Typed errors**: Replaced `anyhow` with `thiserror` for structured error handling
  - `ReplayError`, `TraceGenError`, `ValidationError`, `RpcError`, `DriverError`
- **Builder patterns**: `ApalacheConfigBuilder`, `InteractiveConfigBuilder`, `TraceValidatorConfigBuilder`
- **Progress callbacks**: `replay_traces_with_progress`, `interactive_test_with_progress`
- **Statistics**: `ReplayStats`, `InteractiveStats` returned from test runs
- **Parallel replay**: `replay_traces_parallel` with rayon (requires `parallel` feature)
- **RPC improvements**:
  - `ApalacheRpcClient::ping()` for health checks
  - `RetryConfig` for configurable retry with exponential backoff
  - `ApalacheRpcClient::with_retry_config()` constructor
- **State comparison**: `State::diff()` trait method for custom diff formatting
- **Helper functions**: `debug_diff()` for unified diff output
- **Seedable RNG**: `InteractiveConfig::seed` for reproducible test runs

### Changed

- `State::from_spec` now takes `&itf::Value` instead of owned value
- `replay_traces` now accepts `impl IntoIterator` instead of `&[Trace]`
- `GeneratedTraces` now owns temp directory and cleans up on drop
- RPC testing shuffles transitions and stops at first enabled (less chatty)
- All public enums/structs marked `#[non_exhaustive]` for semver safety
- All `Result`-returning functions marked `#[must_use]`

### Fixed

- Temp directory leak in `generate_traces` - now properly cleaned up
- RPC session cleanup always runs even on error
- NDJSON validation rejects floats, validates consistent schema, escapes strings

## [0.1.0] - Unreleased

### Added

- Initial release
- Three approaches for TLA+/Rust integration:
  1. Batch trace replay (Apalache CLI → ITF → Driver)
  2. Interactive symbolic testing (JSON-RPC to Apalache explorer)
  3. Post-hoc trace validation (NDJSON → Apalache TraceSpec)
- Core traits: `Driver`, `State`
- `switch!` macro for action dispatch
- ITF trace parsing via `itf` crate
- State comparison with unified diff output

[Unreleased]: https://github.com/wiggum-cc/tla-connect/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/wiggum-cc/tla-connect/releases/tag/v0.1.0
