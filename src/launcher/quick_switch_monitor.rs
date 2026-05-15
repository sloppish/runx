use std::ptr::NonNull;

use global_hotkey::hotkey::Code;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};
use tao::event_loop::EventLoopProxy;

use crate::types::AppEvent;

pub struct QuickSwitchMonitor {
    _local: Retained<AnyObject>,
}

impl QuickSwitchMonitor {
    pub fn install(proxy: EventLoopProxy<AppEvent>, key_code: u16) -> Option<Self> {
        let mask = NSEventMask::KeyDown | NSEventMask::FlagsChanged;
        let block = block2::RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            let event_ref = unsafe { event.as_ref() };
            let flags = event_ref.modifierFlags();
            if !flags.contains(NSEventModifierFlags::Option) {
                let _ = proxy.send_event(AppEvent::QuickSwitchModifierReleased);
                return event.as_ptr();
            }
            if event_ref.keyCode() == key_code {
                let delta = if flags.contains(NSEventModifierFlags::Shift) {
                    -1
                } else {
                    1
                };
                let _ = proxy.send_event(AppEvent::QuickSwitchTabCycle { delta });
                return std::ptr::null_mut();
            }
            event.as_ptr()
        });
        let monitor =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) }?;
        Some(Self { _local: monitor })
    }
}

/// Maps a `keyboard_types::Code` to a macOS virtual key code (`u16`).
/// Mirrors the mapping in `global-hotkey`'s internal `key_to_scancode`.
pub fn code_to_macos_keycode(code: Code) -> Option<u16> {
    let keycode: u16 = match code {
        Code::KeyA => 0x00,
        Code::KeyS => 0x01,
        Code::KeyD => 0x02,
        Code::KeyF => 0x03,
        Code::KeyH => 0x04,
        Code::KeyG => 0x05,
        Code::KeyZ => 0x06,
        Code::KeyX => 0x07,
        Code::KeyC => 0x08,
        Code::KeyV => 0x09,
        Code::KeyB => 0x0b,
        Code::KeyQ => 0x0c,
        Code::KeyW => 0x0d,
        Code::KeyE => 0x0e,
        Code::KeyR => 0x0f,
        Code::KeyY => 0x10,
        Code::KeyT => 0x11,
        Code::Digit1 => 0x12,
        Code::Digit2 => 0x13,
        Code::Digit3 => 0x14,
        Code::Digit4 => 0x15,
        Code::Digit6 => 0x16,
        Code::Digit5 => 0x17,
        Code::Equal => 0x18,
        Code::Digit9 => 0x19,
        Code::Digit7 => 0x1a,
        Code::Minus => 0x1b,
        Code::Digit8 => 0x1c,
        Code::Digit0 => 0x1d,
        Code::BracketRight => 0x1e,
        Code::KeyO => 0x1f,
        Code::KeyU => 0x20,
        Code::BracketLeft => 0x21,
        Code::KeyI => 0x22,
        Code::KeyP => 0x23,
        Code::Enter => 0x24,
        Code::KeyL => 0x25,
        Code::KeyJ => 0x26,
        Code::Quote => 0x27,
        Code::KeyK => 0x28,
        Code::Semicolon => 0x29,
        Code::Backslash => 0x2a,
        Code::Comma => 0x2b,
        Code::Slash => 0x2c,
        Code::KeyN => 0x2d,
        Code::KeyM => 0x2e,
        Code::Period => 0x2f,
        Code::Tab => 0x30,
        Code::Space => 0x31,
        Code::Backquote => 0x32,
        Code::Backspace => 0x33,
        Code::Escape => 0x35,
        Code::CapsLock => 0x39,
        Code::F17 => 0x40,
        Code::NumpadDecimal => 0x41,
        Code::NumpadMultiply => 0x43,
        Code::NumpadAdd => 0x45,
        Code::NumLock => 0x47,
        Code::AudioVolumeUp => 0x48,
        Code::AudioVolumeDown => 0x49,
        Code::AudioVolumeMute => 0x4a,
        Code::NumpadDivide => 0x4b,
        Code::NumpadEnter => 0x4c,
        Code::NumpadSubtract => 0x4e,
        Code::F18 => 0x4f,
        Code::F19 => 0x50,
        Code::NumpadEqual => 0x51,
        Code::Numpad0 => 0x52,
        Code::Numpad1 => 0x53,
        Code::Numpad2 => 0x54,
        Code::Numpad3 => 0x55,
        Code::Numpad4 => 0x56,
        Code::Numpad5 => 0x57,
        Code::Numpad6 => 0x58,
        Code::Numpad7 => 0x59,
        Code::F20 => 0x5a,
        Code::Numpad8 => 0x5b,
        Code::Numpad9 => 0x5c,
        Code::F5 => 0x60,
        Code::F6 => 0x61,
        Code::F7 => 0x62,
        Code::F3 => 0x63,
        Code::F8 => 0x64,
        Code::F9 => 0x65,
        Code::F11 => 0x67,
        Code::F13 => 0x69,
        Code::F16 => 0x6a,
        Code::F14 => 0x6b,
        Code::F10 => 0x6d,
        Code::F12 => 0x6f,
        Code::F15 => 0x71,
        Code::Insert => 0x72,
        Code::Home => 0x73,
        Code::PageUp => 0x74,
        Code::Delete => 0x75,
        Code::F4 => 0x76,
        Code::End => 0x77,
        Code::F2 => 0x78,
        Code::PageDown => 0x79,
        Code::F1 => 0x7a,
        Code::ArrowLeft => 0x7b,
        Code::ArrowRight => 0x7c,
        Code::ArrowDown => 0x7d,
        Code::ArrowUp => 0x7e,
        Code::PrintScreen => 0x46,
        _ => return None,
    };
    Some(keycode)
}

impl Drop for QuickSwitchMonitor {
    fn drop(&mut self) {
        unsafe { NSEvent::removeMonitor(&self._local) };
    }
}
