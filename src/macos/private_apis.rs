use std::{
    ffi::{c_char, c_float, c_int, c_void},
    sync::OnceLock,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use core_foundation::{
    array::CFArray, base::TCFType, boolean::CFBoolean, data::CFData, string::CFString,
};
use core_foundation_sys::{
    base::{CFRelease, CFTypeRef},
    data::CFDataRef,
    string::CFStringRef,
};

const AX_MESSAGING_TIMEOUT_SECONDS: c_float = 1.0;
const REMOTE_AX_WINDOW_SCAN_BUDGET: Duration = Duration::from_millis(100);
const REMOTE_AX_WINDOW_SCAN_LIMIT: u64 = 1_000;
const REMOTE_AX_TOKEN_MAGIC: i32 = 0x636f636f;
const RTLD_LAZY: c_int = 0x1;
const RTLD_LOCAL: c_int = 0x4;
const CPS_USER_GENERATED: u32 = 0x200;
const MAKE_KEY_EVENT_BYTES: usize = 0xf8;

pub(super) type AXUIElementRef = *const c_void;
pub(super) type AXError = c_int;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct ProcessSerialNumber {
    high_long_of_psn: u32,
    low_long_of_psn: u32,
}

const K_AX_ERROR_SUCCESS: AXError = 0;
const K_AX_ERROR_CANNOT_COMPLETE: AXError = -25204;
const K_AX_ERROR_ATTRIBUTE_UNSUPPORTED: AXError = -25205;
const K_AX_ERROR_ACTION_UNSUPPORTED: AXError = -25206;
const K_AX_ERROR_API_DISABLED: AXError = -25211;
const K_AX_ERROR_NO_VALUE: AXError = -25212;

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: c_int) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    fn AXUIElementSetMessagingTimeout(
        element: AXUIElementRef,
        timeout_in_seconds: c_float,
    ) -> AXError;
    fn GetProcessForPID(pid: c_int, psn: *mut ProcessSerialNumber) -> c_int;
    fn _AXUIElementGetWindow(element: AXUIElementRef, out: *mut u32) -> AXError;
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

struct WindowServerApis {
    set_front_process_with_options:
        unsafe extern "C" fn(*mut ProcessSerialNumber, u32, u32) -> c_int,
    post_event_record_to: unsafe extern "C" fn(*mut ProcessSerialNumber, *mut u8) -> c_int,
}

pub(super) struct CoreGraphicsApis {
    pub preflight_screen_capture_access: unsafe extern "C" fn() -> bool,
    pub request_screen_capture_access: unsafe extern "C" fn() -> bool,
}

struct ApplicationServicesApis {
    create_ax_element_with_remote_token: unsafe extern "C" fn(CFDataRef) -> AXUIElementRef,
}

pub(super) const AX_MESSAGING_TIMEOUT: c_float = AX_MESSAGING_TIMEOUT_SECONDS;

pub(super) fn remote_ax_window_for_pid(pid: c_int, window_id: u32) -> Option<OwnedAxElement> {
    let apis = application_services_apis()?;
    let mut token = [0_u8; 20];
    token[0..4].copy_from_slice(&pid.to_ne_bytes());
    token[8..12].copy_from_slice(&REMOTE_AX_TOKEN_MAGIC.to_ne_bytes());

    let started_at = Instant::now();
    for ax_id in 0..REMOTE_AX_WINDOW_SCAN_LIMIT {
        token[12..20].copy_from_slice(&ax_id.to_ne_bytes());
        let token_data = CFData::from_buffer(&token);
        let value =
            unsafe { (apis.create_ax_element_with_remote_token)(token_data.as_concrete_TypeRef()) };
        if value.is_null() {
            continue;
        }

        let element = OwnedAxElement(value);
        if matches!(copy_ax_window_id(element.as_ptr()).ok().flatten(), Some(id) if id == window_id)
        {
            return Some(element);
        }

        if started_at.elapsed() >= REMOTE_AX_WINDOW_SCAN_BUDGET {
            return None;
        }
    }

    None
}

pub(super) fn focus_cg_window(pid: c_int, window_id: u32) -> Result<()> {
    let apis = window_server_apis().ok_or_else(|| {
        anyhow::anyhow!("SkyLight window focus APIs are unavailable on this macOS installation")
    })?;
    let mut psn = ProcessSerialNumber::default();
    let status = unsafe { GetProcessForPID(pid, &mut psn) };
    if status != 0 {
        bail!("failed to resolve process serial number for pid {pid}: status {status}");
    }

    let error =
        unsafe { (apis.set_front_process_with_options)(&mut psn, window_id, CPS_USER_GENERATED) };
    if error != 0 {
        bail!("failed to set front process for window {window_id}: error {error}");
    }

    make_key_window(apis, &mut psn, window_id)
}

