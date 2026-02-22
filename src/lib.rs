//! tla-connect: TLA+/Apalache integration for model-based testing.
//!
//! Provides three complementary approaches for connecting TLA+ formal
//! specifications to Rust implementations:
//!
//! 1. **Approach 1 (Apalache -> ITF -> replay)**: Batch trace generation + replay.
//!    Apalache generates ITF traces from TLA+ specs, which are replayed against
//!    a Rust `Driver`. Direct equivalent of the Quint/quint-connect workflow.
//!
//! 2. **Approach 2 (Apalache JSON-RPC)**: Interactive symbolic testing.
//!    Step-by-step symbolic execution via Apalache's explorer server, interleaved
//!    with Rust implementation execution.
//!
//! 3. **Approach 3 (Rust -> NDJSON -> Apalache)**: Post-hoc trace validation.
//!    Record Rust execution traces as NDJSON, then validate against a TLA+
//!    TraceSpec using Apalache.
//!
//! Approaches 1 & 2 catch "implementation doesn't handle a case the spec allows."
//! Approach 3 catches "implementation does something the spec doesn't allow."
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
//!     fn from_driver(driver: &MyDriver) -> Result<Self> { /* ... */ }
//! }
//!
//! struct MyDriver { /* Rust type under test */ }
//!
//! impl Driver for MyDriver {
//!     type State = MyState;
//!     fn step(&mut self, step: &Step) -> Result<()> {
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
//! replay_traces(|| MyDriver::default(), &traces)?;
//! ```

pub mod driver;
pub mod replay;
pub mod rpc;
pub mod trace_gen;
pub mod trace_validation;

// Re-export core types for convenience
pub use driver::{Driver, State, Step};
pub use replay::{replay_trace_str, replay_traces};
pub use rpc::{interactive_test, ApalacheRpcClient, InteractiveConfig};
pub use trace_gen::{generate_traces, ApalacheConfig, ApalacheMode};
pub use trace_validation::{validate_trace, StateEmitter, TraceResult, TraceValidatorConfig};
