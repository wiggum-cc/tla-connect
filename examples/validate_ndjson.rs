//! Example: Post-hoc trace validation (Approach 3).
//!
//! This approach catches "implementation does something the spec doesn't allow"
//! (the reverse of Approaches 1 & 2).
//!
//! Run with: cargo run --example validate_ndjson

use serde::Serialize;
use std::path::Path;
use tla_connect::*;

/// State to record in the trace.
#[derive(Serialize)]
struct RecordedState {
    counter: i64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let trace_path = Path::new("target/example_trace.ndjson");

    println!("Recording execution trace to {}...", trace_path.display());
    record_execution(trace_path)?;

    println!("Validating trace against TLA+ spec...");
    let config = TraceValidatorConfig::builder()
        .trace_spec("specs/CounterTrace.tla")
        .init("TraceInit")
        .next("TraceNext")
        .inv("TraceFinished")
        .cinit("TraceConstInit")
        .build();

    let result = validate_trace(&config, trace_path)?;

    match result {
        TraceResult::Valid => {
            println!("✓ Trace is valid! Implementation matches spec.");
        }
        TraceResult::Invalid { reason } => {
            println!("✗ Trace is invalid: {reason}");
            std::process::exit(1);
        }
        _ => {
            println!("✗ Unexpected result variant");
            std::process::exit(1);
        }
    }

    Ok(())
}

fn record_execution(path: &Path) -> Result<(), Error> {
    let mut emitter = StateEmitter::new(path)?;

    let mut counter = 0i64;

    emitter.emit("init", &RecordedState { counter })?;

    counter += 1;
    emitter.emit("increment", &RecordedState { counter })?;

    counter += 1;
    emitter.emit("increment", &RecordedState { counter })?;

    counter -= 1;
    emitter.emit("decrement", &RecordedState { counter })?;

    let count = emitter.finish()?;
    println!("Recorded {} state transitions", count);

    Ok(())
}
