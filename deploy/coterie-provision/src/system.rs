use anyhow::{Context, Result};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn ok(stdout: impl Into<String>) -> Self {
        Self {
            status: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    pub fn fail(status: i32, stderr: impl Into<String>) -> Self {
        Self {
            status,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    pub fn succeeded(&self) -> bool {
        self.status == 0
    }
}

pub trait SystemCommand {
    fn run(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput>;
    fn run_with_stdin(&self, cmd: &str, args: &[&str], stdin: &[u8]) -> Result<CommandOutput>;
    fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput>;
}

pub struct RealSystem;

impl SystemCommand for RealSystem {
    fn run(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
        let output = Command::new(cmd)
            .args(args)
            .output()
            .with_context(|| format!("failed to spawn {cmd}"))?;
        Ok(CommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn run_with_stdin(&self, cmd: &str, args: &[&str], stdin: &[u8]) -> Result<CommandOutput> {
        use std::io::Write;

        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn {cmd}"))?;
        {
            let mut child_stdin = child
                .stdin
                .take()
                .context("failed to capture child stdin")?;
            child_stdin
                .write_all(stdin)
                .context("failed to write to child stdin")?;
        }
        let output = child
            .wait_with_output()
            .with_context(|| format!("failed to wait on {cmd}"))?;
        Ok(CommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
        let status = Command::new(cmd)
            .args(args)
            .status()
            .with_context(|| format!("failed to spawn {cmd}"))?;
        Ok(CommandOutput {
            status: status.code().unwrap_or(-1),
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}
