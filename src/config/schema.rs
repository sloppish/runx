use std::collections::HashMap;

use anyhow::{Context, Result, anyhow, bail};
use global_hotkey::hotkey::HotKey;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use toml::Table;

use crate::displays::DisplayProfile;

use super::shortcuts::{deserialize_ui_shortcut, parse_hotkey_shortcut};

/// Root configuration object deserialized from `config.toml`.
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Global launcher shortcut.
    pub hotkey: HotKeyConfig,
    /// Geometry and behavior of the Runx window itself.
    pub window: WindowConfig,
    /// Per-display size and density overrides applied after the target display is resolved.
    pub display_overrides: Vec<DisplayOverrideConfig>,
    /// Provider-specific search, empty-query, and activation behavior.
    pub providers: ProvidersConfig,
    /// Result ranking and tie-breaking behavior.
    pub ranking: RankingConfig,
    /// Search and render scheduling knobs.
    pub timing: TimingConfig,
    /// Plugin discovery and subprocess lookup paths.
    pub plugins: PluginsConfig,
    /// Per-plugin configuration tables under `[plugin.<id>]`.
    pub plugin: HashMap<String, Table>,
    /// Launcher appearance and interaction settings.
    pub ui: UiConfig,
}

/// User-facing global hotkey configuration.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HotKeyConfig {
    /// Global shortcut that opens the launcher.
    pub shortcut: String,
}

/// Launcher window behavior and geometry.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct WindowConfig {
    /// Fraction of the chosen display's logical width used before optional width clamps.
    pub width_fraction: f64,
    /// Target number of result rows kept visible before optional height clamps.
    pub visible_rows: usize,
    /// Minimum launcher width in logical pixels.
    pub min_width: Option<f64>,
    /// Maximum launcher width in logical pixels.
    pub max_width: Option<f64>,
    /// Minimum launcher height in logical pixels.
    pub min_height: Option<f64>,
    /// Maximum launcher height in logical pixels.
    pub max_height: Option<f64>,
    /// Hide the launcher automatically when it becomes inactive.
    pub hide_when_inactive: bool,
    /// Keep the launcher above normal windows while it is visible.
    pub always_on_top: bool,
    /// Choose which display Runx appears on when it opens.
    pub show_on: WindowDisplayTarget,
    /// Multiplier applied to UI typography and spacing tokens.
    pub scale: f64,
}

/// Provider-specific runtime behavior.
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ProvidersConfig {
    /// Providers that should not run.
    pub disabled: Vec<String>,
    /// Settings for the macOS windows provider.
    pub windows: WindowsProviderConfig,
    /// Settings for installed application search ranking.
    pub apps: AppsProviderConfig,
}

/// Settings for the macOS windows provider.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct WindowsProviderConfig {
    /// Include windows from other macOS desktops/spaces in search results. Requires Screen Recording permission.
    pub include_other_desktops: bool,
    /// Show open windows before the query is non-empty.
    pub show_on_empty_query: bool,
}

/// Settings for installed application search ranking.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AppsProviderConfig {
    /// Score added when a query exactly matches an installed app name.
    pub exact_name_boost: i64,
    /// Score added when a query is a prefix of an installed app name.
    pub prefix_name_boost: i64,
}

/// Monitor selection strategy for placing the launcher window.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowDisplayTarget {
    /// Always open on the primary display.
    #[default]
    Primary,
    /// Open on the display that currently contains the mouse cursor.
    Cursor,
}

/// Per-display overrides for window sizing and scale.
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct DisplayOverrideConfig {
    /// Match built-in or external displays.
    pub built_in: Option<bool>,
    /// Match the monitor vendor id reported by macOS.
    pub vendor: Option<u32>,
    /// Match the monitor model id reported by macOS.
    pub model: Option<u32>,
    /// Match the monitor serial number reported by macOS.
    pub serial: Option<u32>,
    /// Override `window.width_fraction` for matching displays.
    pub width_fraction: Option<f64>,
    /// Override `window.visible_rows` for matching displays.
    pub visible_rows: Option<usize>,
    /// Override `window.min_width` for matching displays.
    pub min_width: Option<f64>,
    /// Override `window.max_width` for matching displays.
    pub max_width: Option<f64>,
    /// Override `window.min_height` for matching displays.
    pub min_height: Option<f64>,
    /// Override `window.max_height` for matching displays.
    pub max_height: Option<f64>,
    /// Override `window.scale` for matching displays.
    pub scale: Option<f64>,
}

