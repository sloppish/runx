//! Process logging setup and fatal-error persistence.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    panic::{self, PanicHookInfo},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};

use anyhow::Error;
use directories::BaseDirs;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tracing_subscriber::{
    EnvFilter,
    fmt::{self, format::FmtSpan, time::UtcTime, writer::MakeWriter},
    prelude::*,
    util::SubscriberInitExt,
};

const RUNX_LOG_ENV: &str = "RUNX_LOG";
const DEFAULT_FILTER: &str = "runx=info";

static LOG_WRITER: OnceLock<Option<Mutex<BufWriter<File>>>> = OnceLock::new();

pub fn install_panic_hook() {
    panic::set_hook(Box::new(|info| {
        write_panic(info);
    }));
}

pub fn init_launcher_logging(debug_log_enabled: bool) {
    init_launcher_logging_in(
        runtime_log_dir(),
        debug_log_enabled,
        std::env::var(RUNX_LOG_ENV).ok(),
    );
}

pub fn fatal(message: impl AsRef<str>) {
    write_fatal_line(&format!("{} FATAL {}", timestamp(), message.as_ref()));
}

pub fn fatal_error(context: &str, error: &Error) {
    fatal(format!("{context}: {error:#}"));
}

fn init_launcher_logging_in(
    log_dir: Option<PathBuf>,
    debug_log_enabled: bool,
    runx_log: Option<String>,
) {
    let writer = log_dir.and_then(|dir| open_rotated_log_file(&dir).map(BufWriter::new));
    let _ = LOG_WRITER.set(writer.map(Mutex::new));

    let log_config = LogConfig::from_runx_log(debug_log_enabled, runx_log.as_deref());
    if !log_config.attach_file_layer {
        return;
    }

    if LOG_WRITER.get().and_then(Option::as_ref).is_none() {
        eprintln!("Runx logging: debug.log is unavailable; file logging disabled");
        return;
    }

    let timer = UtcTime::new(Rfc3339);
    let layer = fmt::layer()
        .compact()
        .with_ansi(false)
        .with_span_events(FmtSpan::NONE)
        .with_timer(timer)
        .with_writer(SharedLogWriter);

    if let Err(error) = tracing_subscriber::registry()
        .with(log_config.filter)
        .with(layer)
        .try_init()
    {
        eprintln!("Runx logging: tracing subscriber already initialized: {error}");
    }
}

fn runtime_log_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|base_dirs| base_dirs.config_dir().join("runx"))
}

fn open_rotated_log_file(root: &Path) -> Option<File> {
    fs::create_dir_all(root).ok()?;
    let log_path = root.join("debug.log");
    let old_path = root.join("debug.log.old");
    if log_path.exists() {
        let _ = fs::remove_file(&old_path);
        if fs::rename(&log_path, &old_path).is_err() {
            return None;
        }
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .ok()
}

fn write_panic(info: &PanicHookInfo<'_>) {
    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string panic payload>");
    let location = info.location().map_or_else(
        || "<unknown location>".to_owned(),
        |location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        },
    );
    write_fatal_line(&format!("{} PANIC {payload} at {location}", timestamp()));
}

fn write_fatal_line(line: &str) {
    if let Some(Some(writer)) = LOG_WRITER.get() {
        match writer.try_lock() {
            Ok(mut guard) => {
                let _ = writeln!(guard, "{line}");
                let _ = guard.flush();
                return;
            }
            Err(_) => {
                eprintln!("{line}");
                return;
            }
        }
    }
    eprintln!("{line}");
}

fn timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "0000-00-00T00:00:00Z".to_owned())
}

struct LogConfig {
    attach_file_layer: bool,
    filter: EnvFilter,
}

impl LogConfig {
    fn from_runx_log(debug_log_enabled: bool, runx_log: Option<&str>) -> Self {
        match runx_log {
            Some(value) => match EnvFilter::try_new(value) {
                Ok(filter) => Self {
                    attach_file_layer: true,
                    filter,
                },
                Err(error) => {
                    eprintln!(
                        "Runx logging: invalid {RUNX_LOG_ENV}={value:?}: {error}; using {DEFAULT_FILTER}"
                    );
                    Self {
                        attach_file_layer: true,
                        filter: EnvFilter::new(DEFAULT_FILTER),
                    }
                }
            },
            None => Self {
                attach_file_layer: debug_log_enabled,
                filter: EnvFilter::new(DEFAULT_FILTER),
            },
        }
    }
}

struct SharedLogWriter;

impl<'writer> MakeWriter<'writer> for SharedLogWriter {
    type Writer = SharedLogWriterGuard<'writer>;

    fn make_writer(&'writer self) -> Self::Writer {
        let guard = LOG_WRITER
            .get()
            .and_then(Option::as_ref)
            .and_then(|writer| writer.lock().ok());
        SharedLogWriterGuard { guard }
    }
}

struct SharedLogWriterGuard<'writer> {
    guard: Option<MutexGuard<'writer, BufWriter<File>>>,
}

impl Write for SharedLogWriterGuard<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.guard {
            Some(guard) => guard.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.guard {
            Some(guard) => guard.flush(),
            None => Ok(()),
        }
    }
}

impl Drop for SharedLogWriterGuard<'_> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{LOG_WRITER, LogConfig, fatal, init_launcher_logging_in, open_rotated_log_file};

    #[test]
    fn disabled_config_without_runx_log_does_not_attach_file_layer() {
        let config = LogConfig::from_runx_log(false, None);
        assert!(!config.attach_file_layer);
    }

    #[test]
    fn enabled_config_uses_default_filter() {
        let config = LogConfig::from_runx_log(true, None);
        assert!(config.attach_file_layer);
    }

    #[test]
    fn runx_log_forces_file_layer() {
        let config = LogConfig::from_runx_log(false, Some("debug"));
        assert!(config.attach_file_layer);
    }

    #[test]
    fn invalid_runx_log_still_attaches_file_layer() {
        let config = LogConfig::from_runx_log(false, Some("not a valid filter ???"));
        assert!(config.attach_file_layer);
    }

    #[test]
    fn rotation_overwrites_old_generation() {
        let dir = tempdir().expect("tempdir should be available");
        fs::write(dir.path().join("debug.log"), "current").expect("debug log should be writable");
        fs::write(dir.path().join("debug.log.old"), "old")
            .expect("old debug log should be writable");

        let _file = open_rotated_log_file(dir.path()).expect("log file should open");

        assert_eq!(
            fs::read_to_string(dir.path().join("debug.log.old")).expect("old log should exist"),
            "current"
        );
    }

    #[test]
    fn open_failure_degrades_to_no_writer() {
        let dir = tempdir().expect("tempdir should be available");
        let not_a_directory = dir.path().join("not-a-directory");
        fs::write(&not_a_directory, "file").expect("test file should be writable");

        assert!(open_rotated_log_file(&not_a_directory).is_none());
    }

    #[test]
    fn fatal_before_phase_two_does_not_block_later_file_logging() {
        let dir = tempdir().expect("tempdir should be available");
        fatal("before phase two");
        assert!(LOG_WRITER.get().is_none());

        init_launcher_logging_in(Some(dir.path().to_owned()), true, None);
        tracing::info!(target: "runx::logging::tests", "after init");

        let log = fs::read_to_string(dir.path().join("debug.log")).expect("debug log should exist");
        assert!(log.contains("after init"));
    }
}
