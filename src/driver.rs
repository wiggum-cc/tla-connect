//! Core abstractions for connecting Rust implementations to TLA+ specs.
//!
//! Mirrors quint-connect's `Driver`/`State`/`Step` pattern, adapted for
//! TLA+ ITF traces produced by Apalache.

use anyhow::Result;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

/// A single step from an Apalache-generated ITF trace.
///
/// Each ITF state record contains the TLA+ variables plus auxiliary MBT
/// variables (`action_taken`, `nondet_picks`) that identify which action
/// was taken and any nondeterministic choices.
#[derive(Debug, Clone)]
pub struct Step {
    /// The TLA+ action that was taken (e.g., "request_success", "tick").
    pub action_taken: String,

    /// Nondeterministic picks made by this step (ITF Value for proper type handling).
    pub nondet_picks: itf::Value,

    /// Full TLA+ state after this step — an `itf::Value::Record` containing
    /// all state variables. Used for state comparison via `State::from_spec`.
    pub state: itf::Value,
}

/// Core trait for connecting Rust implementations to TLA+ specs.
///
/// Implementors hold the Rust type under test and map TLA+ actions
/// to Rust method calls via `step()`.
pub trait Driver: Sized {
    /// The state type used for comparing TLA+ spec state with Rust state.
    type State: State<Self>;

    /// Execute a single step from the TLA+ trace on the Rust implementation.
    ///
    /// Use the `switch!` macro to dispatch on `step.action_taken`.
    fn step(&mut self, step: &Step) -> Result<()>;
}

/// State comparison between TLA+ spec and Rust implementation.
///
/// Deserializes from ITF `Value` (spec side) and extracts from the Driver (Rust side).
/// Only include fields that should be compared — intentionally exclude fields
/// where spec and implementation have valid semantic differences.
pub trait State<D>: PartialEq + DeserializeOwned + Debug {
    /// Extract the comparable state from the Rust driver.
    fn from_driver(driver: &D) -> Result<Self>;

    /// Deserialize the spec state from an ITF Value.
    ///
    /// The default implementation uses serde deserialization via `itf::Value`,
    /// which transparently handles ITF-specific encodings (`#bigint`, `#set`, etc.).
    fn from_spec(value: itf::Value) -> Result<Self> {
        Self::deserialize(value)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize spec state: {e}"))
    }
}

/// Dispatch a TLA+ action to the corresponding Rust code.
///
/// # Usage
///
/// The first argument must be a variable name (identifier) bound to a `&Step`.
///
/// ```ignore
/// tla_connect::switch!(step {
///     "init" => { /* initialization */ },
///     "request_success" => { self.cb.record_success(); },
///     "tick" => { let _ = self.cb.allows_request(); },
/// })
/// ```
#[macro_export]
macro_rules! switch {
    // Entry: accept identifier + braced body, delegate to internal TT muncher
    ($step:ident { $($tt:tt)+ }) => {{
        #[allow(unreachable_code)]
        {
            let __tla_step: &$crate::Step = $step;
            $crate::__switch_arms!(__tla_step; $($tt)+)
        }
    }};
}

/// Internal TT muncher for switch arms. Not part of public API.
#[macro_export]
#[doc(hidden)]
macro_rules! __switch_arms {
    // Final arm (no trailing comma)
    ($step:ident; $action:literal => $body:expr) => {
        match $step.action_taken.as_str() {
            $action => { $body; Ok(()) },
            other => ::anyhow::bail!("Unknown action: {}", other),
        }
    };
    // Final arm (with trailing comma)
    ($step:ident; $action:literal => $body:expr ,) => {
        match $step.action_taken.as_str() {
            $action => { $body; Ok(()) },
            other => ::anyhow::bail!("Unknown action: {}", other),
        }
    };
    // Collect arms via recursion
    ($step:ident; $action:literal => $body:expr, $($rest:tt)+) => {
        match $step.action_taken.as_str() {
            $action => { $body; Ok(()) },
            _ => $crate::__switch_arms!($step; $($rest)+),
        }
    };
}
