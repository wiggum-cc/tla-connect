//! Core abstractions for connecting Rust implementations to TLA+ specs.
//!
//! Mirrors quint-connect's `Driver`/`State`/`Step` pattern, adapted for
//! TLA+ ITF traces produced by Apalache.
//!
//! # Example
//!
//! ```
//! use tla_connect::{Driver, State, Step, DriverError, switch};
//! use serde::Deserialize;
//!
//! #[derive(Debug, PartialEq, Deserialize)]
//! struct CounterState {
//!     counter: i64,
//! }
//!
//! struct CounterDriver {
//!     value: i64,
//! }
//!
//! impl State<CounterDriver> for CounterState {
//!     fn from_driver(driver: &CounterDriver) -> Result<Self, DriverError> {
//!         Ok(CounterState { counter: driver.value })
//!     }
//! }
//!
//! impl Driver for CounterDriver {
//!     type State = CounterState;
//!
//!     fn step(&mut self, step: &Step) -> Result<(), DriverError> {
//!         switch!(step {
//!             "init" => { self.value = 0; },
//!             "increment" => { self.value += 1; },
//!         })
//!     }
//! }
//! ```

use crate::error::DriverError;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

/// A single step from an Apalache-generated ITF trace.
///
/// Each ITF state record contains the TLA+ variables plus auxiliary MBT
/// variables (`action_taken`, `nondet_picks`) that identify which action
/// was taken and any nondeterministic choices.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
    fn step(&mut self, step: &Step) -> Result<(), DriverError>;
}

/// State comparison between TLA+ spec and Rust implementation.
///
/// Deserializes from ITF `Value` (spec side) and extracts from the Driver (Rust side).
/// Only include fields that should be compared — intentionally exclude fields
/// where spec and implementation have valid semantic differences.
pub trait State<D>: PartialEq + DeserializeOwned + Debug {
    /// Extract the comparable state from the Rust driver.
    fn from_driver(driver: &D) -> Result<Self, DriverError>;

    /// Deserialize the spec state from an ITF Value.
    ///
    /// The default implementation uses serde deserialization via `itf::Value`,
    /// which transparently handles ITF-specific encodings (`#bigint`, `#set`, etc.).
    ///
    /// Takes a reference to avoid unnecessary cloning.
    fn from_spec(value: &itf::Value) -> Result<Self, DriverError> {
        Self::deserialize(value.clone()).map_err(|e| DriverError::StateExtraction(e.to_string()))
    }

    /// Generate a human-readable diff between two states.
    ///
    /// The default implementation uses Debug formatting with unified diff.
    /// Override this for custom diff output (e.g., field-by-field comparison).
    fn diff(&self, other: &Self) -> String {
        use std::fmt::Write;
        let self_str = format!("{self:#?}");
        let other_str = format!("{other:#?}");

        let mut output = String::new();
        let self_lines: Vec<&str> = self_str.lines().collect();
        let other_lines: Vec<&str> = other_str.lines().collect();

        for (i, (a, b)) in self_lines.iter().zip(other_lines.iter()).enumerate() {
            if a != b {
                let _ = writeln!(output, "  line {}: {} -> {}", i + 1, a.trim(), b.trim());
            }
        }

        if self_lines.len() != other_lines.len() {
            let _ = writeln!(
                output,
                "  (line count differs: {} vs {})",
                self_lines.len(),
                other_lines.len()
            );
        }

        if output.is_empty() {
            output = "(states appear equal but PartialEq returned false)".to_string();
        }

        output
    }
}

/// Helper to create a unified diff between two Debug-formatted values.
///
/// Useful for implementing custom `State::diff` methods.
#[cfg(feature = "replay")]
pub fn debug_diff<T: Debug, U: Debug>(left: &T, right: &U) -> String {
    let left_str = format!("{left:#?}");
    let right_str = format!("{right:#?}");
    crate::replay::unified_diff(&left_str, &right_str)
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
            other => Err($crate::DriverError::UnknownAction(other.to_string())),
        }
    };
    // Final arm (with trailing comma)
    ($step:ident; $action:literal => $body:expr ,) => {
        match $step.action_taken.as_str() {
            $action => { $body; Ok(()) },
            other => Err($crate::DriverError::UnknownAction(other.to_string())),
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
