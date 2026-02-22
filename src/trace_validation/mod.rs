//! Trace validation: record Rust execution traces and validate against TLA+
//! specs using Apalache (Approach 3).
//!
//! This approach provides the reverse direction of verification:
//! - Approaches 1 & 2 catch "implementation doesn't handle a case the spec allows"
//! - Approach 3 catches "implementation does something the spec doesn't allow"
//!
//! ## Workflow
//!
//! 1. Instrument Rust code with `StateEmitter` to record state transitions as NDJSON
//! 2. Write a TLA+ `TraceSpec` that constrains the original spec using the recorded trace
//! 3. Run `validate_trace` to check the trace is a valid behavior of the spec
//!
//! # Example
//!
//! ```ignore
//! use tla_connect::{StateEmitter, validate_trace, TraceValidatorConfig, TraceResult};
//! use serde::Serialize;
//! use std::path::Path;
//!
//! #[derive(Serialize)]
//! struct MyState { counter: i64 }
//!
//! // Record execution
//! let mut emitter = StateEmitter::new(Path::new("trace.ndjson"))?;
//! emitter.emit("init", &MyState { counter: 0 })?;
//! emitter.emit("increment", &MyState { counter: 1 })?;
//! emitter.finish()?;
//!
//! // Validate against spec
//! let config = TraceValidatorConfig::builder()
//!     .trace_spec("specs/CounterTrace.tla")
//!     .build();
//!
//! match validate_trace(&config, Path::new("trace.ndjson"))? {
//!     TraceResult::Valid => println!("Valid!"),
//!     TraceResult::Invalid { reason } => println!("Invalid: {reason}"),
//! }
//! ```

pub mod emitter;
pub mod validator;

pub use emitter::StateEmitter;
pub use validator::{validate_trace, TraceResult, TraceValidatorConfig, TraceValidatorConfigBuilder};

#[doc(hidden)]
pub use validator::ndjson_to_tla_module;
