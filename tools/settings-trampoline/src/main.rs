use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("runx-settings"));
    // Navigate from Contents/Applications/Runx Settings.app/Contents/MacOS/runx-settings
    // up to Contents/MacOS/runx
    let runx = exe
        .parent() // MacOS/
        .and_then(|p| p.parent()) // Contents/
        .and_then(|p| p.parent()) // Runx Settings.app/
        .and_then(|p| p.parent()) // Applications/
        .and_then(|p| p.parent()) // Contents/
        .map(|p| p.join("MacOS").join("runx"));

    let Some(runx) = runx.filter(|p| p.is_file()) else {
        eprintln!("settings-trampoline: cannot locate runx binary");
        std::process::exit(1);
    };

    let err = Command::new(&runx).arg("--settings").exec();
    eprintln!("settings-trampoline: exec failed: {err}");
    std::process::exit(1);
}
