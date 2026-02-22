//! Example: Replay ITF traces against a Driver (Approach 1).
//!
//! This example demonstrates how to:
//! 1. Generate ITF traces from a TLA+ spec using Apalache
//! 2. Replay those traces against a Rust implementation
//! 3. Compare spec state with driver state at each step
//!
//! Run with: cargo run --example replay_itf

use serde::Deserialize;
use tla_connect::*;

/// The state we compare between TLA+ spec and Rust implementation.
#[derive(Debug, PartialEq, Deserialize)]
struct CounterState {
    counter: i64,
}

impl State<CounterDriver> for CounterState {
    fn from_driver(driver: &CounterDriver) -> Result<Self, DriverError> {
        Ok(CounterState {
            counter: driver.value,
        })
    }
}

/// The Rust implementation under test.
struct CounterDriver {
    value: i64,
}

impl Default for CounterDriver {
    fn default() -> Self {
        Self { value: 0 }
    }
}

impl Driver for CounterDriver {
    type State = CounterState;

    fn step(&mut self, step: &Step) -> Result<(), DriverError> {
        switch!(step {
            "init" => {
                self.value = 0;
            },
            "increment" => {
                self.value += 1;
            },
            "decrement" => {
                self.value = self.value.saturating_sub(1);
            },
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ApalacheConfig {
        spec: "specs/Counter.tla".into(),
        inv: "TraceComplete".into(),
        max_traces: 10,
        max_length: 20,
        mode: ApalacheMode::Simulate,
        ..Default::default()
    };

    println!("Generating traces from TLA+ spec...");
    let generated = generate_traces(&config)?;
    println!("Generated {} traces", generated.traces.len());

    println!("Replaying traces against CounterDriver...");
    replay_traces(CounterDriver::default, &generated.traces)?;
    println!("All traces replayed successfully!");

    Ok(())
}
