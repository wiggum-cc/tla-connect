//! Example: Interactive symbolic testing via Apalache JSON-RPC (Approach 2).
//!
//! Prerequisites:
//! - Start the Apalache server:
//!   ```bash
//!   apalache-mc server --port=8822 --server-type=explorer
//!   ```
//!
//! Run with: cargo run --example interactive_rpc --features rpc

use serde::Deserialize;
use tla_connect::*;

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Connecting to Apalache server at http://localhost:8822...");
    let client = ApalacheRpcClient::new("http://localhost:8822").await?;

    let config = InteractiveConfig {
        spec: "specs/Counter.tla".into(),
        init: "Init".into(),
        next: "Next".into(),
        num_runs: 50,
        max_steps: 100,
        seed: Some(42),
        ..Default::default()
    };

    println!("Running {} interactive test runs...", config.num_runs);
    interactive_test(CounterDriver::default, &client, &config).await?;
    println!("All runs completed successfully!");

    Ok(())
}
