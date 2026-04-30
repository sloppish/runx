//! Tiny local IPC used by the standalone Settings app to tell the launcher to reload config.

use std::{
    env, fs, io,
    os::unix::fs::FileTypeExt,
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
    thread,
};

use anyhow::{Context, Result, bail};
use tao::event_loop::EventLoopProxy;

use crate::types::AppEvent;

const RELOAD_MESSAGE: &[u8] = b"reload";

pub(crate) fn start_listener(proxy: EventLoopProxy<AppEvent>) -> Result<()> {
    let socket_path = socket_path()?;
    remove_stale_socket(&socket_path)?;
    let socket = UnixDatagram::bind(&socket_path)
        .with_context(|| format!("failed to bind {}", socket_path.display()))?;

    thread::Builder::new()
        .name("runx-config-reload-ipc".to_owned())
        .spawn(move || {
            let mut buffer = [0_u8; 64];
            loop {
                match socket.recv(&mut buffer) {
                    Ok(size) if &buffer[..size] == RELOAD_MESSAGE => {
                        let _ = proxy.send_event(AppEvent::ReloadConfig);
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        })
        .context("failed to start the config reload IPC listener")?;

    Ok(())
}

pub(crate) fn notify_reload() -> Result<()> {
    let socket_path = socket_path()?;
    let socket = UnixDatagram::unbound().context("failed to create config reload IPC socket")?;
    socket
        .send_to(RELOAD_MESSAGE, &socket_path)
        .with_context(|| format!("failed to notify {}", socket_path.display()))?;
    Ok(())
}

fn socket_path() -> Result<PathBuf> {
    let dir = env::temp_dir().join("runx");
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    Ok(dir.join("reload.sock"))
}

fn remove_stale_socket(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path)
            .with_context(|| format!("failed to remove stale {}", path.display())),
        Ok(_) => bail!("{} exists and is not a socket", path.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::socket_path;

    #[test]
    fn places_reload_socket_in_temp_dir() {
        let path = socket_path().expect("path should be derived");

        assert_eq!(path.file_name().unwrap(), "reload.sock");
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "runx");
    }
}
