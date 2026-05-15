use serde::Serialize;

/// Snapshot of the app that was frontmost before Runx appeared.
#[derive(Debug, Clone, Default)]
pub struct FrontmostApp {
    pub name: Option<String>,
    pub bundle_id: Option<String>,
    pub path: Option<String>,
}

/// Metadata for one running macOS application.
#[derive(Debug, Clone, Serialize)]
pub struct RunningApp {
    pub pid: i64,
    pub name: String,
    pub bundle_id: Option<String>,
    pub path: Option<String>,
}

/// Direct CoreGraphics snapshot of the display currently containing the mouse cursor.
#[derive(Debug, Clone, Copy)]
pub struct CursorDisplayLocation {
    pub display_id: u32,
}

/// Window metadata exposed through macOS Accessibility.
#[derive(Debug, Clone, Serialize)]
pub struct AccessibilityWindow {
    pub window_id: u32,
    pub title: String,
    pub subrole: String,
}