/// Ranking and truncation rules for the merged result list.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RankingConfig {
    /// Maximum raw-score gap that still counts as a tie between providers.
    pub tie_threshold: i64,
    /// Tie-break priority when multiple providers return similarly scored items.
    pub provider_order: Vec<String>,
    /// Per-provider additive score adjustments applied after matching.
    pub provider_score_boosts: HashMap<String, i64>,
    /// Text-based additive score rules evaluated on merged results.
    pub score_rules: Vec<RankingScoreRule>,
    /// Maximum number of rows shown in the launcher.
    pub result_limit: usize,
}

/// One additive ranking rule matched against a result row.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RankingScoreRule {
    /// Restrict the rule to specific providers. Empty means all providers.
    pub providers: Vec<String>,
    /// Result field matched against `pattern`.
    pub field: RankingScoreRuleField,
    #[serde(rename = "match")]
    /// Text-matching mode used for `pattern`.
    pub match_kind: RankingScoreRuleMatchKind,
    /// Needle matched against the selected field.
    pub pattern: String,
    /// Additive score applied when the rule matches.
    pub boost: i64,
}

/// Search-item field targeted by a ranking score rule.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RankingScoreRuleField {
    /// Match against the result title.
    #[default]
    Title,
    /// Match against the result subtitle.
    Subtitle,
    /// Match against the result badge text.
    Badge,
    /// Match against the internal result id.
    Id,
}

/// Text-matching mode used by a ranking score rule.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RankingScoreRuleMatchKind {
    /// Require an exact string match.
    Exact,
    /// Require the field to start with `pattern`.
    Prefix,
    /// Require the field to contain `pattern`.
    #[default]
    Contains,
}

/// Debounce timings for the search pipeline.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct TimingConfig {
    /// Delay before a changed query starts a new search generation.
    pub search_debounce_ms: u64,
}

/// Plugin discovery and subprocess lookup configuration.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PluginsConfig {
    /// Extra directories that should be scanned for `.lua` plugins.
    pub directories: Vec<String>,
    /// Extra PATH entries exposed to plugin subprocess helpers.
    pub search_paths: Vec<String>,
}

/// Theme tokens injected into the embedded HTML/CSS UI templates.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// Show the small header label at the top of the launcher.
    pub show_header: bool,
    /// Wrap selection movement from end to start with arrows and Ctrl-N/Ctrl-P.
    pub cycle_selection: bool,
    /// Selected UI colorscheme: `system`, a built-in name, or a custom scheme name.
    pub colorscheme: String,
    /// CSS font-family stack used by the launcher UI.
    pub font_family: String,
    /// Named built-in and custom colorscheme definitions.
    pub colorschemes: HashMap<String, UiColorschemeConfig>,
    /// Canvas styling for the outer launcher panel.
    pub canvas: UiCanvasConfig,
    /// Styling for individual result-row surfaces.
    pub entries: UiEntriesConfig,
    /// Keyboard shortcuts handled inside the launcher UI.
    pub shortcuts: UiShortcutsConfig,
    /// UI font-size tokens.
    pub font_sizes: UiFontSizesConfig,
    /// Non-typographic layout tokens.
    pub layout: UiLayoutConfig,
}

/// One named colorscheme entry under `[ui.colorschemes.<name>]`.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UiColorschemeConfig {
    /// Base palette inherited by a custom colorscheme. Without a base, custom schemes must define every color token. Built-in schemes must not set this.
    pub base: Option<String>,
    #[serde(flatten)]
    /// Color token values, or optional overrides when `base` is set.
    pub overrides: UiColorOverridesConfig,
}

