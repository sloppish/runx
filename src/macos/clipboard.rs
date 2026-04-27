use std::{thread, time::Duration};

use anyhow::{Context, Result, bail};
use core_graphics::{
    event::{CGEvent, CGEventFlags, CGEventTapLocation, KeyCode},
    event_source::{CGEventSource, CGEventSourceStateID},
};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;

use crate::debug_log;

use super::{
    permissions::{ensure_accessibility_trusted, open_accessibility_settings},
    types::FrontmostApp,
};

const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(250);

/// Writes plain text to the macOS clipboard using the native pasteboard API.
pub fn copy_text_to_clipboard(text: &str) -> Result<String> {
    write_clipboard_text(text)?;
    Ok("Copied to clipboard".to_owned())
}

/// Reads plain text from the macOS clipboard.
pub fn read_clipboard_text() -> Result<String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let text = pasteboard
        .stringForType(pasteboard_type_string())
        .ok_or_else(|| anyhow::anyhow!("clipboard does not currently contain plain text"))?;
    Ok(text.to_string())
}

/// Types text into the app that was focused before Runx was shown.
pub fn type_text_into_previous_app(
    text: &str,
    previous_app: Option<&FrontmostApp>,
) -> Result<String> {
    debug_log::append(format!(
        "type_text_into_previous_app start len={} previous_app={:?}",
        text.chars().count(),
        previous_app
    ));
    if !ensure_accessibility_trusted(true) {
        debug_log::append("type_text_into_previous_app accessibility preflight returned false");
        open_accessibility_settings();
        bail!(
            "Runx needs Accessibility permission to type into other apps. Approve the system prompt or enable your terminal/runx in System Settings > Privacy & Security > Accessibility, then retry."
        );
    }

    if requires_clipboard_paste(text) {
        debug_log::append("type_text_into_previous_app using clipboard paste path");
        return paste_text_into_previous_app(text);
    }

    post_keyboard_text(text).context("failed to post keyboard events for text typing")?;
    Ok("Typed into the previous app".to_owned())
}

fn paste_text_into_previous_app(text: &str) -> Result<String> {
    let previous_clipboard = read_clipboard_text().ok();
    write_clipboard_text(text)?;
    post_command_v().context("failed to post keyboard events for clipboard paste")?;
    schedule_clipboard_restore(previous_clipboard, text.to_owned());
    Ok("Typed into the previous app".to_owned())
}

fn schedule_clipboard_restore(previous_clipboard: Option<String>, inserted_text: String) {
    let Some(previous_clipboard) = previous_clipboard else {
        return;
    };

    thread::spawn(move || {
        thread::sleep(CLIPBOARD_RESTORE_DELAY);

        match read_clipboard_text() {
            Ok(current) if current == inserted_text => {
                if let Err(error) = write_clipboard_text(&previous_clipboard) {
                    debug_log::append(format!("clipboard restore failed after paste: {error:#}"));
                }
            }
            Ok(_) => {}
            Err(error) => debug_log::append(format!(
                "clipboard restore skipped after paste; could not read clipboard: {error:#}"
            )),
        }
    });
}

pub(super) fn requires_clipboard_paste(text: &str) -> bool {
    !text.is_ascii()
}

fn write_clipboard_text(text: &str) -> Result<()> {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    let text = NSString::from_str(text);
    if pasteboard.setString_forType(&text, pasteboard_type_string()) {
        return Ok(());
    }
    bail!("failed to write plain text to the macOS pasteboard")
}

fn pasteboard_type_string() -> &'static objc2_app_kit::NSPasteboardType {
    // SAFETY: Apple exports `NSPasteboardTypeString` as a process-global constant.
    unsafe { NSPasteboardTypeString }
}

fn post_keyboard_text(text: &str) -> Result<()> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| anyhow::anyhow!("failed to create Quartz event source"))?;
    let key_down = CGEvent::new_keyboard_event(source.clone(), 0, true)
        .map_err(|_| anyhow::anyhow!("failed to create key-down event"))?;
    key_down.set_string(text);
    key_down.post(CGEventTapLocation::HID);

    let key_up = CGEvent::new_keyboard_event(source, 0, false)
        .map_err(|_| anyhow::anyhow!("failed to create key-up event"))?;
    key_up.set_string(text);
    key_up.post(CGEventTapLocation::HID);
    Ok(())
}

fn post_command_v() -> Result<()> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| anyhow::anyhow!("failed to create Quartz event source"))?;

    let command_down = CGEvent::new_keyboard_event(source.clone(), KeyCode::COMMAND, true)
        .map_err(|_| anyhow::anyhow!("failed to create command key-down event"))?;
    command_down.set_flags(CGEventFlags::CGEventFlagCommand);
    command_down.post(CGEventTapLocation::HID);

    let v_down = CGEvent::new_keyboard_event(source.clone(), KeyCode::ANSI_V, true)
        .map_err(|_| anyhow::anyhow!("failed to create V key-down event"))?;
    v_down.set_flags(CGEventFlags::CGEventFlagCommand);
    v_down.post(CGEventTapLocation::HID);

    let v_up = CGEvent::new_keyboard_event(source.clone(), KeyCode::ANSI_V, false)
        .map_err(|_| anyhow::anyhow!("failed to create V key-up event"))?;
    v_up.set_flags(CGEventFlags::CGEventFlagCommand);
    v_up.post(CGEventTapLocation::HID);

    let command_up = CGEvent::new_keyboard_event(source, KeyCode::COMMAND, false)
        .map_err(|_| anyhow::anyhow!("failed to create command key-up event"))?;
    command_up.set_flags(CGEventFlags::CGEventFlagNull);
    command_up.post(CGEventTapLocation::HID);
    Ok(())
}
