//! Test execution, output capture, and replay.

use crate::signals;
use anyhow::{Context, Result};
use std::{
    ffi::{OsStr, OsString},
    io::{self, Read, Write},
    process::{Command, ExitStatus, Stdio},
    thread,
};

pub struct CapturedOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub struct Execution {
    pub status: ExitStatus,
    pub output: CapturedOutput,
    pub interrupted: bool,
}

pub fn run(executable: &OsStr, args: &[OsString]) -> Result<Execution> {
    let (stderr, stderr_writer) = io::pipe()?;
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(stderr_writer);
    let gate = signals::prepare(&mut command)?;

    thread::scope(|scope| {
        let test = scope.spawn(move || -> Result<_> {
            let child = command.spawn();
            drop(command); // Release our copy of the stderr writer, including on a spawn error.
            let mut child =
                child.with_context(|| format!("starting {}", executable.to_string_lossy()))?;
            let pipe = child.stdout.take().expect("stdout is piped");
            let stdout = capture(pipe, io::stdout().lock());
            let status = child.wait().context("waiting for test")?;
            Ok((status, stdout.context("capturing stdout")?))
        });

        // spawn waits for exec, so answer the child here before draining stderr.
        let started = gate.release();
        let stderr = capture(stderr, io::stderr().lock());
        let (status, stdout) = test.join().expect("test thread panicked")?;
        started?;
        Ok(Execution {
            interrupted: signals::pending()?.is_some(),
            status,
            output: CapturedOutput {
                stdout,
                stderr: stderr.context("capturing stderr")?,
            },
        })
    })
}

fn capture(mut reader: impl Read, mut writer: impl Write) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 16 * 1024];
    let mut write_error = None;
    loop {
        let count = match reader.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if write_error.is_none() {
            write_error = writer
                .write_all(&buffer[..count])
                .and_then(|()| writer.flush())
                .err();
        }
        // Keep draining after a write error so the child cannot block on a full pipe.
    }
    match write_error {
        Some(error) => Err(error),
        None => Ok(bytes),
    }
}

pub fn replay(output: &CapturedOutput) -> Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(&output.stdout)?;
    stdout.flush()?;
    let mut stderr = io::stderr().lock();
    stderr.write_all(&output.stderr)?;
    stderr.flush()?;
    Ok(())
}
