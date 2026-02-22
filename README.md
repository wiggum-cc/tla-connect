# tla-connect

TLA+/Apalache integration for model-based testing in Rust.

## Overview

`tla-connect` provides tools for integrating [TLA+](https://lamport.azurewebsites.net/tla/tla.html) and [Apalache](https://apalache.informal.systems/) model checking into Rust test suites. It enables:

- **Trace validation**: Verify that your implementation matches TLA+ specifications
- **Model-based testing**: Generate test cases from TLA+ models
- **Counterexample replay**: Automatically reproduce bugs found by model checkers

## Features

- ITF (Informal Trace Format) parsing and validation
- Apalache JSON-RPC client for running model checks
- Trace generation from TLA+ specifications
- State comparison and diff output for debugging mismatches
- Support for both file-based and RPC-based workflows

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
tla-connect = "0.1"
```

## Quick Start

```rust
use tla_connect::trace_validation::Validator;

// Define your state type
#[derive(Debug, serde::Deserialize)]
struct YourState {
    // Your state fields
}

// Create a validator
let validator = Validator::new();

// Validate traces against your implementation
// See examples for detailed usage
```

## Requirements

- Rust 1.93 or later
- Apalache (if using model checking features)

## Documentation

For detailed documentation, see [docs.rs/tla-connect](https://docs.rs/tla-connect).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