/// Full per-scheme UI color-token overrides.
/// Invokes `$callback!($field)` for every UI color token field name.
#[macro_export]
macro_rules! for_each_color_token {
    ($callback:ident) => {
        $callback!(accent);
        $callback!(panel);
        $callback!(text);
        $callback!(muted);
        $callback!(canvas_bg);
        $callback!(canvas_shadow);
        $callback!(canvas_border);
        $callback!(label_strong);
        $callback!(input_bg);
        $callback!(input_border);
        $callback!(input_shadow);
        $callback!(placeholder);
        $callback!(scrollbar);
        $callback!(item_bg);
        $callback!(item_hover);
        $callback!(item_selected_bg);
        $callback!(item_selected_shadow);
        $callback!(badge_bg);
        $callback!(badge_border);
        $callback!(badge_text);
        $callback!(badge_icon_bg);
        $callback!(chip_text);
        $callback!(chip_bg);
        $callback!(chip_border);
        $callback!(config_error_bg);
        $callback!(config_error_border);
        $callback!(config_error_shadow);
        $callback!(config_error_title);
        $callback!(config_error_copy);
        $callback!(canvas_hidden_input_bg);
        $callback!(canvas_hidden_input_border);
        $callback!(canvas_hidden_input_shadow);
        $callback!(canvas_hidden_item_bg);
        $callback!(canvas_hidden_item_hover);
        $callback!(canvas_hidden_item_selected_bg);
        $callback!(canvas_hidden_config_error_bg);
    };
}

macro_rules! define_color_tokens {
    ($($field:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
        #[serde(default, deny_unknown_fields)]
        pub struct UiColorOverridesConfig {
            $(pub $field: Option<String>,)+
        }

        /// Configurable UI color token names for `[ui.colorschemes.<name>]`.
        pub const UI_COLOR_TOKEN_NAMES: [&str; define_color_tokens!(@count $($field)+)] = [
            $(stringify!($field),)+
        ];

        impl UiColorOverridesConfig {
            pub(super) fn missing_required_fields(&self) -> Vec<&'static str> {
                let mut missing = Vec::new();
                $(if self.$field.is_none() { missing.push(stringify!($field)); })+
                missing
            }

            pub fn to_token_map(&self) -> std::collections::HashMap<String, String> {
                let mut tokens = std::collections::HashMap::new();
                $(if let Some(value) = &self.$field {
                    tokens.insert(stringify!($field).to_owned(), value.clone());
                })+
                tokens
            }
        }
    };
    (@count $($t:tt)+) => {
        0 $(+ define_color_tokens!(@one $t))+
    };
    (@one $t:tt) => { 1 };
}

define_color_tokens! {
    accent,
    panel,
    text,
    muted,
    canvas_bg,
    canvas_shadow,
    canvas_border,
    label_strong,
    input_bg,
    input_border,
    input_shadow,
    placeholder,
    scrollbar,
    item_bg,
    item_hover,
    item_selected_bg,
    item_selected_shadow,
    badge_bg,
    badge_border,
    badge_text,
    badge_icon_bg,
    chip_text,
    chip_bg,
    chip_border,
    config_error_bg,
    config_error_border,
    config_error_shadow,
    config_error_title,
    config_error_copy,
    canvas_hidden_input_bg,
    canvas_hidden_input_border,
    canvas_hidden_input_shadow,
    canvas_hidden_item_bg,
    canvas_hidden_item_hover,
    canvas_hidden_item_selected_bg,
    canvas_hidden_config_error_bg,
}

/// Canvas tokens injected into the embedded UI theme.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UiCanvasConfig {
    /// Show the outer launcher panel behind the input and results.
    pub show: bool,
    /// Corner radius of the outer launcher panel.
    pub radius: u16,
    /// Opacity of the panel fill behind the launcher contents.
    pub background_opacity: f64,
    /// Opacity of the panel border and shadow.
    #[serde(alias = "opacity")]
    pub chrome_opacity: f64,
}

/// Entry background tokens injected into the embedded UI theme.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UiEntriesConfig {
    /// Opacity multiplier applied to result-row background surfaces.
    pub opacity: f64,
}