fn make_key_window(
    apis: &WindowServerApis,
    psn: &mut ProcessSerialNumber,
    window_id: u32,
) -> Result<()> {
    let mut bytes = [0_u8; MAKE_KEY_EVENT_BYTES];
    bytes[0x04] = MAKE_KEY_EVENT_BYTES as u8;
    bytes[0x3a] = 0x10;
    bytes[0x3c..0x40].copy_from_slice(&window_id.to_ne_bytes());
    bytes[0x20..0x30].fill(0xff);

    bytes[0x08] = 0x01;
    let first_error = unsafe { (apis.post_event_record_to)(psn, bytes.as_mut_ptr()) };
    if first_error != 0 {
        bail!("failed to post key-window begin event: error {first_error}");
    }

    bytes[0x08] = 0x02;
    let second_error = unsafe { (apis.post_event_record_to)(psn, bytes.as_mut_ptr()) };
    if second_error != 0 {
        bail!("failed to post key-window end event: error {second_error}");
    }

    Ok(())
}

fn window_server_apis() -> Option<&'static WindowServerApis> {
    static APIS: OnceLock<Option<WindowServerApis>> = OnceLock::new();
    APIS.get_or_init(load_window_server_apis).as_ref()
}

pub(super) fn core_graphics_apis() -> Option<&'static CoreGraphicsApis> {
    static APIS: OnceLock<Option<CoreGraphicsApis>> = OnceLock::new();
    APIS.get_or_init(load_core_graphics_apis).as_ref()
}

fn application_services_apis() -> Option<&'static ApplicationServicesApis> {
    static APIS: OnceLock<Option<ApplicationServicesApis>> = OnceLock::new();
    APIS.get_or_init(load_application_services_apis).as_ref()
}

fn load_core_graphics_apis() -> Option<CoreGraphicsApis> {
    let handle = unsafe {
        dlopen(
            c"/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics".as_ptr(),
            RTLD_LAZY | RTLD_LOCAL,
        )
    };
    if handle.is_null() {
        return None;
    }

    Some(CoreGraphicsApis {
        preflight_screen_capture_access: load_symbol(handle, "CGPreflightScreenCaptureAccess")?,
        request_screen_capture_access: load_symbol(handle, "CGRequestScreenCaptureAccess")?,
    })
}

fn load_application_services_apis() -> Option<ApplicationServicesApis> {
    let handle = unsafe {
        dlopen(
            c"/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices"
                .as_ptr(),
            RTLD_LAZY | RTLD_LOCAL,
        )
    };
    if handle.is_null() {
        return None;
    }

    Some(ApplicationServicesApis {
        create_ax_element_with_remote_token: load_symbol(
            handle,
            "_AXUIElementCreateWithRemoteToken",
        )?,
    })
}

fn load_window_server_apis() -> Option<WindowServerApis> {
    let handle = unsafe {
        dlopen(
            c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight".as_ptr(),
            RTLD_LAZY | RTLD_LOCAL,
        )
    };
    if handle.is_null() {
        return None;
    }

    Some(WindowServerApis {
        set_front_process_with_options: load_symbol(handle, "_SLPSSetFrontProcessWithOptions")?,
        post_event_record_to: load_symbol(handle, "SLPSPostEventRecordTo")?,
    })
}

fn load_symbol<T>(handle: *mut c_void, name: &str) -> Option<T> {
    let mut symbol_name = Vec::with_capacity(name.len() + 1);
    symbol_name.extend_from_slice(name.as_bytes());
    symbol_name.push(0);

    let symbol = unsafe { dlsym(handle, symbol_name.as_ptr().cast()) };
    if symbol.is_null() {
        return None;
    }
    Some(unsafe { std::mem::transmute_copy(&symbol) })
}

pub(super) fn ax_focused_attribute() -> CFString {
    CFString::from_static_string("AXFocused")
}

pub(super) fn ax_main_attribute() -> CFString {
    CFString::from_static_string("AXMain")
}

pub(super) fn ax_raise_action() -> CFString {
    CFString::from_static_string("AXRaise")
}

pub(super) fn ax_title_attribute() -> CFString {
    CFString::from_static_string("AXTitle")
}

pub(super) fn ax_subrole_attribute() -> CFString {
    CFString::from_static_string("AXSubrole")
}

