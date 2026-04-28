//! Display identity helpers shared by runtime sizing and config editing.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayProfile {
    pub name: Option<String>,
    pub native_id: u32,
    pub built_in: bool,
    pub vendor: Option<u32>,
    pub model: Option<u32>,
    pub serial: Option<u32>,
    pub primary: bool,
}

impl DisplayProfile {
    pub fn label(&self) -> String {
        if let Some(name) = self.name.as_deref().map(str::trim)
            && !name.is_empty()
        {
            return name.to_owned();
        }

        let base = if self.built_in {
            "Built-in display".to_owned()
        } else {
            "External display".to_owned()
        };

        let mut details = Vec::new();
        if let Some(vendor) = self.vendor {
            details.push(format!("vendor {vendor}"));
        }
        if let Some(model) = self.model {
            details.push(format!("model {model}"));
        }
        if let Some(serial) = self.serial {
            details.push(format!("serial {serial}"));
        }

        if details.is_empty() {
            base
        } else {
            format!("{base} ({})", details.join(", "))
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::collections::HashMap;

    use core_graphics::{
        display::CGDisplay,
        event::CGEvent,
        event_source::{CGEventSource, CGEventSourceStateID},
    };
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{NSNumber, ns_string};
    use tao::{monitor::MonitorHandle, platform::macos::MonitorHandleExtMacOS};

    use super::DisplayProfile;

    pub fn active_displays() -> Vec<DisplayProfile> {
        let names = display_names_by_id();
        CGDisplay::active_displays()
            .map(|displays| {
                displays
                    .into_iter()
                    .map(|display_id| {
                        profile_from_display(
                            CGDisplay::new(display_id),
                            names.get(&display_id).cloned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn current_display() -> Option<DisplayProfile> {
        let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
        let event = CGEvent::new(source).ok()?;
        let point = event.location();
        let (display_ids, matching_count) = CGDisplay::displays_with_point(point, 8).ok()?;
        let display_id = display_ids
            .into_iter()
            .take(matching_count as usize)
            .next()?;
        Some(profile_from_display(
            CGDisplay::new(display_id),
            display_name_by_id(display_id),
        ))
    }

    pub fn primary_display() -> Option<DisplayProfile> {
        let display = CGDisplay::main();
        Some(profile_from_display(
            display,
            display_name_by_id(display.id),
        ))
    }

    pub fn profile_from_monitor(monitor: &MonitorHandle) -> DisplayProfile {
        profile_from_display(
            CGDisplay::new(monitor.native_id()),
            monitor
                .name()
                .filter(|name| !name.trim().is_empty())
                .or_else(|| display_name_by_id(monitor.native_id())),
        )
    }

    fn profile_from_display(display: CGDisplay, name: Option<String>) -> DisplayProfile {
        DisplayProfile {
            name,
            native_id: display.id,
            built_in: display.is_builtin(),
            vendor: some_nonzero(display.vendor_number()),
            model: some_nonzero(display.model_number()),
            serial: some_nonzero(display.serial_number()),
            primary: display.is_main(),
        }
    }

    fn some_nonzero(value: u32) -> Option<u32> {
        (value != 0).then_some(value)
    }

    fn display_name_by_id(display_id: u32) -> Option<String> {
        display_names_by_id().remove(&display_id)
    }

    fn display_names_by_id() -> HashMap<u32, String> {
        let Some(mtm) = MainThreadMarker::new() else {
            return HashMap::new();
        };

        let screens = NSScreen::screens(mtm);
        let mut names = HashMap::new();
        for index in 0..screens.count() {
            let screen = screens.objectAtIndex(index);
            let display_id = screen_display_id(&screen);
            let name = screen.localizedName().to_string();
            if display_id != 0 && !name.trim().is_empty() {
                names.insert(display_id, name);
            }
        }
        names
    }

    fn screen_display_id(screen: &NSScreen) -> u32 {
        screen
            .deviceDescription()
            .objectForKey(ns_string!("NSScreenNumber"))
            .and_then(|value| value.downcast::<NSNumber>().ok())
            .map_or(0, |value| value.as_u32())
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::DisplayProfile;

    pub fn active_displays() -> Vec<DisplayProfile> {
        Vec::new()
    }

    pub fn current_display() -> Option<DisplayProfile> {
        None
    }

    pub fn primary_display() -> Option<DisplayProfile> {
        None
    }
}

pub use imp::{active_displays, current_display, primary_display};

#[cfg(target_os = "macos")]
pub use imp::profile_from_monitor;

#[cfg(test)]
mod tests {
    use super::DisplayProfile;

    #[test]
    fn label_prefers_human_readable_name() {
        let display = DisplayProfile {
            name: Some("DELL U2720Q".to_owned()),
            native_id: 1,
            built_in: false,
            vendor: Some(4268),
            model: Some(12345),
            serial: Some(987654),
            primary: false,
        };

        assert_eq!(display.label(), "DELL U2720Q");
    }

    #[test]
    fn label_falls_back_to_identity_details_without_name() {
        let display = DisplayProfile {
            name: None,
            native_id: 1,
            built_in: false,
            vendor: Some(4268),
            model: Some(12345),
            serial: None,
            primary: false,
        };

        assert_eq!(
            display.label(),
            "External display (vendor 4268, model 12345)"
        );
    }
}