/// Keyboard shortcuts handled inside the launcher UI.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UiShortcutsConfig {
    /// Shortcut for activating the selected window.
    #[serde(
        default = "default_focus_window_shortcut",
        deserialize_with = "deserialize_ui_shortcut"
    )]
    pub focus_window: Option<UiShortcutConfig>,
    /// Shortcut for activating the selected app and bringing all its windows forward.
    #[serde(
        default = "default_activate_all_windows_shortcut",
        deserialize_with = "deserialize_ui_shortcut"
    )]
    pub activate_all_windows: Option<UiShortcutConfig>,
}

/// A browser-keyboard shortcut passed through to the embedded UI.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UiShortcutConfig {
    pub key: Option<String>,
    pub code: Option<String>,
    pub alt: bool,
    pub ctrl: bool,
    pub meta: bool,
    pub shift: bool,
}

/// Font-size tokens injected into the embedded UI theme.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UiFontSizesConfig {
    /// Size of small labels and helper text.
    pub label: u16,
    /// Search input text size.
    pub input: u16,
    /// Primary result title text size.
    pub title: u16,
    /// Secondary result subtitle text size.
    pub subtitle: u16,
    /// Badge text size.
    pub badge: u16,
    /// Accelerator chip text size.
    pub accelerator: u16,
    /// Config-error title text size.
    pub config_error_title: u16,
    /// Config-error body text size.
    pub config_error_body: u16,
}

/// Layout tokens injected into the embedded UI theme.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UiLayoutConfig {
    /// Gap between top-level shell sections such as input and results.
    pub section_gap: u16,
    /// Vertical padding inside the search input.
    pub input_padding_y: u16,
    /// Horizontal padding inside the search input.
    pub input_padding_x: u16,
    /// Corner radius of the search input.
    pub input_radius: u16,
    /// Vertical gap between visible result rows.
    pub list_gap: u16,
    /// Vertical padding inside each result row.
    pub entry_padding_y: u16,
    /// Horizontal padding inside each result row.
    pub entry_padding_x: u16,
    /// Gap between columns inside each result row.
    pub entry_gap: u16,
    /// Corner radius of result rows.
    pub row_radius: u16,
    /// Size of badge containers.
    pub badge_size: u16,
    /// Corner radius of badge containers.
    pub badge_radius: u16,
    /// Size of icon images inside icon badges.
    pub icon_size: u16,
}

impl Config {
    /// Converts the configured shortcut string into a `global_hotkey` binding.
    pub fn hotkey(&self) -> Result<HotKey> {
        self.hotkey.to_hotkey()
    }

    /// Returns per-plugin config tables with routing metadata stripped out.
    pub fn plugin_config(&self) -> Result<HashMap<String, JsonValue>> {
        self.plugin
            .iter()
            .map(|(id, table)| {
                let mut table = table.clone();
                table.remove("commands");
                let value = serde_json::to_value(table)
                    .with_context(|| format!("failed to serialize plugin config for `{id}`"))?;
                Ok((id.clone(), value))
            })
            .collect()
    }

    /// Returns the command-prefix routing table for configured plugins.
    pub fn plugin_routes(&self) -> Result<HashMap<String, HashMap<String, String>>> {
        let mut routes_by_plugin = HashMap::new();

        for (plugin_id, table) in &self.plugin {
            let Some(commands) = table.get("commands") else {
                continue;
            };

            let commands = commands
                .as_table()
                .ok_or_else(|| anyhow!("`[plugin.{plugin_id}.commands]` must be a TOML table"))?;

            let mut routes = HashMap::new();
            for (command, handler_value) in commands {
                let normalized_command = command.trim();
                if normalized_command.is_empty() {
                    bail!("`[plugin.{plugin_id}.commands]` contains an empty command name");
                }

                let handler = handler_value.as_str().ok_or_else(|| {
                    anyhow!(
                        "`[plugin.{plugin_id}.commands.{normalized_command}]` must be a string handler name"
                    )
                })?;
                let normalized_handler = handler.trim();
                if normalized_handler.is_empty() {
                    bail!("`[plugin.{plugin_id}.commands.{normalized_command}]` must not be empty");
                }

                routes.insert(normalized_command.to_owned(), normalized_handler.to_owned());
            }

            if !routes.is_empty() {
                routes_by_plugin.insert(plugin_id.clone(), routes);
            }
        }

        Ok(routes_by_plugin)
    }
}

