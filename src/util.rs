//! Internal utility functions.

/// Run a subprocess command with an optional timeout.
///
/// If `timeout` is `Some`, spawns the process and polls `try_wait` in a loop,
/// killing the child if it exceeds the timeout. If `timeout` is `None`, uses
/// the standard blocking `output()` call.
#[cfg(any(feature = "trace-gen", feature = "trace-validation"))]
pub fn run_with_timeout(
    cmd: &mut std::process::Command,
    timeout: Option<std::time::Duration>,
) -> Result<std::process::Output, crate::error::ApalacheError> {
    use crate::error::ApalacheError;

    let Some(timeout) = timeout else {
        let output = cmd
            .output()
            .map_err(|e| ApalacheError::NotFound(e.to_string()))?;
        return Ok(output);
    };

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| ApalacheError::NotFound(e.to_string()))?;

    let start = std::time::Instant::now();
    let poll_interval = std::time::Duration::from_millis(100);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = child.stdout.take().map_or_else(Vec::new, |mut s| {
                    let mut buf = Vec::new();
                    let _ = std::io::Read::read_to_end(&mut s, &mut buf);
                    buf
                });
                let stderr = child.stderr.take().map_or_else(Vec::new, |mut s| {
                    let mut buf = Vec::new();
                    let _ = std::io::Read::read_to_end(&mut s, &mut buf);
                    buf
                });
                return Ok(std::process::Output { status, stdout, stderr });
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ApalacheError::Timeout { duration: timeout });
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => {
                return Err(ApalacheError::NotFound(e.to_string()));
            }
        }
    }
}
