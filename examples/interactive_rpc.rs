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

impl State for CounterState {}

impl ExtractState<CounterDriver> for CounterState {
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
                Ok(())
            },
            "increment" => {
                self.value += 1;
                Ok(())
            },
            "decrement" => {
                self.value = self.value.saturating_sub(1);
                Ok(())
            },
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Connecting to Apalache server at http://localhost:8822...");
    let client = ApalacheRpcClient::new("http://localhost:8822")?;

    let config = InteractiveConfig::builder()
        .spec("specs/Counter.tla")
        .init("Init")
        .next("Next")
        .num_runs(50usize)
        .max_steps(100usize)
        .seed(42u64)
        .build()?;

    println!("Running {} interactive test runs...", config.num_runs);
    let _stats = interactive_test(CounterDriver::default, &client, &config).await?;
    println!("All runs completed successfully!");

    Ok(())
}