impl UiConfig {
    /// Returns the selected colorscheme name, or `system` to follow the built-in light/dark pair.
    pub fn colorscheme(&self) -> &str {
        self.colorscheme.as_str()
    }

    /// Returns a named colorscheme, or an empty override set when it is not configured.
    pub fn colorscheme_config(&self, name: &str) -> UiColorschemeConfig {
        self.colorschemes.get(name).cloned().unwrap_or_default()
    }

    /// Estimates the launcher height for a fixed number of visible result rows.
    ///
    /// This is a bootstrap fallback used before the embedded frontend reports its
    /// measured preferred height from the live CSS/DOM layout.
    pub fn estimated_window_height(&self, visible_rows: usize, scale: f64) -> f64 {
        const LINE_HEIGHT: f64 = 1.2;
        const BODY_PADDING: f64 = 18.0;
        const SHELL_PADDING: f64 = 18.0;
        const SUBTITLE_MARGIN_TOP: f64 = 4.0;
        const CHIP_PADDING_Y: f64 = 6.0;
        const COMPACT_COPY_MIN_HEIGHT: f64 = 24.0;
        const BORDER_WIDTH: f64 = 1.0;

        let scaled = |value: f64| value * scale;
        let line_box = |font_size: u16| f64::from(font_size) * scale * LINE_HEIGHT;

        let header_height = if self.show_header {
            line_box(self.font_sizes.label)
        } else {
            0.0
        };
        let section_gaps = if self.show_header { 2.0 } else { 1.0 };
        let input_height = line_box(self.font_sizes.input)
            + scaled(f64::from(self.layout.input_padding_y) * 2.0)
            + BORDER_WIDTH * 2.0;
        let title_stack_height = line_box(self.font_sizes.title)
            + scaled(SUBTITLE_MARGIN_TOP)
            + line_box(self.font_sizes.subtitle);
        let compact_copy_height = scaled(COMPACT_COPY_MIN_HEIGHT);
        let accelerator_height = line_box(self.font_sizes.accelerator)
            + scaled(CHIP_PADDING_Y * 2.0)
            + BORDER_WIDTH * 2.0;
        let row_content_height = scaled(f64::from(self.layout.badge_size))
            .max(scaled(f64::from(self.layout.icon_size)))
            .max(title_stack_height.max(compact_copy_height))
            .max(accelerator_height);
        let row_height = row_content_height + scaled(f64::from(self.layout.entry_padding_y) * 2.0);
        let results_height = if visible_rows == 0 {
            0.0
        } else {
            row_height * visible_rows as f64
                + scaled(f64::from(self.layout.list_gap)) * visible_rows.saturating_sub(1) as f64
        };

        scaled(BODY_PADDING * 2.0)
            + scaled(SHELL_PADDING * 2.0)
            + scaled(f64::from(self.layout.section_gap)) * section_gaps
            + header_height
            + input_height
            + results_height
    }
}

impl Default for HotKeyConfig {
    fn default() -> Self {
        Self {
            shortcut: "Option+Space".to_owned(),
        }
    }
}

impl HotKeyConfig {
    /// Parses the configured shortcut into a `global_hotkey` binding.
    pub fn to_hotkey(&self) -> Result<HotKey> {
        parse_hotkey_shortcut(&self.shortcut)
    }
}

impl Config {
    /// Resolves the effective window and UI config for a specific display.
    pub fn resolved_window_and_ui(
        &self,
        display: Option<&DisplayProfile>,
    ) -> (WindowConfig, UiConfig) {
        let mut window = self.window.clone();
        let ui = self.ui.clone();

        if let Some(display_override) = self.display_override_for(display) {
            display_override.apply(&mut window);
        }

        (window, ui)
    }

    /// Returns the best matching display override for a specific display, if any.
    pub fn display_override_for(
        &self,
        display: Option<&DisplayProfile>,
    ) -> Option<&DisplayOverrideConfig> {
        self.display_override_index_for(display)
            .and_then(|index| self.display_overrides.get(index))
    }

