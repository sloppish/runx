//! Minimal file-based debug logging used during macOS packaging and launch triage.

use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    sync::{Mutex, OnceLock},
};

use directories::BaseDirs;

static LOG_WRITER: OnceLock<Option<Mutex<BufWriter<File>>>> = OnceLock::new();

fn open_log_file() -> Option<File> {
    let base_dirs = BaseDirs::new()?;
    let root = base_dirs.config_dir().join("runx");
    fs::create_dir_all(&root).ok()?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("debug.log"))
        .ok()
}

/// Appends a line to `~/Library/Application Support/runx/debug.log`.
pub fn append(message: impl AsRef<str>) {
    let Some(writer) = LOG_WRITER.get_or_init(|| {
        open_log_file().map(|file| Mutex::new(BufWriter::new(file)))
    }) else {
        return;
    };

    if let Ok(mut guard) = writer.lock() {
        let _ = writeln!(guard, "{}", message.as_ref());
    }
}
