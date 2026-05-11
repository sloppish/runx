//! Shared data structures exchanged between modules.
//!
//! These types are the common language between the launcher runtime, providers,
//! plugins, and the embedded frontend.

use std::collections::HashMap;

use global_hotkey::GlobalHotKeyEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Canonical internal representation of one search result candidate.
#[derive(Debug, Clone)]
pub struct SearchItem {
    pub id: String,
    pub provider: String,
    pub badge: String,
    pub icon: Option<String>,
    pub title: String,
    pub subtitle: String,
    pub compact: bool,
    pub raw_score: i64,
    pub action: Action,
}

/// Opaque plugin payload carried from search results to the `run` handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginActionPayload(pub JsonValue);

/// Actions that Runx can execute after a search result is activated.
#[derive(Debug, Clone)]
pub enum Action {
    Noop,
    OpenApplication {
        path: String,
    },
    OpenSettings {
        url: String,
        title: String,
    },
    FocusWindow {
        app_name: String,
        window_title: String,
        window_id: u32,
        pid: i64,
    },
    Plugin {
        plugin_id: String,
        payload: PluginActionPayload,
    },
}

/// User events sent through Tao's custom event channel.
#[derive(Debug, Clone)]
pub enum AppEvent {
    GlobalHotKey(GlobalHotKeyEvent),
    Frontend(FrontendCommand),
    Settings(SettingsCommand),
    ReloadConfig,
    Render,
    IconReady,
    StartSearch {
        token: u64,
    },
    TrayToggle,
    TrayOpen,
    TraySettings,
    TrayToggleAutostart,
    TraySponsor,
    TrayCheckForUpdates,
    Quit,
    FrontendReadyWatchdog,
    QuickSwitchPoll,
    QuickSwitchShow,
    ProviderItems {
        generation: u64,
        provider: String,
        items: Vec<SearchItem>,
    },
    ProviderError {
        generation: u64,
        provider: String,
        message: String,
    },
    ActionOutcome {
        message: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    Regular,
    QuickSwitch,
}

/// Commands emitted by the embedded frontend back into the Rust event loop.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FrontendCommand {
    Ready,
    PreferredHeight { height: f64, layout_version: u64 },
    QueryChanged { query: String },
    Activate { index: usize, all_windows: bool },
    CopyText { text: String },
    PasteText,
    Hide,
    QuickSwitchCycle,
}

/// Commands emitted by the Settings webview back into the Rust event loop.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettingsCommand {
    Ready,
    Reload,
    Save { draft: Box<SettingsDraft> },
    SaveRaw { raw: String },
    OpenUrl { url: String },
    UpdatePlugins,
    ClientError { message: String },
    Close,
}