    /// Returns the index of the best matching display override for a specific display, if any.
    pub fn display_override_index_for(&self, display: Option<&DisplayProfile>) -> Option<usize> {
        let display = display?;
        self.display_overrides
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.matches(display))
            .max_by_key(|(index, entry)| (entry.match_priority(), *index))
            .map(|(index, _)| index)
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width_fraction: 0.4,
            visible_rows: 5,
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            hide_when_inactive: true,
            always_on_top: true,
            show_on: WindowDisplayTarget::Cursor,
            scale: 1.0,
        }
    }
}

impl DisplayOverrideConfig {
    /// Builds an empty override entry targeting a specific display identity.
    pub fn for_display(display: &DisplayProfile) -> Self {
        Self {
            built_in: Some(display.built_in),
            vendor: display.vendor,
            model: display.model,
            serial: display.serial,
            width_fraction: None,
            visible_rows: None,
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            scale: None,
        }
    }

    /// Builds a capture entry for a specific display from the current base sizing settings.
    pub fn capture(display: &DisplayProfile, window: &WindowConfig) -> Self {
        Self {
            width_fraction: Some(window.width_fraction),
            visible_rows: Some(window.visible_rows),
            min_width: window.min_width,
            max_width: window.max_width,
            min_height: window.min_height,
            max_height: window.max_height,
            scale: Some(window.scale),
            ..Self::for_display(display)
        }
    }

    pub fn has_override_values(&self) -> bool {
        self.width_fraction.is_some()
            || self.visible_rows.is_some()
            || self.min_width.is_some()
            || self.max_width.is_some()
            || self.min_height.is_some()
            || self.max_height.is_some()
            || self.scale.is_some()
    }

    pub fn label(&self) -> String {
        let base = match self.built_in {
            Some(true) => "Built-in display".to_owned(),
            Some(false) => "External display".to_owned(),
            None => "Display override".to_owned(),
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

    pub fn matches_same_display(&self, other: &Self) -> bool {
        self.built_in == other.built_in
            && self.vendor == other.vendor
            && self.model == other.model
            && self.serial == other.serial
    }

    fn apply(&self, window: &mut WindowConfig) {
        if let Some(value) = self.width_fraction {
            window.width_fraction = value;
        }
        if let Some(value) = self.visible_rows {
            window.visible_rows = value;
        }
        if let Some(value) = self.min_width {
            window.min_width = Some(value);
        }
        if let Some(value) = self.max_width {
            window.max_width = Some(value);
        }
        if let Some(value) = self.min_height {
            window.min_height = Some(value);
        }
        if let Some(value) = self.max_height {
            window.max_height = Some(value);
        }
        if let Some(value) = self.scale {
            window.scale = value;
        }
    }

    fn matches(&self, display: &DisplayProfile) -> bool {
        self.built_in.is_none_or(|value| value == display.built_in)
            && self
                .vendor
                .is_none_or(|value| display.vendor == Some(value))
            && self.model.is_none_or(|value| display.model == Some(value))
            && self
                .serial
                .is_none_or(|value| display.serial == Some(value))
    }

    fn match_priority(&self) -> (u8, u8, u8, u8) {
        let vendor_model = self.vendor.is_some() && self.model.is_some();
        let specified = self.built_in.is_some() as u8
            + self.vendor.is_some() as u8
            + self.model.is_some() as u8
            + self.serial.is_some() as u8;
        (
            self.serial.is_some() as u8,
            vendor_model as u8,
            self.built_in.is_some() as u8,
            specified,
        )
    }
}

impl WindowConfig {
    /// Returns a conservative logical size used before a display is resolved.
    pub fn fallback_size(&self, ui: &UiConfig) -> (f64, f64) {
        (
            self.resolve_width(FALLBACK_DISPLAY_WIDTH),
            self.fallback_height(ui),
        )
    }

    /// Resolves launcher width for a display with the given logical width.
    pub fn resolve_width(&self, display_width: f64) -> f64 {
        clamp_axis(
            display_width * self.width_fraction,
            self.min_width,
            self.max_width,
        )
    }

    /// Clamps a preferred logical height to the configured guardrails.
    pub fn clamp_height(&self, preferred_height: f64) -> f64 {
        clamp_axis(preferred_height, self.min_height, self.max_height)
    }

    fn fallback_height(&self, ui: &UiConfig) -> f64 {
        self.clamp_height(ui.estimated_window_height(self.visible_rows, self.scale))
    }
}

impl Default for RankingConfig {
    fn default() -> Self {
        Self {
            tie_threshold: 120,
            provider_order: vec![
                "windows".to_owned(),
                "apps".to_owned(),
                "settings".to_owned(),
                "plugins".to_owned(),
            ],
            provider_score_boosts: HashMap::new(),
            score_rules: Vec::new(),
            result_limit: 24,
        }
    }
}

impl Default for WindowsProviderConfig {
    fn default() -> Self {
        Self {
            include_other_desktops: false,
            show_on_empty_query: true,
        }
    }
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            search_debounce_ms: 24,
        }
    }
}

