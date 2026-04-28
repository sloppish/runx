use std::str::FromStr;

use anyhow::{Context, Result};
use global_hotkey::hotkey::HotKey;
use serde::{Deserialize, Deserializer};

use super::schema::UiShortcutConfig;

pub(super) fn parse_hotkey_shortcut(value: &str) -> Result<HotKey> {
    HotKey::from_str(value.trim())
        .with_context(|| format!("unsupported hotkey shortcut `{value}` in config.toml"))
}

pub(super) fn deserialize_ui_shortcut<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<UiShortcutConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    parse_ui_shortcut(&value).map_err(serde::de::Error::custom)
}

pub(super) fn parse_ui_shortcut(
    value: &str,
) -> std::result::Result<Option<UiShortcutConfig>, String> {
    let value = value.trim();
    if value.is_empty()
        || value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("disabled")
        || value.eq_ignore_ascii_case("off")
    {
        return Ok(None);
    }

    let mut shortcut = UiShortcutConfig {
        key: None,
        code: None,
        alt: false,
        ctrl: false,
        meta: false,
        shift: false,
    };

    for part in value.split('+') {
        let token = part.trim();
        if token.is_empty() {
            return Err(format!("invalid UI shortcut `{value}`"));
        }

        match token.to_ascii_lowercase().as_str() {
            "alt" | "option" => shortcut.alt = true,
            "control" | "ctrl" => shortcut.ctrl = true,
            "command" | "cmd" | "super" | "meta" => shortcut.meta = true,
            "shift" => shortcut.shift = true,
            _ => {
                if shortcut.key.is_some() || shortcut.code.is_some() {
                    return Err(format!("UI shortcut `{value}` contains more than one key"));
                }

                let (key, code) = parse_ui_shortcut_key(token)
                    .ok_or_else(|| format!("unsupported UI shortcut key `{token}`"))?;
                shortcut.key = key;
                shortcut.code = code;
            }
        }
    }

    if shortcut.key.is_none() && shortcut.code.is_none() {
        return Err(format!("UI shortcut `{value}` is missing a key"));
    }

    Ok(Some(shortcut))
}

pub(super) fn parse_ui_shortcut_key(token: &str) -> Option<(Option<String>, Option<String>)> {
    let normalized = token.trim();
    let lower = normalized.to_ascii_lowercase();
    let key = match lower.as_str() {
        "enter" | "return" => return Some((Some("Enter".to_owned()), None)),
        "escape" | "esc" => return Some((Some("Escape".to_owned()), None)),
        "tab" => return Some((Some("Tab".to_owned()), None)),
        "space" => return Some((None, Some("Space".to_owned()))),
        "backspace" => return Some((Some("Backspace".to_owned()), None)),
        "delete" => return Some((Some("Delete".to_owned()), None)),
        "up" | "arrowup" => "ArrowUp",
        "down" | "arrowdown" => "ArrowDown",
        "left" | "arrowleft" => "ArrowLeft",
        "right" | "arrowright" => "ArrowRight",
        "numpadenter" => "NumpadEnter",
        _ => {
            if lower.len() == 1 {
                let ch = lower.chars().next()?;
                if ch.is_ascii_alphabetic() {
                    return Some((None, Some(format!("Key{}", ch.to_ascii_uppercase()))));
                }
                if ch.is_ascii_digit() {
                    return Some((None, Some(format!("Digit{ch}"))));
                }
            }

            if normalized.starts_with("Key")
                || normalized.starts_with("Digit")
                || normalized.starts_with("Numpad")
                || normalized.starts_with("Arrow")
            {
                return Some((None, Some(normalized.to_owned())));
            }

            return None;
        }
    };

    Some((None, Some(key.to_owned())))
}