pub(super) fn ax_windows_attribute() -> CFString {
    CFString::from_static_string("AXWindows")
}

pub(super) fn copy_ax_window_id(element: AXUIElementRef) -> Result<Option<u32>> {
    let mut value = 0_u32;
    let error = unsafe { _AXUIElementGetWindow(element, &mut value) };
    match error {
        K_AX_ERROR_SUCCESS => Ok(Some(value)),
        K_AX_ERROR_ATTRIBUTE_UNSUPPORTED | K_AX_ERROR_NO_VALUE => Ok(None),
        other => bail!(ax_error_message(other)),
    }
}

pub(super) fn copy_ax_string(
    element: AXUIElementRef,
    attribute: CFStringRef,
) -> Result<Option<String>> {
    let Some(value) = copy_ax_attribute_value(element, attribute)
        .map_err(|error| anyhow::anyhow!(ax_error_message(error)))?
    else {
        return Ok(None);
    };
    Ok(value.downcast::<CFString>().map(|value| value.to_string()))
}

fn copy_ax_attribute_value(
    element: AXUIElementRef,
    attribute: CFStringRef,
) -> std::result::Result<Option<core_foundation::base::CFType>, AXError> {
    let mut value: CFTypeRef = std::ptr::null();
    let error = unsafe { AXUIElementCopyAttributeValue(element, attribute, &mut value) };
    match error {
        K_AX_ERROR_SUCCESS if value.is_null() => Ok(None),
        K_AX_ERROR_SUCCESS => Ok(Some(unsafe {
            core_foundation::base::CFType::wrap_under_create_rule(value)
        })),
        K_AX_ERROR_NO_VALUE => Ok(None),
        other => Err(other),
    }
}

pub(super) fn set_ax_bool_attribute(
    element: AXUIElementRef,
    attribute: CFStringRef,
    value: bool,
) -> std::result::Result<(), AXError> {
    let boolean = if value {
        CFBoolean::true_value()
    } else {
        CFBoolean::false_value()
    };
    let error = unsafe { AXUIElementSetAttributeValue(element, attribute, boolean.as_CFTypeRef()) };
    match error {
        K_AX_ERROR_SUCCESS => Ok(()),
        other => Err(other),
    }
}

pub(super) fn perform_ax_action(
    element: AXUIElementRef,
    action: CFStringRef,
) -> std::result::Result<(), AXError> {
    let error = unsafe { AXUIElementPerformAction(element, action) };
    match error {
        K_AX_ERROR_SUCCESS => Ok(()),
        other => Err(other),
    }
}

pub(super) fn ax_error_message(error: AXError) -> &'static str {
    match error {
        K_AX_ERROR_SUCCESS => "success",
        K_AX_ERROR_CANNOT_COMPLETE => "the target app did not respond to Accessibility",
        K_AX_ERROR_ATTRIBUTE_UNSUPPORTED => {
            "the target window does not expose the requested Accessibility attribute"
        }
        K_AX_ERROR_ACTION_UNSUPPORTED => {
            "the target window does not expose the requested Accessibility action"
        }
        K_AX_ERROR_API_DISABLED => "Accessibility access is disabled for this process",
        K_AX_ERROR_NO_VALUE => "the requested Accessibility value is missing",
        _ => "an unknown Accessibility error occurred",
    }
}

pub(super) struct OwnedAxElement(AXUIElementRef);

impl OwnedAxElement {
    pub(super) fn application(pid: c_int) -> Option<Self> {
        let value = unsafe { AXUIElementCreateApplication(pid) };
        if value.is_null() {
            return None;
        }
        Some(Self(value))
    }

    pub(super) fn as_ptr(&self) -> AXUIElementRef {
        self.0
    }

    pub(super) fn set_messaging_timeout(
        &self,
        timeout_in_seconds: c_float,
    ) -> std::result::Result<(), AXError> {
        let error = unsafe { AXUIElementSetMessagingTimeout(self.as_ptr(), timeout_in_seconds) };
        match error {
            K_AX_ERROR_SUCCESS => Ok(()),
            other => Err(other),
        }
    }

    pub(super) fn copy_array_attribute(
        &self,
        attribute: CFStringRef,
    ) -> std::result::Result<Option<CFArray>, AXError> {
        let Some(value) = copy_ax_attribute_value(self.as_ptr(), attribute)? else {
            return Ok(None);
        };
        Ok(value.downcast_into::<CFArray>())
    }
}

impl Drop for OwnedAxElement {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0 as _);
        }
    }
}
