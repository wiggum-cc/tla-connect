//! Core abstractions for connecting Rust implementations to TLA+ specs.
//!
//! Mirrors quint-connect's `Driver`/`State`/`Step` pattern, adapted for
//! TLA+ ITF traces produced by Apalache.
//!
//! # Example
//!
//! ```
//! use tla_connect::{Driver, State, ExtractState, Step, DriverError, switch};
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
//! impl State for CounterState {}
//!
//! impl ExtractState<CounterDriver> for CounterState {
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
//!             "init" => { self.value = 0; Ok(()) },
//!             "increment" => { self.value += 1; Ok(()) },
//!         })
//!     }
//! }
//! ```

use crate::error::DriverError;
use serde::de::DeserializeOwned;
use similar::{ChangeTag, TextDiff};
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
///
/// # Parallel replay
///
/// When using [`replay_traces_parallel`](crate::replay_traces_parallel)
/// (requires the `parallel` feature), your `Driver` must also implement
/// `Send`. Each trace is replayed in its own thread, so the driver
/// factory closure must be `Sync` and the resulting driver must be `Send`.
pub trait Driver: Sized {
    /// The state type used for comparing TLA+ spec state with Rust state.
    type State: State + ExtractState<Self>;

    /// Execute a single step from the TLA+ trace on the Rust implementation.
    ///
    /// Use the `switch!` macro to dispatch on `step.action_taken`.
    fn step(&mut self, step: &Step) -> Result<(), DriverError>;
}

/// State comparison between TLA+ spec and Rust implementation.
///
/// Deserializes from ITF `Value` (spec side). Only include fields that should
/// be compared — intentionally exclude fields where spec and implementation
/// have valid semantic differences.
pub trait State: PartialEq + DeserializeOwned + Debug {
    /// Deserialize the spec state from an ITF Value.
    ///
    /// The default implementation uses serde deserialization via `itf::Value`,
    /// which transparently handles ITF-specific encodings (`#bigint`, `#set`, etc.).
    ///
    /// Note: The default implementation clones the `itf::Value` because serde's
    /// `DeserializeOwned` trait requires ownership. This clone happens once per
    /// state per trace and may be significant for large state records. Override
    /// this method if you need to avoid the clone (e.g., by deserializing
    /// specific fields manually).
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

/// Extract the comparable state from the Rust driver.
///
/// Separated from [`State`] so that `State` does not require a generic
/// parameter for the driver type, making it easier to use in contexts
/// that only need deserialization and comparison.
pub trait ExtractState<D>: State {
    /// Extract the comparable state from the Rust driver.
    fn from_driver(driver: &D) -> Result<Self, DriverError>;
}

/// Produce a unified diff between two strings.
pub fn unified_diff(left: &str, right: &str) -> String {
    let diff = TextDiff::from_lines(left, right);
    let mut output = String::new();

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        output.push_str(sign);
        output.push_str(change.value());
        if !change.value().ends_with('\n') {
            output.push('\n');
        }
    }

    output
}

/// Format a state mismatch between spec and driver states for error reporting.
pub fn format_state_mismatch<S: State>(spec: &S, driver: &S) -> String {
    let summary = spec.diff(driver);
    let full = unified_diff(&format!("{spec:#?}"), &format!("{driver:#?}"));
    format!("State differences:\n{summary}\n--- spec (TLA+)\n+++ driver (Rust)\n{full}")
}

/// Helper to create a unified diff between two Debug-formatted values.
///
/// Useful for implementing custom `State::diff` methods.
pub fn debug_diff<T: Debug, U: Debug>(left: &T, right: &U) -> String {
    let left_str = format!("{left:#?}");
    let right_str = format!("{right:#?}");
    unified_diff(&left_str, &right_str)
}

/// Dispatch a TLA+ action to the corresponding Rust code.
///
/// Generates a single flat `match` on `step.action_taken`, mapping each
/// TLA+ action name to the corresponding Rust code block.
///
/// # Usage
///
/// The first argument must be a variable name (identifier) bound to a `&Step`.
/// Each arm body must evaluate to `Result<(), DriverError>`.
///
/// ```ignore
/// tla_connect::switch!(step {
///     "init" => { /* initialization */ Ok(()) },
///     "request_success" => { self.cb.record_success(); Ok(()) },
///     "tick" => { let _ = self.cb.allows_request(); Ok(()) },
/// })
/// ```
#[macro_export]
macro_rules! switch {
    ($step:ident { $( $action:literal => $body:expr ),+ $(,)? }) => {{
        let __tla_step: &$crate::Step = $step;
        match __tla_step.action_taken.as_str() {
            $( $action => { $body }, )+
            other => Err($crate::DriverError::UnknownAction(other.to_string())),
        }
    }};
}
