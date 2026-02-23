//! tla-connect: TLA+/Apalache integration for model-based testing.
//!
//! Provides three complementary approaches for connecting TLA+ formal
//! specifications to Rust implementations:
//!
//! 1. **Approach 1 (Apalache -> ITF -> replay)**: Batch trace generation + replay.
//!    Apalache generates ITF traces from TLA+ specs, which are replayed against
//!    a Rust `Driver`. Direct equivalent of the Quint/quint-connect workflow.
//!    Requires features: `replay`, `trace-gen`.
//!
//! 2. **Approach 2 (Apalache JSON-RPC)**: Interactive symbolic testing.
//!    Step-by-step symbolic execution via Apalache's explorer server, interleaved
//!    with Rust implementation execution. Requires feature: `rpc`.
//!
//! 3. **Approach 3 (Rust -> NDJSON -> Apalache)**: Post-hoc trace validation.
//!    Record Rust execution traces as NDJSON, then validate against a TLA+
//!    TraceSpec using Apalache. Requires feature: `trace-validation`.
//!
//! Approaches 1 & 2 catch "implementation doesn't handle a case the spec allows."
//! Approach 3 catches "implementation does something the spec doesn't allow."
//!
//! # Feature Flags
//!
//! - `replay` (default): ITF trace replay against a Driver
//! - `trace-gen` (default): Apalache CLI trace generation
//! - `trace-validation` (default): Post-hoc NDJSON trace validation
//! - `rpc`: Interactive symbolic testing via Apalache JSON-RPC
//! - `parallel`: Parallel trace replay using rayon
//! - `full`: Enable all features
//!
//! # Quick Start (Approach 1)
//!
//! ```ignore
//! use tla_connect::*;
//!
//! #[derive(Debug, PartialEq, Deserialize)]
//! struct MyState { /* TLA+ vars to compare */ }
//!
//! impl State<MyDriver> for MyState {
//!     fn from_driver(driver: &MyDriver) -> Result<Self, DriverError> { /* ... */ }
//! }
//!
//! struct MyDriver { /* Rust type under test */ }
//!
//! impl Driver for MyDriver {
//!     type State = MyState;
//!     fn step(&mut self, step: &Step) -> Result<(), DriverError> {
//!         switch!(step {
//!             "init" => { /* init */ },
//!             "action1" => { /* ... */ },
//!         })
//!     }
//! }
//!
//! let traces = generate_traces(&ApalacheConfig {
//!     spec: "../../formal/tla/MySpec.tla".into(),
//!     ..Default::default()
//! })?;
//! replay_traces(|| MyDriver::default(), &traces.traces)?;
//! ```

pub mod driver;
pub mod error;

#[cfg(feature = "replay")]
pub mod replay;

#[cfg(feature = "rpc")]
pub mod rpc;

#[cfg(feature = "trace-gen")]
pub mod trace_gen;

#[cfg(feature = "trace-validation")]
pub mod trace_validation;

// Re-export core types (always available)
pub use driver::{Driver, State, Step};
#[cfg(feature = "replay")]
pub use driver::debug_diff;
pub use error::{ApalacheError, BuilderError, DirectoryReadError, DriverError, Error, ReplayError, TlaResult, TraceGenError, ValidationError};

// Re-export replay types
#[cfg(feature = "replay")]
pub use replay::{
    replay_trace_str, replay_traces, replay_traces_with_progress, ReplayProgress, ReplayProgressFn,
    ReplayStats,
};

#[cfg(feature = "parallel")]
pub use replay::replay_traces_parallel;

// Re-export RPC types
#[cfg(feature = "rpc")]
pub use error::RpcError;
#[cfg(feature = "rpc")]
pub use rpc::{
    interactive_test, interactive_test_with_progress, ApalacheRpcClient, InteractiveConfig,
    InteractiveConfigBuilder, InteractiveProgress, InteractiveProgressFn, InteractiveStats,
    RetryConfig,
};

// Re-export trace generation types
#[cfg(feature = "trace-gen")]
pub use trace_gen::{generate_traces, ApalacheConfig, ApalacheConfigBuilder, ApalacheMode, GeneratedTraces};

// Re-export trace validation types
#[cfg(feature = "trace-validation")]
pub use trace_validation::{validate_trace, StateEmitter, TraceResult, TraceValidatorConfig, TraceValidatorConfigBuilder};
#[cfg(feature = "trace-validation")]
#[doc(hidden)]
pub use trace_validation::ndjson_to_tla_module;
