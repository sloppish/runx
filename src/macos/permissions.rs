use core_foundation::{
    base::TCFType, boolean::CFBoolean, dictionary::CFDictionary, string::CFString,
};
use core_foundation_sys::{base::Boolean, dictionary::CFDictionaryRef, string::CFStringRef};
use objc2_foundation::{NSOperatingSystemVersion, NSProcessInfo};

use super::{private_apis::core_graphics_apis, process::run_quiet};

const PRIVACY_ACCESSIBILITY: &str = "Privacy_Accessibility";
const PRIVACY_SCREEN_CAPTURE: &str = "Privacy_ScreenCapture";

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> Boolean;
}

/// Returns whether the current process is trusted for Accessibility APIs.
pub fn ensure_accessibility_trusted(prompt: bool) -> bool {
    let prompt_key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let prompt_value = if prompt {
        CFBoolean::true_value()
    } else {
        CFBoolean::false_value()
    };
    let options = CFDictionary::from_CFType_pairs(&[(prompt_key, prompt_value)]);

    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) != 0 }
}

/// Returns whether the current process is trusted for Screen Recording APIs.
pub fn ensure_screen_recording_trusted(prompt: bool) -> bool {
    if !screen_recording_permission_supported() {
        return true;
    }

    let Some(apis) = core_graphics_apis() else {
        return false;
    };

    if unsafe { (apis.preflight_screen_capture_access)() } {
        return true;
    }

    if !prompt {
        return false;
    }

    let trusted = unsafe { (apis.request_screen_capture_access)() };
    if !trusted {
        open_screen_recording_settings();
    }
    trusted
}

pub(super) fn open_accessibility_settings() {
    open_privacy_settings(PRIVACY_ACCESSIBILITY);
}

pub(super) fn open_screen_recording_settings() {
    open_privacy_settings(PRIVACY_SCREEN_CAPTURE);
}

fn screen_recording_permission_supported() -> bool {
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: 10,
        minorVersion: 15,
        patchVersion: 0,
    })
}

fn open_privacy_settings(anchor: &str) {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
    let _ = run_quiet("open", &[url.as_str()]);
}