impl RankingConfig {
    /// Returns the configured tie-break priority for a provider name.
    pub fn provider_rank(&self, provider: &str) -> usize {
        self.provider_order
            .iter()
            .position(|candidate| candidate == provider)
            .unwrap_or(self.provider_order.len() + 1)
    }

    /// Returns the configured additive score adjustment for a provider.
    pub fn provider_score_boost(&self, provider: &str) -> i64 {
        self.provider_score_boosts
            .get(provider)
            .copied()
            .unwrap_or(0)
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_header: true,
            cycle_selection: false,
            colorscheme: "system".to_owned(),
            font_family: "\"SF Pro Display\", \"Avenir Next\", \"Helvetica Neue\", sans-serif"
                .to_owned(),
            colorschemes: HashMap::new(),
            canvas: UiCanvasConfig::default(),
            entries: UiEntriesConfig::default(),
            shortcuts: UiShortcutsConfig::default(),
            font_sizes: UiFontSizesConfig::default(),
            layout: UiLayoutConfig::default(),
        }
    }
}

impl Default for AppsProviderConfig {
    fn default() -> Self {
        Self {
            exact_name_boost: 200,
            prefix_name_boost: 100,
        }
    }
}

impl Default for UiCanvasConfig {
    fn default() -> Self {
        Self {
            show: true,
            radius: 24,
            background_opacity: 0.97,
            chrome_opacity: 1.0,
        }
    }
}

impl Default for UiEntriesConfig {
    fn default() -> Self {
        Self { opacity: 1.0 }
    }
}

impl Default for UiShortcutsConfig {
    fn default() -> Self {
        Self {
            focus_window: default_focus_window_shortcut(),
            activate_all_windows: default_activate_all_windows_shortcut(),
        }
    }
}

fn default_focus_window_shortcut() -> Option<UiShortcutConfig> {
    Some(UiShortcutConfig {
        key: Some("Enter".to_owned()),
        code: None,
        alt: false,
        ctrl: false,
        meta: false,
        shift: false,
    })
}

fn default_activate_all_windows_shortcut() -> Option<UiShortcutConfig> {
    Some(UiShortcutConfig {
        key: Some("Enter".to_owned()),
        code: None,
        alt: true,
        ctrl: false,
        meta: false,
        shift: false,
    })
}

impl Default for UiFontSizesConfig {
    fn default() -> Self {
        Self {
            label: 10,
            input: 30,
            title: 16,
            subtitle: 12,
            badge: 11,
            accelerator: 12,
            config_error_title: 24,
            config_error_body: 15,
        }
    }
}

impl Default for UiLayoutConfig {
    fn default() -> Self {
        Self {
            section_gap: 14,
            input_padding_y: 14,
            input_padding_x: 18,
            input_radius: 18,
            list_gap: 8,
            entry_padding_y: 13,
            entry_padding_x: 14,
            entry_gap: 14,
            row_radius: 18,
            badge_size: 46,
            badge_radius: 14,
            icon_size: 46,
        }
    }
}

fn clamp_axis(value: f64, min: Option<f64>, max: Option<f64>) -> f64 {
    let value = min.map_or(value, |min| value.max(min));
    max.map_or(value, |max| value.min(max))
}

const FALLBACK_DISPLAY_WIDTH: f64 = 1920.0;
