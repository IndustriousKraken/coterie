use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.status == 0
    }
}

pub trait SystemCommand {
    fn run(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput>;
    fn run_with_stdin(&self, cmd: &str, args: &[&str], stdin_bytes: &[u8])
        -> Result<CommandOutput>;
    fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput>;
}

pub struct RealSystem;

impl RealSystem {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RealSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemCommand for RealSystem {
    fn run(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
        let output = Command::new(cmd)
            .args(args)
            .output()
            .with_context(|| format!("failed to spawn `{cmd}`"))?;
        Ok(CommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn run_with_stdin(
        &self,
        cmd: &str,
        args: &[&str],
        stdin_bytes: &[u8],
    ) -> Result<CommandOutput> {
        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn `{cmd}`"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(stdin_bytes)
                .with_context(|| format!("failed to write stdin to `{cmd}`"))?;
        }
        let output = child
            .wait_with_output()
            .with_context(|| format!("failed to wait on `{cmd}`"))?;
        Ok(CommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
        let status = Command::new(cmd)
            .args(args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .with_context(|| format!("failed to spawn `{cmd}`"))?;
        Ok(CommandOutput {
            status: status.code().unwrap_or(-1),
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}