/// Structured subset edited by the first Settings window implementation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SettingsDraft {
    pub debug_log: bool,
    pub hotkey: HotkeySettingsDraft,
    pub window: WindowSettingsDraft,
    pub display_overrides: Vec<DisplayOverrideSettingsDraft>,
    pub providers: ProvidersSettingsDraft,
    pub ranking: RankingSettingsDraft,
    pub timing: TimingSettingsDraft,
    pub plugins: PluginsSettingsDraft,
    pub ui: UiSettingsDraft,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HotkeySettingsDraft {
    pub shortcut: String,
    pub quick_switch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowSettingsDraft {
    pub width_fraction: f64,
    pub visible_rows: usize,
    pub min_width: Option<f64>,
    pub max_width: Option<f64>,
    pub min_height: Option<f64>,
    pub max_height: Option<f64>,
    pub hide_when_inactive: bool,
    pub always_on_top: bool,
    pub show_on: String,
    pub scale: f64,
    pub show_animation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DisplayOverrideSettingsDraft {
    pub built_in: Option<bool>,
    pub vendor: Option<u32>,
    pub model: Option<u32>,
    pub serial: Option<u32>,
    pub width_fraction: Option<f64>,
    pub visible_rows: Option<usize>,
    pub min_width: Option<f64>,
    pub max_width: Option<f64>,
    pub min_height: Option<f64>,
    pub max_height: Option<f64>,
    pub scale: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProvidersSettingsDraft {
    pub disabled: Vec<String>,
    pub windows: WindowsProviderSettingsDraft,
    pub apps: AppsProviderSettingsDraft,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowsProviderSettingsDraft {
    pub include_other_desktops: bool,
    pub show_on_empty_query: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppsProviderSettingsDraft {
    pub exact_name_boost: i64,
    pub prefix_name_boost: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RankingSettingsDraft {
    pub tie_threshold: i64,
    pub provider_order: Vec<String>,
    pub provider_score_boosts: HashMap<String, i64>,
    pub score_rules: Vec<RankingScoreRuleSettingsDraft>,
    pub result_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RankingScoreRuleSettingsDraft {
    pub providers: Vec<String>,
    pub field: String,
    pub match_kind: String,
    pub pattern: String,
    pub boost: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimingSettingsDraft {
    pub search_debounce_ms: u64,
    pub quick_switch_show_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginsSettingsDraft {
    pub directories: Vec<String>,
    pub search_paths: Vec<String>,
    pub install: Vec<PluginInstallSettingsDraft>,
    pub plugin_toml: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginInstallSettingsDraft {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiSettingsDraft {
    pub show_header: bool,
    pub cycle_selection: bool,
    pub colorscheme: String,
    pub font_family: String,
    pub canvas: UiCanvasSettingsDraft,
    pub entries: UiEntriesSettingsDraft,
    pub shortcuts: UiShortcutsSettingsDraft,
    pub colorschemes: Vec<UiColorschemeSettingsDraft>,
    pub font_sizes: UiFontSizesSettingsDraft,
    pub layout: UiLayoutSettingsDraft,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiCanvasSettingsDraft {
    pub show: bool,
    pub radius: u16,
    pub background_opacity: f64,
    pub chrome_opacity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiEntriesSettingsDraft {
    pub opacity: f64,
    pub show_hover: bool,
    pub transition_ms: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiShortcutsSettingsDraft {
    pub focus_window: String,
    pub activate_all_windows: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiColorschemeSettingsDraft {
    pub name: String,
    pub base: String,
    pub tokens: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiFontSizesSettingsDraft {
    pub label: u16,
    pub input: u16,
    pub title: u16,
    pub subtitle: u16,
    pub badge: u16,
    pub accelerator: u16,
    pub config_error_title: u16,
    pub config_error_body: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiLayoutSettingsDraft {
    pub section_gap: u16,
    pub input_padding_y: u16,
    pub input_padding_x: u16,
    pub input_radius: u16,
    pub list_gap: u16,
    pub entry_padding_y: u16,
    pub entry_padding_x: u16,
    pub entry_gap: u16,
    pub row_radius: u16,
    pub badge_size: u16,
    pub badge_radius: u16,
    pub icon_size: u16,
}

/// Serialized frontend state pushed into the webview on each render.
#[derive(Debug, Clone, Serialize)]
pub struct ViewState {
    pub mode: ViewMode,
    pub query: String,
    pub config_error: Option<String>,
    pub selected_index: usize,
    pub items: Vec<ViewItem>,
}

/// One visible row in the launcher result list.
#[derive(Debug, Clone, Serialize)]
pub struct ViewItem {
    pub title: String,
    pub subtitle: String,
    pub badge: String,
    pub icon: Option<String>,
    pub accelerator: Option<String>,
    pub compact: bool,
}

#[cfg(test)]
mod tests {
    use super::PluginActionPayload;

    #[test]
    fn plugin_action_payload_preserves_fields() {
        let payload = serde_json::from_value::<PluginActionPayload>(serde_json::json!({
            "kind": "copy_password",
            "entry": "mail/example",
            "count": 2
        }))
        .expect("valid payload");

        let obj = payload.0.as_object().expect("should be an object");
        assert_eq!(obj.get("kind"), Some(&serde_json::json!("copy_password")));
        assert_eq!(obj.get("entry"), Some(&serde_json::json!("mail/example")));
        assert_eq!(obj.get("count"), Some(&serde_json::json!(2)));
    }
}
