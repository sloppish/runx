//! Tiny local IPC used by the standalone Settings app to tell the launcher to reload config.

use std::{
    fs, io,
    os::unix::fs::FileTypeExt,
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
    thread,
};

use anyhow::{Context, Result, bail};
use tao::event_loop::EventLoopProxy;

use crate::types::AppEvent;

const SOCKET_FILE_NAME: &str = "reload.sock";
const RELOAD_MESSAGE: &[u8] = b"reload";

pub(crate) fn start_listener(config_path: &Path, proxy: EventLoopProxy<AppEvent>) -> Result<()> {
    let socket_path = socket_path(config_path)?;
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

pub(crate) fn notify_reload(config_path: &Path) -> Result<()> {
    let socket_path = socket_path(config_path)?;
    let socket = UnixDatagram::unbound().context("failed to create config reload IPC socket")?;
    socket
        .send_to(RELOAD_MESSAGE, &socket_path)
        .with_context(|| format!("failed to notify {}", socket_path.display()))?;
    Ok(())
}

fn socket_path(config_path: &Path) -> Result<PathBuf> {
    let Some(config_dir) = config_path.parent() else {
        bail!(
            "config path has no parent directory: {}",
            config_path.display()
        );
    };
    Ok(config_dir.join(SOCKET_FILE_NAME))
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
    fn places_reload_socket_next_to_config() {
        let path = socket_path(std::path::Path::new("/tmp/runx/config.toml"))
            .expect("path should be derived");

        assert_eq!(path, std::path::Path::new("/tmp/runx/reload.sock"));
    }
}
