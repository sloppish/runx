use std::{
    fs::{self, OpenOptions},
    io::Write,
};

use directories::BaseDirs;

pub fn append(message: impl AsRef<str>) {
    let Some(base_dirs) = BaseDirs::new() else {
        return;
    };
    let root = base_dirs.config_dir().join("runx");
    if fs::create_dir_all(&root).is_err() {
        return;
    }

    let path = root.join("debug.log");
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };

    let _ = writeln!(file, "{}", message.as_ref());
}
