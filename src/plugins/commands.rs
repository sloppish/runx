use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};

#[derive(Debug)]
pub(crate) struct SearchCancelled;

impl std::fmt::Display for SearchCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("search cancelled")
    }
}

impl std::error::Error for SearchCancelled {}

pub(super) fn parse_shell_args(raw: &str) -> Result<Vec<String>> {
    #[derive(Copy, Clone, Eq, PartialEq)]
    enum Mode {
        Unquoted,
        SingleQuoted,
        DoubleQuoted,
    }

    let mut mode = Mode::Unquoted;
    let mut escaped = false;
    let mut current = String::new();
    let mut args = Vec::new();

    for ch in raw.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        match mode {
            Mode::Unquoted => match ch {
                '\\' => escaped = true,
                '\'' => mode = Mode::SingleQuoted,
                '"' => mode = Mode::DoubleQuoted,
                ch if ch.is_whitespace() => {
                    if !current.is_empty() {
                        args.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(ch),
            },
            Mode::SingleQuoted => match ch {
                '\'' => mode = Mode::Unquoted,
                _ => current.push(ch),
            },
            Mode::DoubleQuoted => match ch {
                '\\' => escaped = true,
                '"' => mode = Mode::Unquoted,
                _ => current.push(ch),
            },
        }
    }

    if escaped {
        bail!("unterminated escape sequence in command arguments");
    }

    match mode {
        Mode::Unquoted => {}
        Mode::SingleQuoted => bail!("unterminated single-quoted string in command arguments"),
        Mode::DoubleQuoted => bail!("unterminated double-quoted string in command arguments"),
    }

    if !current.is_empty() {
        args.push(current);
    }

    Ok(args)
}

pub(super) fn walk_files(root: &Path) -> Result<Vec<String>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    walk_directory(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk_directory(root: &Path, current: &Path, files: &mut Vec<String>) -> Result<()> {
    for entry in
        fs::read_dir(current).with_context(|| format!("failed to read {}", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_directory(root, &path, files)?;
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("failed to relativize {}", path.display()))?;
        files.push(relative.to_string_lossy().to_string());
    }

    Ok(())
}

pub(super) fn exec_capture(
    program: &str,
    args: &[String],
    first_line_only: bool,
    trim: bool,
    search_paths: &[PathBuf],
    cancel: &AtomicBool,
) -> Result<String> {
    let mut child = command_for_plugin(program, search_paths)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run `{program}`"))?;

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SearchCancelled.into());
        }
        match child
            .try_wait()
            .context("failed to wait on child process")?
        {
            Some(_) => break,
            None => thread::sleep(Duration::from_millis(1)),
        }
    }

    let output = child
        .wait_with_output()
        .with_context(|| format!("failed to read output from `{program}`"))?;

    if !output.status.success() {
        bail_command_failure(program, &output.stderr, output.status)?;
    }

    let stdout = String::from_utf8(output.stdout).context("command output was not UTF-8")?;
    let text = if first_line_only {
        let line = stdout.lines().next().unwrap_or_default();
        if trim {
            line.trim().to_owned()
        } else {
            line.to_owned()
        }
    } else if trim {
        stdout.trim().to_owned()
    } else {
        stdout
    };

    Ok(text)
}

pub(super) fn exec_status(
    program: &str,
    args: &[String],
    silence_stderr: bool,
    search_paths: &[PathBuf],
    cancel: &AtomicBool,
) -> Result<()> {
    let mut command = command_for_plugin(program, search_paths);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    if silence_stderr {
        command.stderr(Stdio::null());
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to run `{program}`"))?;

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SearchCancelled.into());
        }
        match child
            .try_wait()
            .context("failed to wait on child process")?
        {
            Some(_) => break,
            None => thread::sleep(Duration::from_millis(1)),
        }
    }

    let status = child
        .wait()
        .with_context(|| format!("failed to wait on `{program}`"))?;
    if status.success() {
        return Ok(());
    }

    bail!("underlying command exited {status}");
}

fn bail_command_failure(program: &str, stderr: &[u8], status: ExitStatus) -> Result<()> {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.is_empty() {
        bail!("`{program}` exited with status {status}");
    }
    bail!("{message}");
}

fn command_for_plugin(program: &str, search_paths: &[PathBuf]) -> Command {
    let mut command = Command::new(program);
    command.env("PATH", plugin_search_path(search_paths));
    command
}

const IMPLICIT_SEARCH_PATHS: &[&str] = &["/opt/homebrew/bin"];

fn plugin_search_path(search_paths: &[PathBuf]) -> OsString {
    let mut paths = match env::var_os("PATH") {
        Some(value) => env::split_paths(&value).collect::<Vec<_>>(),
        None => Vec::new(),
    };

    for path in IMPLICIT_SEARCH_PATHS
        .iter()
        .map(PathBuf::from)
        .chain(search_paths.iter().cloned())
    {
        if !paths.iter().any(|candidate| candidate == &path) {
            paths.push(path);
        }
    }

    env::join_paths(paths).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::{env, path::PathBuf};

    use super::{parse_shell_args, plugin_search_path};

    #[test]
    fn plugin_search_path_includes_configured_paths() {
        let path = plugin_search_path(&[PathBuf::from("/opt/homebrew/bin")])
            .to_string_lossy()
            .into_owned();

        assert!(path.contains("/opt/homebrew/bin"));
    }

    #[test]
    fn plugin_search_path_keeps_existing_path_entries() {
        let path = plugin_search_path(&[PathBuf::from("/opt/homebrew/bin")])
            .to_string_lossy()
            .into_owned();

        if let Some(existing) = env::var_os("PATH") {
            let existing = existing.to_string_lossy();
            if !existing.is_empty() {
                assert!(path.contains(existing.as_ref()));
            }
        }
    }

    #[test]
    fn parse_shell_args_supports_quotes_and_escapes() {
        let args = parse_shell_args(r#"1 "2 3" '4 "5"' six\ seven"#).expect("parsed args");

        assert_eq!(args, vec!["1", "2 3", r#"4 "5""#, "six seven"]);
    }

    #[test]
    fn parse_shell_args_rejects_unterminated_quotes() {
        let error = parse_shell_args(r#""unterminated"#)
            .expect_err("unterminated quotes should fail")
            .to_string();

        assert!(error.contains("unterminated double-quoted string"));
    }
}
