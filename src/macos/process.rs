use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

pub(super) fn run_quiet(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to run `{program}`"))?;

    if status.success() {
        return Ok(());
    }

    bail!("`{program}` exited with status {status}");
}
