//! Menu bar tray integration.
//!
//! This module owns the status-item icon and its small menu, then translates
//! tray interactions back into [`crate::types::AppEvent`] values.

use std::{fs::File, io::BufReader, path::Path};

use anyhow::{Context, Result, bail};
use png::{ColorType, Decoder, Transformations};
use tao::event_loop::EventLoopProxy;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem},
};

use crate::{assets::tray_icon_path, macos, types::AppEvent};
use tracing::warn;

const TRAY_ID: &str = "runx-tray";
const MENU_OPEN_ID: &str = "tray-open";
const MENU_SETTINGS_ID: &str = "tray-settings";
const MENU_AUTOSTART_ID: &str = "tray-autostart";
const MENU_CHECK_UPDATES_ID: &str = "tray-check-updates";
const MENU_SPONSOR_ID: &str = "tray-sponsor";
const MENU_QUIT_ID: &str = "tray-quit";

/// Holds the tray icon and menu resources for the lifetime of the app.
pub struct TrayState {
    _tray: TrayIcon,
    _menu: Menu,
    _open_item: MenuItem,
    _settings_item: MenuItem,
    autostart_item: CheckMenuItem,
    _check_updates_item: MenuItem,
    _sponsor_item: MenuItem,
    _quit_item: MenuItem,
}

impl TrayState {
    /// Installs the menu bar item and wires it to the app event loop.
    pub fn install(proxy: EventLoopProxy<AppEvent>) -> Result<Self> {
        let menu = Menu::new();
        let autostart_supported = macos::current_app_supports_login_item();
        let autostart_enabled = if autostart_supported {
            match macos::current_app_login_item_enabled() {
                Ok(Some(enabled)) => enabled,
                Ok(None) => false,
                Err(error) => {
                    warn!(
                        error = %format!("{error:#}"),
                        "tray install could not read launch at login status"
                    );
                    false
                }
            }
        } else {
            false
        };
        let open_item = MenuItem::with_id(MENU_OPEN_ID, "Open Runx", true, None);
        let settings_item = MenuItem::with_id(MENU_SETTINGS_ID, "Settings", true, None);
        let autostart_item = CheckMenuItem::with_id(
            MENU_AUTOSTART_ID,
            "Launch at Login",
            autostart_supported,
            autostart_enabled,
            None,
        );
        let check_updates_item =
            MenuItem::with_id(MENU_CHECK_UPDATES_ID, "Check for Updates…", true, None);
        let sponsor_item = MenuItem::with_id(MENU_SPONSOR_ID, "Sponsor", true, None);
        let quit_item = MenuItem::with_id(MENU_QUIT_ID, "Quit Runx", true, None);
        menu.append_items(&[
            &open_item,
            &settings_item,
            &autostart_item,
            &check_updates_item,
            &sponsor_item,
            &quit_item,
        ])
        .context("failed to build the tray menu")?;

        let open_id = open_item.id().clone();
        let settings_id = settings_item.id().clone();
        let autostart_id = autostart_item.id().clone();
        let check_updates_id = check_updates_item.id().clone();
        let sponsor_id = sponsor_item.id().clone();
        let quit_id = quit_item.id().clone();
        MenuEvent::set_event_handler(Some({
            let proxy = proxy.clone();
            move |event: MenuEvent| {
                let app_event = if event.id == open_id {
                    Some(AppEvent::TrayOpen)
                } else if event.id == settings_id {
                    Some(AppEvent::TraySettings)
                } else if event.id == autostart_id {
                    Some(AppEvent::TrayToggleAutostart)
                } else if event.id == check_updates_id {
                    Some(AppEvent::TrayCheckForUpdates)
                } else if event.id == sponsor_id {
                    Some(AppEvent::TraySponsor)
                } else if event.id == quit_id {
                    Some(AppEvent::Quit)
                } else {
                    None
                };
                if let Some(event) = app_event {
                    let _ = proxy.send_event(event);
                }
            }
        }));

        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let _ = proxy.send_event(AppEvent::TrayToggle);
            }
        }));

        let icon_path = tray_icon_path()?;
        let icon = load_icon(&icon_path)?;
        let tray = TrayIconBuilder::new()
            .with_id(TRAY_ID)
            .with_icon(icon)
            .with_icon_as_template(true)
            .with_tooltip("Runx")
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(false)
            .build()
            .context("failed to create the tray icon")?;

        Ok(Self {
            _tray: tray,
            _menu: menu,
            _open_item: open_item,
            _settings_item: settings_item,
            autostart_item,
            _check_updates_item: check_updates_item,
            _sponsor_item: sponsor_item,
            _quit_item: quit_item,
        })
    }

    /// Updates the checked state of the Launch at Login menu item.
    pub fn set_autostart_enabled(&self, enabled: bool) {
        self.autostart_item.set_checked(enabled);
    }
}

fn load_icon(path: &Path) -> Result<Icon> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut decoder = Decoder::new(BufReader::new(file));
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .with_context(|| format!("failed to decode {}", path.display()))?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .with_context(|| format!("failed to read icon data from {}", path.display()))?;
    let pixels = &buffer[..info.buffer_size()];
    let rgba = rgba_pixels(pixels, info.color_type)?;
    Icon::from_rgba(rgba, info.width, info.height)
        .with_context(|| format!("failed to build tray icon from {}", path.display()))
}

fn rgba_pixels(pixels: &[u8], color_type: ColorType) -> Result<Vec<u8>> {
    let rgba = match color_type {
        ColorType::Rgba => pixels.to_vec(),
        ColorType::Rgb => pixels
            .chunks_exact(3)
            .flat_map(|chunk| [chunk[0], chunk[1], chunk[2], 255])
            .collect(),
        ColorType::Grayscale => pixels
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect(),
        ColorType::GrayscaleAlpha => pixels
            .chunks_exact(2)
            .flat_map(|chunk| [chunk[0], chunk[0], chunk[0], chunk[1]])
            .collect(),
        ColorType::Indexed => bail!("indexed PNG tray icons are not supported"),
    };
    Ok(rgba)
}
