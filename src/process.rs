//! Bounded output and cleanup for trusted bundled helper processes.
use std::{
    io::{Read, Write},
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};

pub const NO_WINDOW: u32 = 0x0800_0000;
const MAX_STDOUT: usize = 32 * 1024 * 1024;
const MAX_STDERR: usize = 256 * 1024;
const MAX_STDIN: usize = 1024;

pub fn hidden(path: &Path) -> Command {
    let mut command = Command::new(path);
    command.creation_flags(NO_WINDOW);
    command
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn read_output(
    mut reader: impl Read,
    limit: usize,
    overflow: &AtomicBool,
) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(output);
        }
        let remaining = limit.saturating_sub(output.len());
        output.extend_from_slice(&buffer[..count.min(remaining)]);
        if count > remaining {
            // Keep draining until the parent kills the process; never grow past the cap.
            overflow.store(true, Ordering::Release);
        }
    }
}

/// Collects bounded helper output within one deadline, including pipe completion.
/// Timeout, oversized output and I/O errors terminate the owned helper.
pub fn output_timeout(
    mut command: Command,
    timeout: Duration,
    input: Option<&str>,
) -> Result<Vec<u8>> {
    if input.is_some_and(|value| value.len() > MAX_STDIN) {
        bail!("Helper input exceeds the supported size.");
    }
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let deadline = Instant::now() + timeout;
    let mut child = ChildGuard(command.spawn().context("Could not launch helper process")?);
    let stdout = child
        .0
        .stdout
        .take()
        .context("Helper stdout is unavailable")?;
    let stderr = child
        .0
        .stderr
        .take()
        .context("Helper stderr is unavailable")?;
    let overflow = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    for (is_error, reader, limit) in [
        (false, Box::new(stdout) as Box<dyn Read + Send>, MAX_STDOUT),
        (true, Box::new(stderr) as Box<dyn Read + Send>, MAX_STDERR),
    ] {
        let tx = tx.clone();
        let overflow = Arc::clone(&overflow);
        thread::Builder::new()
            .name("helper-output".into())
            .spawn(move || {
                let _ = tx.send((is_error, read_output(reader, limit, &overflow)));
            })
            .context("Could not start helper output reader")?;
    }
    drop(tx);
    if let Some(input) = input {
        let mut stdin = child
            .0
            .stdin
            .take()
            .context("Helper stdin is unavailable")?;
        stdin
            .write_all(input.as_bytes())
            .context("Could not send helper input")?;
    }
    let (mut stdout, mut stderr, mut status) = (None, None, None);
    loop {
        if overflow.load(Ordering::Acquire) {
            bail!("Helper output exceeds the supported size.");
        }
        if Instant::now() >= deadline {
            bail!("Device request timed out.");
        }
        if status.is_none() {
            status = child.0.try_wait()?;
        }
        if status.is_some() && stdout.is_some() && stderr.is_some() {
            break;
        }
        match rx.recv_timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(15)),
        ) {
            Ok((is_error, result)) => {
                let bytes = result.context("Could not read helper output")?;
                if is_error {
                    stderr = Some(bytes);
                } else {
                    stdout = Some(bytes);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) if stdout.is_none() || stderr.is_none() => {
                bail!("Helper output reader stopped unexpectedly.")
            }
            Err(_) => thread::sleep(Duration::from_millis(1)),
        }
    }
    let stdout = stdout.context("Missing helper output")?;
    let stderr = stderr.context("Missing helper error output")?;
    if !status.context("Missing helper exit status")?.success() {
        let bytes = if stderr.is_empty() { &stdout } else { &stderr };
        let mut detail = String::from_utf8_lossy(bytes).trim().to_owned();
        if let Some(secret) = input.map(str::trim).filter(|value| !value.is_empty()) {
            detail = detail.replace(secret, "[redacted]");
        }
        bail!(
            "{}",
            if detail.is_empty() {
                "Device request failed."
            } else {
                &detail
            }
        );
    }
    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_reader_caps_storage_and_marks_overflow() {
        let overflow = AtomicBool::new(false);
        let result = read_output(&b"abcdef"[..], 3, &overflow).unwrap();
        assert_eq!(result, b"abc");
        assert!(overflow.load(Ordering::Acquire));
    }
    #[test]
    fn output_reader_accepts_exact_limit() {
        let overflow = AtomicBool::new(false);
        assert_eq!(read_output(&b"abc"[..], 3, &overflow).unwrap(), b"abc");
        assert!(!overflow.load(Ordering::Acquire));
    }
    #[test]
    fn helper_rejects_large_stdin_before_launch() {
        let error = output_timeout(
            hidden(Path::new("does-not-exist.exe")),
            Duration::from_secs(1),
            Some(&"x".repeat(MAX_STDIN + 1)),
        )
        .unwrap_err();
        assert!(error.to_string().contains("input exceeds"));
    }
    #[test]
    fn helper_deadline_terminates_a_slow_owned_process() {
        let system = std::env::var_os("SystemRoot").unwrap();
        let executable =
            std::path::PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut command = hidden(&executable);
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 5",
        ]);
        let start = Instant::now();
        let error = output_timeout(command, Duration::from_millis(100), None).unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
