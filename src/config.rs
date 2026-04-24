//! Loading and normalizing `config.toml`.
//!
//! The config layer owns both user-facing deserialization and the derived
//! runtime values that the rest of the launcher needs, such as resolved plugin
//! directories, command routes, and search paths.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result, anyhow, bail};
use directories::BaseDirs;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use miette::{
    GraphicalReportHandler, GraphicalTheme, LabeledSpan, MietteDiagnostic, NamedSource, Report,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value as JsonValue;
use toml::{Spanned, Table};

const DEFAULT_CONFIG: &str = r##"# Runx configuration
#
# `provider_order` controls which provider wins when scores are close.
# `tie_threshold` is the raw fuzzy-score delta that still counts as "similar".
# `empty_query_providers` controls which providers run before you type anything.
# `provider_score_boosts` lets you nudge merged scores per provider.
# `score_rules` lets you boost or demote specific result text patterns.
# `search_debounce_ms` and `render_coalesce_ms` tune search/render scheduling.

[hotkey]
key = "Space"
modifiers = ["Alt"]

[window]
width = 760
height = 520
hide_on_blur = true
always_on_top = true
show_on = "cursor"

[providers]
disabled = []

[providers.windows]
include_other_desktops = false

[providers.apps]
exact_name_boost = 200
prefix_name_boost = 100

[ranking]
tie_threshold = 120
provider_order = ["windows", "apps", "settings", "plugins"]
empty_query_providers = ["windows"]
result_limit = 24
# Example:
# [ranking.provider_score_boosts]
# apps = 60
# [[ranking.score_rules]]
# providers = ["apps", "windows"]
# field = "title"
# match = "contains"
# pattern = "spotify"
# boost = 120

[timing]
search_debounce_ms = 24
render_coalesce_ms = 8

[plugins]
directories = []
search_paths = []
# Example:
# search_paths = ["/opt/homebrew/bin"]

# Per-plugin configuration can live under `[plugin.<id>]`.
# Command routing can be configured under `[plugin.<id>.commands]`.
[ui]
show_header = true
cycle_selection = false
colorscheme = "system"
font_family = "\"SF Pro Display\", \"Avenir Next\", \"Helvetica Neue\", sans-serif"
# Built-in light/dark scheme overrides and custom schemes live under:
# [ui.colorschemes.builtin_light]
# [ui.colorschemes.builtin_dark]
# [ui.colorschemes.gruvbox]
# base = "builtin_dark"

[ui.canvas]
show = true
radius = 24
opacity = 1.0
background_opacity = 0.97

[ui.entries]
opacity = 1.0

[ui.shortcuts]
focus_window = "Enter"
activate_all_windows = "Option+Enter"

[ui.font_sizes]
label = 10
input = 30
title = 16
subtitle = 12
badge = 11
accelerator = 12
config_error_title = 24
config_error_body = 15

[ui.layout]
section_gap = 14
input_padding_y = 14
input_padding_x = 18
input_radius = 18
list_gap = 8
entry_padding_y = 13
entry_padding_x = 14
entry_gap = 14
row_radius = 18
badge_size = 46
badge_radius = 14
icon_size = 46
"##;

const KNOWN_PROVIDER_NAMES: [&str; 4] = ["windows", "apps", "settings", "plugins"];
const BUILTIN_COLORSCHEME_NAMES: [&str; 2] = ["builtin_light", "builtin_dark"];

/// Fully loaded configuration together with derived filesystem paths.
pub struct LoadedConfig {
    pub config: Config,
    pub config_path: PathBuf,
    pub plugin_dirs: Vec<PathBuf>,
    pub plugin_search_paths: Vec<PathBuf>,
}

/// Root configuration object deserialized from `config.toml`.
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Global launcher shortcut.
    pub hotkey: HotKeyConfig,
    /// Geometry and behavior of the Runx window itself.
    pub window: WindowConfig,
    /// Provider-specific search and activation behavior.
    pub providers: ProvidersConfig,
    /// Result ranking, tie-breaking, and empty-query behavior.
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
    /// Trigger key for the global launcher shortcut.
    pub key: String,
    /// Modifier keys that must be held with `key`.
    pub modifiers: Vec<String>,
}

/// Launcher window behavior and geometry.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct WindowConfig {
    /// Launcher width in logical pixels.
    pub width: f64,
    /// Launcher height in logical pixels.
    pub height: f64,
    /// Hide the launcher automatically when it loses focus.
    pub hide_on_blur: bool,
    /// Keep the launcher above normal windows while it is visible.
    pub always_on_top: bool,
    /// Choose which display Runx appears on when it opens.
    pub show_on: WindowDisplayTarget,
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
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct WindowsProviderConfig {
    /// Include windows from other macOS desktops/spaces in search results.
    pub include_other_desktops: bool,
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

/// Ranking and truncation rules for the merged result list.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RankingConfig {
    /// Maximum raw-score gap that still counts as a tie between providers.
    pub tie_threshold: i64,
    /// Tie-break priority when multiple providers return similarly scored items.
    pub provider_order: Vec<String>,
    /// Providers that should run before the query is non-empty.
    pub empty_query_providers: Vec<String>,
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

/// Debounce and coalescing timings for the search/render pipeline.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct TimingConfig {
    /// Delay before a changed query starts a new search generation.
    pub search_debounce_ms: u64,
    /// Small render delay used to coalesce provider updates.
    pub render_coalesce_ms: u64,
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
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UiColorOverridesConfig {
    /// Accent color used for highlighted details and emphasis.
    pub accent: Option<String>,
    /// Broad page/background tone used by the built-in palette.
    pub background: Option<String>,
    /// Panel surface color used by the built-in palette.
    pub panel: Option<String>,
    /// Primary foreground text color.
    pub text: Option<String>,
    /// Secondary or de-emphasized foreground text color.
    pub muted: Option<String>,
    /// Background fill for the outer launcher shell.
    pub shell_bg: Option<String>,
    /// Shadow for the outer launcher shell.
    pub shell_shadow: Option<String>,
    /// Border color for the outer launcher shell.
    pub shell_border: Option<String>,
    /// Stronger label color used for prominent small text.
    pub label_strong: Option<String>,
    /// Background of the search input field.
    pub input_bg: Option<String>,
    /// Border color of the search input field.
    pub input_border: Option<String>,
    /// Shadow of the search input field.
    pub input_shadow: Option<String>,
    /// Placeholder text color in the search input.
    pub placeholder: Option<String>,
    /// Scrollbar thumb color inside the results list.
    pub scrollbar: Option<String>,
    /// Default result-row background.
    pub item_bg: Option<String>,
    /// Result-row background on hover.
    pub item_hover: Option<String>,
    /// Background of the currently selected result row.
    pub item_selected_bg: Option<String>,
    /// Shadow of the currently selected result row.
    pub item_selected_shadow: Option<String>,
    /// Background of text badges.
    pub badge_bg: Option<String>,
    /// Border color of text badges.
    pub badge_border: Option<String>,
    /// Foreground text color of text badges.
    pub badge_text: Option<String>,
    /// Background behind icon badges.
    pub badge_icon_bg: Option<String>,
    /// Foreground color of accelerator chips.
    pub chip_text: Option<String>,
    /// Background color of accelerator chips.
    pub chip_bg: Option<String>,
    /// Border color of accelerator chips.
    pub chip_border: Option<String>,
    /// Background of the dedicated config-error panel.
    pub config_error_bg: Option<String>,
    /// Border color of the config-error panel.
    pub config_error_border: Option<String>,
    /// Shadow of the config-error panel.
    pub config_error_shadow: Option<String>,
    /// Title color used in the config-error view.
    pub config_error_title: Option<String>,
    /// Body text color used in the config-error view.
    pub config_error_copy: Option<String>,
    /// Input background when the outer canvas is disabled.
    pub canvas_hidden_input_bg: Option<String>,
    /// Input border when the outer canvas is disabled.
    pub canvas_hidden_input_border: Option<String>,
    /// Input shadow when the outer canvas is disabled.
    pub canvas_hidden_input_shadow: Option<String>,
    /// Result-row background when the outer canvas is disabled.
    pub canvas_hidden_item_bg: Option<String>,
    /// Result-row hover background when the outer canvas is disabled.
    pub canvas_hidden_item_hover: Option<String>,
    /// Selected result-row background when the outer canvas is disabled.
    pub canvas_hidden_item_selected_bg: Option<String>,
    /// Config-error panel background when the outer canvas is disabled.
    pub canvas_hidden_config_error_bg: Option<String>,
}

impl UiColorOverridesConfig {
    fn missing_required_fields(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();

        macro_rules! require {
            ($field:ident) => {
                if self.$field.is_none() {
                    missing.push(stringify!($field));
                }
            };
        }

        require!(accent);
        require!(background);
        require!(panel);
        require!(text);
        require!(muted);
        require!(shell_bg);
        require!(shell_shadow);
        require!(shell_border);
        require!(label_strong);
        require!(input_bg);
        require!(input_border);
        require!(input_shadow);
        require!(placeholder);
        require!(scrollbar);
        require!(item_bg);
        require!(item_hover);
        require!(item_selected_bg);
        require!(item_selected_shadow);
        require!(badge_bg);
        require!(badge_border);
        require!(badge_text);
        require!(badge_icon_bg);
        require!(chip_text);
        require!(chip_bg);
        require!(chip_border);
        require!(config_error_bg);
        require!(config_error_border);
        require!(config_error_shadow);
        require!(config_error_title);
        require!(config_error_copy);
        require!(canvas_hidden_input_bg);
        require!(canvas_hidden_input_border);
        require!(canvas_hidden_input_shadow);
        require!(canvas_hidden_item_bg);
        require!(canvas_hidden_item_hover);
        require!(canvas_hidden_item_selected_bg);
        require!(canvas_hidden_config_error_bg);

        missing
    }
}

/// Canvas tokens injected into the embedded UI theme.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UiCanvasConfig {
    /// Show the outer launcher panel behind the input and results.
    pub show: bool,
    /// Corner radius of the outer launcher panel.
    pub radius: u16,
    /// Overall opacity multiplier for the panel layer.
    pub opacity: f64,
    /// Opacity of the panel fill itself, separate from border/shadow opacity.
    pub background_opacity: f64,
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

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawConfigSpans {
    hotkey: RawHotKeySpans,
    window: WindowConfig,
    providers: RawProvidersSpans,
    ranking: RawRankingSpans,
    timing: TimingConfig,
    plugins: PluginsConfig,
    plugin: HashMap<String, Table>,
    ui: RawUiSpans,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawUiSpans {
    show_header: bool,
    cycle_selection: bool,
    colorscheme: Option<Spanned<String>>,
    font_family: String,
    colorschemes: HashMap<String, RawUiColorschemeSpans>,
    canvas: RawUiCanvasSpans,
    entries: RawUiEntriesSpans,
    shortcuts: UiShortcutsConfig,
    font_sizes: UiFontSizesConfig,
    layout: UiLayoutConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawUiColorschemeSpans {
    base: Option<Spanned<String>>,
    #[allow(dead_code)]
    #[serde(flatten)]
    overrides: UiColorOverridesConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawUiCanvasSpans {
    show: bool,
    radius: u16,
    opacity: Option<Spanned<f64>>,
    background_opacity: Option<Spanned<f64>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawUiEntriesSpans {
    opacity: Option<Spanned<f64>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawHotKeySpans {
    key: Option<Spanned<String>>,
    modifiers: Vec<Spanned<String>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawProvidersSpans {
    disabled: Vec<Spanned<String>>,
    windows: WindowsProviderConfig,
    apps: AppsProviderConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawRankingSpans {
    tie_threshold: i64,
    provider_order: Vec<Spanned<String>>,
    empty_query_providers: Vec<Spanned<String>>,
    provider_score_boosts: RawProviderScoreBoostsSpans,
    score_rules: Vec<RawRankingScoreRuleSpans>,
    result_limit: usize,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawProviderScoreBoostsSpans {
    windows: Option<i64>,
    apps: Option<i64>,
    settings: Option<i64>,
    plugins: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawRankingScoreRuleSpans {
    providers: Vec<Spanned<String>>,
    field: RankingScoreRuleField,
    #[serde(rename = "match")]
    match_kind: RankingScoreRuleMatchKind,
    pattern: String,
    boost: i64,
}

impl LoadedConfig {
    /// Loads the user config, writing the default template on first launch.
    pub fn load() -> Result<Self> {
        let (root_dir, plugin_dir, config_path) = runtime_paths()?;
        if !config_path.exists() {
            fs::write(&config_path, DEFAULT_CONFIG)
                .with_context(|| format!("failed to write {}", config_path.display()))?;
        }

        let raw = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let config: Config = toml::from_str(&raw)
            .map_err(|error| anyhow!(render_toml_parse_error(&config_path, &raw, &error)))?;
        let raw_spans: RawConfigSpans = toml::from_str(&raw)
            .map_err(|error| anyhow!(render_toml_parse_error(&config_path, &raw, &error)))?;
        validate_config_with_spans(&config_path, &raw, &raw_spans)?;
        validate_config(&config)?;

        Self::from_parts(config, root_dir, plugin_dir, config_path)
    }

    /// Builds a runtime config from built-in defaults without persisting anything.
    pub fn load_defaults() -> Result<Self> {
        let (root_dir, plugin_dir, config_path) = runtime_paths()?;
        Self::from_parts(Config::default(), root_dir, plugin_dir, config_path)
    }

    fn from_parts(
        config: Config,
        root_dir: PathBuf,
        plugin_dir: PathBuf,
        config_path: PathBuf,
    ) -> Result<Self> {
        let base_dirs =
            BaseDirs::new().context("could not resolve the current user's home directory")?;
        fs::create_dir_all(&plugin_dir)
            .with_context(|| format!("failed to create {}", plugin_dir.display()))?;

        let mut plugin_dirs = vec![plugin_dir];
        for configured in &config.plugins.directories {
            plugin_dirs.push(resolve_path(&root_dir, base_dirs.home_dir(), configured));
        }
        dedup_paths(&mut plugin_dirs);

        let mut plugin_search_paths = Vec::new();
        for configured in &config.plugins.search_paths {
            plugin_search_paths.push(resolve_path(&root_dir, base_dirs.home_dir(), configured));
        }
        dedup_paths(&mut plugin_search_paths);

        Ok(Self {
            config,
            config_path,
            plugin_dirs,
            plugin_search_paths,
        })
    }
}

fn validate_config(config: &Config) -> Result<()> {
    validate_provider_names(&config.providers.disabled, "[providers].disabled")?;
    validate_provider_names(&config.ranking.provider_order, "[ranking].provider_order")?;
    validate_provider_names(
        &config.ranking.empty_query_providers,
        "[ranking].empty_query_providers",
    )?;
    validate_opacity(
        config.ui.canvas.opacity,
        "[ui.canvas].opacity must be between 0.0 and 1.0",
    )?;
    validate_opacity(
        config.ui.canvas.background_opacity,
        "[ui.canvas].background_opacity must be between 0.0 and 1.0",
    )?;
    validate_opacity(
        config.ui.entries.opacity,
        "[ui.entries].opacity must be between 0.0 and 1.0",
    )?;
    validate_ui_colorscheme_selector(config)?;
    validate_ui_colorschemes(&config.ui.colorschemes)?;

    for provider in config.ranking.provider_score_boosts.keys() {
        validate_provider_name(provider, "[ranking.provider_score_boosts] key")?;
    }

    for (index, rule) in config.ranking.score_rules.iter().enumerate() {
        let context = format!("[[ranking.score_rules]] entry {}", index + 1);
        validate_provider_names(&rule.providers, &format!("{context}.providers"))?;
    }

    Ok(())
}

fn validate_config_with_spans(config_path: &Path, raw: &str, spans: &RawConfigSpans) -> Result<()> {
    if let Some(key) = &spans.hotkey.key
        && let Err(error) = parse_key(key.get_ref())
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            key.span(),
        )));
    }

    for modifier in &spans.hotkey.modifiers {
        if let Err(error) = parse_modifier(modifier.get_ref()) {
            return Err(anyhow!(render_config_validation_error(
                config_path,
                raw,
                &error.to_string(),
                modifier.span(),
            )));
        }
    }

    validate_spanned_provider_names(
        config_path,
        raw,
        &spans.providers.disabled,
        "[providers].disabled",
    )?;
    validate_spanned_provider_names(
        config_path,
        raw,
        &spans.ranking.provider_order,
        "[ranking].provider_order",
    )?;
    validate_spanned_provider_names(
        config_path,
        raw,
        &spans.ranking.empty_query_providers,
        "[ranking].empty_query_providers",
    )?;

    for (index, rule) in spans.ranking.score_rules.iter().enumerate() {
        validate_spanned_provider_names(
            config_path,
            raw,
            &rule.providers,
            &format!("[[ranking.score_rules]] entry {}.providers", index + 1),
        )?;
    }

    if let Some(opacity) = &spans.ui.canvas.opacity
        && let Err(error) = validate_opacity(
            *opacity.get_ref(),
            "[ui.canvas].opacity must be between 0.0 and 1.0",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            opacity.span(),
        )));
    }

    if let Some(opacity) = &spans.ui.canvas.background_opacity
        && let Err(error) = validate_opacity(
            *opacity.get_ref(),
            "[ui.canvas].background_opacity must be between 0.0 and 1.0",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            opacity.span(),
        )));
    }

    if let Some(opacity) = &spans.ui.entries.opacity
        && let Err(error) = validate_opacity(
            *opacity.get_ref(),
            "[ui.entries].opacity must be between 0.0 and 1.0",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            opacity.span(),
        )));
    }

    if let Some(colorscheme) = &spans.ui.colorscheme
        && let Err(error) = validate_ui_colorscheme_name_impl(
            colorscheme.get_ref(),
            spans.ui.colorschemes.keys().map(String::as_str),
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            colorscheme.span(),
        )));
    }

    for (name, scheme) in &spans.ui.colorschemes {
        if BUILTIN_COLORSCHEME_NAMES.contains(&name.as_str()) {
            if let Some(base) = &scheme.base {
                let message = format!(
                    "[ui.colorschemes.{name}].base is not allowed for built-in colorschemes"
                );
                return Err(anyhow!(render_config_validation_error(
                    config_path,
                    raw,
                    &message,
                    base.span(),
                )));
            }
            continue;
        }

        if let Some(base) = &scheme.base
            && !BUILTIN_COLORSCHEME_NAMES.contains(&base.get_ref().as_str())
        {
            let message = format!(
                "[ui.colorschemes.{name}].base must be one of: {}",
                BUILTIN_COLORSCHEME_NAMES.join(", ")
            );
            return Err(anyhow!(render_config_validation_error(
                config_path,
                raw,
                &message,
                base.span(),
            )));
        }
    }

    Ok(())
}

fn validate_opacity(opacity: f64, context: &str) -> Result<()> {
    if (0.0..=1.0).contains(&opacity) {
        Ok(())
    } else {
        bail!("{context}");
    }
}

fn validate_provider_names(providers: &[String], context: &str) -> Result<()> {
    for provider in providers {
        validate_provider_name(provider, context)?;
    }
    Ok(())
}

fn validate_provider_name(provider: &str, context: &str) -> Result<()> {
    if KNOWN_PROVIDER_NAMES.contains(&provider) {
        Ok(())
    } else {
        bail!(
            "{context} contains unknown provider `{provider}`. Expected one of: {}",
            KNOWN_PROVIDER_NAMES.join(", ")
        )
    }
}

fn validate_spanned_provider_names(
    config_path: &Path,
    raw: &str,
    providers: &[Spanned<String>],
    context: &str,
) -> Result<()> {
    for provider in providers {
        let name = provider.get_ref();
        if !KNOWN_PROVIDER_NAMES.contains(&name.as_str()) {
            let message = format!(
                "{context} contains unknown provider `{name}`. Expected one of: {}",
                KNOWN_PROVIDER_NAMES.join(", ")
            );
            return Err(anyhow!(render_config_validation_error(
                config_path,
                raw,
                &message,
                provider.span(),
            )));
        }
    }

    Ok(())
}

fn validate_ui_colorscheme_selector(config: &Config) -> Result<()> {
    validate_ui_colorscheme_name_impl(
        &config.ui.colorscheme,
        config.ui.colorschemes.keys().map(String::as_str),
    )
}

fn validate_ui_colorschemes(colorschemes: &HashMap<String, UiColorschemeConfig>) -> Result<()> {
    for (name, scheme) in colorschemes {
        if BUILTIN_COLORSCHEME_NAMES.contains(&name.as_str()) {
            if scheme.base.is_some() {
                bail!("[ui.colorschemes.{name}].base is not allowed for built-in colorschemes");
            }
            continue;
        }

        if let Some(base) = &scheme.base
            && !BUILTIN_COLORSCHEME_NAMES.contains(&base.as_str())
        {
            bail!(
                "[ui.colorschemes.{name}].base must be one of: {}",
                BUILTIN_COLORSCHEME_NAMES.join(", ")
            );
        }

        if scheme.base.is_none() {
            let missing = scheme.overrides.missing_required_fields();
            if !missing.is_empty() {
                bail!(
                    "[ui.colorschemes.{name}] has no base, so it must define every color token. Missing: {}",
                    missing.join(", ")
                );
            }
        }
    }
    Ok(())
}

fn validate_ui_colorscheme_name_impl<'a>(
    name: &str,
    available: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    if name == "system" {
        return Ok(());
    }

    if BUILTIN_COLORSCHEME_NAMES.contains(&name) {
        return Ok(());
    }

    if available.into_iter().any(|candidate| candidate == name) {
        Ok(())
    } else {
        bail!(
            "[ui].colorscheme must be `system`, one of the built-ins (`{}`), or a custom name under [ui.colorschemes.<name>]",
            BUILTIN_COLORSCHEME_NAMES.join("`, `")
        )
    }
}

fn render_toml_parse_error(config_path: &Path, raw: &str, error: &toml::de::Error) -> String {
    render_config_error_report(
        config_path,
        raw,
        "failed to parse",
        "runx::config::toml_parse",
        error.message(),
        error.span(),
    )
}

fn render_config_validation_error(
    config_path: &Path,
    raw: &str,
    message: &str,
    span: std::ops::Range<usize>,
) -> String {
    render_config_error_report(
        config_path,
        raw,
        "invalid configuration",
        "runx::config::validation",
        message,
        Some(span),
    )
}

fn render_config_error_report(
    config_path: &Path,
    raw: &str,
    title: &str,
    code: &str,
    message: &str,
    span: Option<std::ops::Range<usize>>,
) -> String {
    let mut diagnostic = MietteDiagnostic::new(format!("{title} {}", config_path.display()))
        .with_code(code)
        .with_help("Fix the highlighted config value, then save config.toml again.");

    if let Some(span) = span {
        diagnostic =
            diagnostic.with_label(LabeledSpan::new_with_span(Some(message.to_owned()), span));
    }

    let source = NamedSource::new(config_path.display().to_string(), raw.to_owned());
    let report = Report::new(diagnostic).with_source_code(source);
    let handler = GraphicalReportHandler::new_themed(GraphicalTheme::unicode_nocolor())
        .without_cause_chain()
        .with_context_lines(2)
        .with_width(100);
    let mut rendered = String::new();

    if handler
        .render_report(&mut rendered, report.as_ref())
        .is_ok()
    {
        rendered
    } else {
        format!("{title} {}: {message}", config_path.display())
    }
}

fn runtime_paths() -> Result<(PathBuf, PathBuf, PathBuf)> {
    let base_dirs =
        BaseDirs::new().context("could not resolve the current user's home directory")?;
    let root_dir = base_dirs.config_dir().join("runx");
    let plugin_dir = root_dir.join("plugins");
    let config_path = root_dir.join("config.toml");
    Ok((root_dir, plugin_dir, config_path))
}

/// Returns the current modification time for the config file, if it exists.
pub fn config_modified_at(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

impl Config {
    /// Converts the configured key/modifier pair into a `global_hotkey` binding.
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
}

impl Default for HotKeyConfig {
    fn default() -> Self {
        Self {
            key: "Space".to_owned(),
            modifiers: vec!["Alt".to_owned()],
        }
    }
}

impl HotKeyConfig {
    /// Parses the config strings into `global_hotkey` types.
    pub fn to_hotkey(&self) -> Result<HotKey> {
        let mut modifiers = Modifiers::empty();
        for modifier in &self.modifiers {
            modifiers |= parse_modifier(modifier)?;
        }
        let code = parse_key(&self.key)?;
        Ok(HotKey::new(Some(modifiers), code))
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 760.0,
            height: 520.0,
            hide_on_blur: true,
            always_on_top: true,
            show_on: WindowDisplayTarget::Cursor,
        }
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
            empty_query_providers: vec!["windows".to_owned()],
            provider_score_boosts: HashMap::new(),
            score_rules: Vec::new(),
            result_limit: 24,
        }
    }
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            search_debounce_ms: 24,
            render_coalesce_ms: 8,
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
        let mut colorschemes = HashMap::new();
        colorschemes.insert("builtin_light".to_owned(), UiColorschemeConfig::default());
        colorschemes.insert("builtin_dark".to_owned(), UiColorschemeConfig::default());
        Self {
            show_header: true,
            cycle_selection: false,
            colorscheme: "system".to_owned(),
            font_family: "\"SF Pro Display\", \"Avenir Next\", \"Helvetica Neue\", sans-serif"
                .to_owned(),
            colorschemes,
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
            opacity: 1.0,
            background_opacity: 0.97,
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

fn resolve_path(root_dir: &Path, home_dir: &Path, raw: &str) -> PathBuf {
    if let Some(stripped) = raw.strip_prefix("~/") {
        return home_dir.join(stripped);
    }

    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        root_dir.join(path)
    }
}

fn dedup_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

fn deserialize_ui_shortcut<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<UiShortcutConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    parse_ui_shortcut(&value).map_err(serde::de::Error::custom)
}

fn parse_ui_shortcut(value: &str) -> std::result::Result<Option<UiShortcutConfig>, String> {
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

fn parse_ui_shortcut_key(token: &str) -> Option<(Option<String>, Option<String>)> {
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

fn parse_modifier(value: &str) -> Result<Modifiers> {
    match value.to_ascii_lowercase().as_str() {
        "alt" | "option" => Ok(Modifiers::ALT),
        "control" | "ctrl" => Ok(Modifiers::CONTROL),
        "shift" => Ok(Modifiers::SHIFT),
        "command" | "cmd" | "super" | "meta" => Ok(Modifiers::META),
        other => bail!("unsupported hotkey modifier `{other}` in config.toml"),
    }
}

fn parse_key(value: &str) -> Result<Code> {
    let normalized = value.trim().to_ascii_uppercase();
    if normalized.len() == 1 {
        let Some(ch) = normalized.chars().next() else {
            bail!("hotkey key must not be empty in config.toml");
        };
        return match ch {
            'A' => Ok(Code::KeyA),
            'B' => Ok(Code::KeyB),
            'C' => Ok(Code::KeyC),
            'D' => Ok(Code::KeyD),
            'E' => Ok(Code::KeyE),
            'F' => Ok(Code::KeyF),
            'G' => Ok(Code::KeyG),
            'H' => Ok(Code::KeyH),
            'I' => Ok(Code::KeyI),
            'J' => Ok(Code::KeyJ),
            'K' => Ok(Code::KeyK),
            'L' => Ok(Code::KeyL),
            'M' => Ok(Code::KeyM),
            'N' => Ok(Code::KeyN),
            'O' => Ok(Code::KeyO),
            'P' => Ok(Code::KeyP),
            'Q' => Ok(Code::KeyQ),
            'R' => Ok(Code::KeyR),
            'S' => Ok(Code::KeyS),
            'T' => Ok(Code::KeyT),
            'U' => Ok(Code::KeyU),
            'V' => Ok(Code::KeyV),
            'W' => Ok(Code::KeyW),
            'X' => Ok(Code::KeyX),
            'Y' => Ok(Code::KeyY),
            'Z' => Ok(Code::KeyZ),
            '0' => Ok(Code::Digit0),
            '1' => Ok(Code::Digit1),
            '2' => Ok(Code::Digit2),
            '3' => Ok(Code::Digit3),
            '4' => Ok(Code::Digit4),
            '5' => Ok(Code::Digit5),
            '6' => Ok(Code::Digit6),
            '7' => Ok(Code::Digit7),
            '8' => Ok(Code::Digit8),
            '9' => Ok(Code::Digit9),
            _ => bail!("unsupported hotkey key `{value}` in config.toml"),
        };
    }

    match normalized.as_str() {
        "SPACE" => Ok(Code::Space),
        "ENTER" | "RETURN" => Ok(Code::Enter),
        "ESC" | "ESCAPE" => Ok(Code::Escape),
        "TAB" => Ok(Code::Tab),
        "BACKSPACE" => Ok(Code::Backspace),
        "UP" => Ok(Code::ArrowUp),
        "DOWN" => Ok(Code::ArrowDown),
        "LEFT" => Ok(Code::ArrowLeft),
        "RIGHT" => Ok(Code::ArrowRight),
        other => bail!("unsupported hotkey key `{other}` in config.toml"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Config, WindowDisplayTarget, parse_key, parse_modifier, render_toml_parse_error};
    use global_hotkey::hotkey::{Code, Modifiers};

    mod parse_key_tests {
        use super::{Code, parse_key};

        #[test]
        fn accepts_single_letter_keys() {
            assert!(matches!(parse_key("a"), Ok(Code::KeyA)));
        }

        #[test]
        fn rejects_empty_keys() {
            let error = parse_key("").expect_err("empty key should fail");
            assert!(error.to_string().contains("unsupported hotkey key"));
        }
    }

    mod parse_modifier_tests {
        use super::{Modifiers, parse_modifier};

        #[test]
        fn accepts_common_aliases() {
            assert!(matches!(parse_modifier("option"), Ok(Modifiers::ALT)));
            assert!(matches!(parse_modifier("cmd"), Ok(Modifiers::META)));
        }
    }

    mod window_display_target_tests {
        use super::{Config, WindowDisplayTarget};

        #[test]
        fn defaults_to_cursor() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert_eq!(config.window.show_on, WindowDisplayTarget::Cursor);
        }

        #[test]
        fn accepts_cursor() {
            let cursor: Config =
                toml::from_str("[window]\nshow_on = \"cursor\"\n").expect("cursor should parse");
            assert_eq!(cursor.window.show_on, WindowDisplayTarget::Cursor);
        }

        #[test]
        fn defaults_to_current_windows_provider_settings() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert!(!config.providers.windows.include_other_desktops);
        }

        #[test]
        fn accepts_include_other_desktops() {
            let config: Config =
                toml::from_str("[providers.windows]\ninclude_other_desktops = true\n")
                    .expect("include_other_desktops should parse");
            assert!(config.providers.windows.include_other_desktops);
        }

        #[test]
        fn partial_provider_tables_keep_disabled_default() {
            let config: Config =
                toml::from_str("[providers.windows]\ninclude_other_desktops = true\n")
                    .expect("partial provider config should parse");
            assert!(config.providers.disabled.is_empty());
        }

        #[test]
        fn defaults_to_no_disabled_providers() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert!(config.providers.disabled.is_empty());
        }

        #[test]
        fn accepts_disabled_provider_list() {
            let config: Config = toml::from_str("[providers]\ndisabled = [\"windows\"]\n")
                .expect("disabled providers should parse");
            assert_eq!(config.providers.disabled, vec!["windows"]);
        }
    }

    mod apps_provider_config_tests {
        use super::Config;

        #[test]
        fn defaults_to_current_app_name_boosts() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert_eq!(config.providers.apps.exact_name_boost, 200);
            assert_eq!(config.providers.apps.prefix_name_boost, 100);
        }

        #[test]
        fn accepts_custom_app_name_boosts() {
            let config: Config = toml::from_str(
                "[providers.apps]\nexact_name_boost = 350\nprefix_name_boost = 80\n",
            )
            .expect("app boosts should parse");
            assert_eq!(config.providers.apps.exact_name_boost, 350);
            assert_eq!(config.providers.apps.prefix_name_boost, 80);
        }
    }

    mod empty_query_provider_tests {
        use super::Config;
        use crate::config::{
            RankingScoreRule, RankingScoreRuleField, RankingScoreRuleMatchKind, validate_config,
        };

        #[test]
        fn defaults_to_windows_only() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert_eq!(config.ranking.empty_query_providers, vec!["windows"]);
        }

        #[test]
        fn accepts_custom_provider_list() {
            let config: Config =
                toml::from_str("[ranking]\nempty_query_providers = [\"windows\", \"plugins\"]\n")
                    .expect("custom empty query providers should parse");
            assert_eq!(
                config.ranking.empty_query_providers,
                vec!["windows", "plugins"]
            );
        }

        #[test]
        fn accepts_provider_score_boosts() {
            let config: Config =
                toml::from_str("[ranking.provider_score_boosts]\nsettings = -150\napps = 60\n")
                    .expect("provider score boosts should parse");
            assert_eq!(config.ranking.provider_score_boost("settings"), -150);
            assert_eq!(config.ranking.provider_score_boost("apps"), 60);
            assert_eq!(config.ranking.provider_score_boost("windows"), 0);
        }

        #[test]
        fn accepts_score_rules() {
            let config: Config = toml::from_str(
                "[[ranking.score_rules]]\nproviders = [\"apps\", \"windows\"]\nfield = \"title\"\nmatch = \"contains\"\npattern = \"spotify\"\nboost = 120\n",
            )
            .expect("score rules should parse");
            let rule = &config.ranking.score_rules[0];
            assert_eq!(rule.providers, vec!["apps", "windows"]);
            assert_eq!(rule.field, super::super::RankingScoreRuleField::Title);
            assert_eq!(
                rule.match_kind,
                super::super::RankingScoreRuleMatchKind::Contains
            );
            assert_eq!(rule.pattern, "spotify");
            assert_eq!(rule.boost, 120);
        }

        #[test]
        fn rejects_unknown_top_level_section() {
            let error =
                toml::from_str::<Config>("[windo]\nwidth = 760\n").expect_err("typo should fail");
            assert!(error.message().contains("unknown field `windo`"));
        }

        #[test]
        fn rejects_unknown_nested_field() {
            let error = toml::from_str::<Config>("[window]\nwidt = 760\n")
                .expect_err("unknown nested field should fail");
            assert!(error.message().contains("unknown field `widt`"));
        }

        #[test]
        fn rejects_unknown_provider_in_provider_order() {
            let mut config = Config::default();
            config.ranking.provider_order = vec!["windows".to_owned(), "asdfasdf".to_owned()];

            let error = validate_config(&config).expect_err("unknown provider should fail");
            assert!(
                error
                    .to_string()
                    .contains("[ranking].provider_order contains unknown provider `asdfasdf`")
            );
        }

        #[test]
        fn rejects_unknown_provider_in_empty_query_providers() {
            let mut config = Config::default();
            config.ranking.empty_query_providers = vec!["asdfasdf".to_owned()];

            let error = validate_config(&config).expect_err("unknown provider should fail");
            assert!(
                error.to_string().contains(
                    "[ranking].empty_query_providers contains unknown provider `asdfasdf`"
                )
            );
        }

        #[test]
        fn rejects_unknown_provider_in_disabled_providers() {
            let mut config = Config::default();
            config.providers.disabled = vec!["apps".to_owned(), "asdfasdf".to_owned()];

            let error = validate_config(&config).expect_err("unknown provider should fail");
            assert!(
                error
                    .to_string()
                    .contains("[providers].disabled contains unknown provider `asdfasdf`")
            );
        }

        #[test]
        fn rejects_unknown_provider_in_provider_score_boosts() {
            let mut config = Config::default();
            config
                .ranking
                .provider_score_boosts
                .insert("asdfasdf".to_owned(), 10);

            let error = validate_config(&config).expect_err("unknown provider should fail");
            assert!(error.to_string().contains(
                "[ranking.provider_score_boosts] key contains unknown provider `asdfasdf`"
            ));
        }

        #[test]
        fn rejects_unknown_provider_in_score_rules() {
            let mut config = Config::default();
            config.ranking.score_rules = vec![RankingScoreRule {
                providers: vec!["asdfasdf".to_owned()],
                field: RankingScoreRuleField::Title,
                match_kind: RankingScoreRuleMatchKind::Contains,
                pattern: "spotify".to_owned(),
                boost: 120,
            }];

            let error = validate_config(&config).expect_err("unknown provider should fail");
            assert!(error.to_string().contains(
                "[[ranking.score_rules]] entry 1.providers contains unknown provider `asdfasdf`"
            ));
        }
    }

    mod timing_tests {
        use super::Config;

        #[test]
        fn defaults_to_current_search_and_render_timings() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert_eq!(config.timing.search_debounce_ms, 24);
            assert_eq!(config.timing.render_coalesce_ms, 8);
        }

        #[test]
        fn accepts_custom_timing_values() {
            let config: Config =
                toml::from_str("[timing]\nsearch_debounce_ms = 32\nrender_coalesce_ms = 12\n")
                    .expect("custom timing values should parse");
            assert_eq!(config.timing.search_debounce_ms, 32);
            assert_eq!(config.timing.render_coalesce_ms, 12);
        }
    }

    mod ui_tests {
        use super::Config;
        use crate::config::{UiColorOverridesConfig, UiColorschemeConfig, validate_config};

        fn complete_color_overrides() -> UiColorOverridesConfig {
            macro_rules! token {
                () => {
                    Some("#000000".to_owned())
                };
            }

            UiColorOverridesConfig {
                accent: token!(),
                background: token!(),
                panel: token!(),
                text: token!(),
                muted: token!(),
                shell_bg: token!(),
                shell_shadow: token!(),
                shell_border: token!(),
                label_strong: token!(),
                input_bg: token!(),
                input_border: token!(),
                input_shadow: token!(),
                placeholder: token!(),
                scrollbar: token!(),
                item_bg: token!(),
                item_hover: token!(),
                item_selected_bg: token!(),
                item_selected_shadow: token!(),
                badge_bg: token!(),
                badge_border: token!(),
                badge_text: token!(),
                badge_icon_bg: token!(),
                chip_text: token!(),
                chip_bg: token!(),
                chip_border: token!(),
                config_error_bg: token!(),
                config_error_border: token!(),
                config_error_shadow: token!(),
                config_error_title: token!(),
                config_error_copy: token!(),
                canvas_hidden_input_bg: token!(),
                canvas_hidden_input_border: token!(),
                canvas_hidden_input_shadow: token!(),
                canvas_hidden_item_bg: token!(),
                canvas_hidden_item_hover: token!(),
                canvas_hidden_item_selected_bg: token!(),
                canvas_hidden_config_error_bg: token!(),
            }
        }

        #[test]
        fn defaults_to_current_ui_font_sizes() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert!(config.ui.show_header);
            assert!(!config.ui.cycle_selection);
            assert_eq!(config.ui.colorscheme, "system");
            assert!(config.ui.canvas.show);
            assert_eq!(config.ui.canvas.radius, 24);
            assert_eq!(config.ui.canvas.opacity, 1.0);
            assert_eq!(config.ui.canvas.background_opacity, 0.97);
            assert_eq!(config.ui.entries.opacity, 1.0);
            assert_eq!(
                config.ui.shortcuts.focus_window,
                Some(crate::config::UiShortcutConfig {
                    key: Some("Enter".to_owned()),
                    code: None,
                    alt: false,
                    ctrl: false,
                    meta: false,
                    shift: false,
                })
            );
            assert_eq!(
                config.ui.shortcuts.activate_all_windows,
                Some(crate::config::UiShortcutConfig {
                    key: Some("Enter".to_owned()),
                    code: None,
                    alt: true,
                    ctrl: false,
                    meta: false,
                    shift: false,
                })
            );
            assert!(config.ui.colorschemes.contains_key("builtin_light"));
            assert!(config.ui.colorschemes.contains_key("builtin_dark"));
            assert_eq!(config.ui.font_sizes.label, 10);
            assert_eq!(config.ui.font_sizes.input, 30);
            assert_eq!(config.ui.font_sizes.title, 16);
            assert_eq!(config.ui.font_sizes.subtitle, 12);
            assert_eq!(config.ui.font_sizes.badge, 11);
            assert_eq!(config.ui.font_sizes.accelerator, 12);
            assert_eq!(config.ui.font_sizes.config_error_title, 24);
            assert_eq!(config.ui.font_sizes.config_error_body, 15);
            assert_eq!(config.ui.layout.input_padding_y, 14);
            assert_eq!(config.ui.layout.input_padding_x, 18);
            assert_eq!(config.ui.layout.input_radius, 18);
            assert_eq!(config.ui.layout.section_gap, 14);
            assert_eq!(config.ui.layout.list_gap, 8);
            assert_eq!(config.ui.layout.entry_padding_y, 13);
            assert_eq!(config.ui.layout.entry_padding_x, 14);
            assert_eq!(config.ui.layout.entry_gap, 14);
            assert_eq!(config.ui.layout.row_radius, 18);
            assert_eq!(config.ui.layout.badge_size, 46);
            assert_eq!(config.ui.layout.badge_radius, 14);
            assert_eq!(config.ui.layout.icon_size, 46);
        }

        #[test]
        fn accepts_custom_ui_font_sizes() {
            let config: Config = toml::from_str(
                "[ui]\nshow_header = false\ncycle_selection = true\ncolorscheme = \"builtin_dark\"\n[ui.canvas]\nshow = false\nradius = 18\nopacity = 0.75\nbackground_opacity = 0.9\n[ui.entries]\nopacity = 0.64\n[ui.font_sizes]\ninput = 34\ntitle = 18\nsubtitle = 13\nbadge = 12\naccelerator = 13\nconfig_error_title = 28\nconfig_error_body = 17\nlabel = 11\n",
            )
            .expect("custom ui font sizes should parse");

            assert!(!config.ui.show_header);
            assert!(config.ui.cycle_selection);
            assert_eq!(config.ui.colorscheme, "builtin_dark");
            assert!(!config.ui.canvas.show);
            assert_eq!(config.ui.canvas.radius, 18);
            assert_eq!(config.ui.canvas.opacity, 0.75);
            assert_eq!(config.ui.canvas.background_opacity, 0.9);
            assert_eq!(config.ui.entries.opacity, 0.64);
            assert_eq!(config.ui.font_sizes.label, 11);
            assert_eq!(config.ui.font_sizes.input, 34);
            assert_eq!(config.ui.font_sizes.title, 18);
            assert_eq!(config.ui.font_sizes.subtitle, 13);
            assert_eq!(config.ui.font_sizes.badge, 12);
            assert_eq!(config.ui.font_sizes.accelerator, 13);
            assert_eq!(config.ui.font_sizes.config_error_title, 28);
            assert_eq!(config.ui.font_sizes.config_error_body, 17);
        }

        #[test]
        fn accepts_custom_ui_layout() {
            let config: Config = toml::from_str(
                "[ui.layout]\nsection_gap = 16\ninput_padding_y = 16\ninput_padding_x = 20\ninput_radius = 20\nlist_gap = 10\nentry_padding_y = 14\nentry_padding_x = 16\nentry_gap = 15\nrow_radius = 20\nbadge_size = 48\nbadge_radius = 16\nicon_size = 40\n",
            )
            .expect("custom ui layout should parse");

            assert_eq!(config.ui.layout.section_gap, 16);
            assert_eq!(config.ui.layout.input_padding_y, 16);
            assert_eq!(config.ui.layout.input_padding_x, 20);
            assert_eq!(config.ui.layout.input_radius, 20);
            assert_eq!(config.ui.layout.list_gap, 10);
            assert_eq!(config.ui.layout.entry_padding_y, 14);
            assert_eq!(config.ui.layout.entry_padding_x, 16);
            assert_eq!(config.ui.layout.entry_gap, 15);
            assert_eq!(config.ui.layout.row_radius, 20);
            assert_eq!(config.ui.layout.badge_size, 48);
            assert_eq!(config.ui.layout.badge_radius, 16);
            assert_eq!(config.ui.layout.icon_size, 40);
        }

        #[test]
        fn accepts_custom_ui_shortcut() {
            let config: Config = toml::from_str(
                "[ui.shortcuts]\nfocus_window = \"Cmd+Enter\"\nactivate_all_windows = \"Cmd+Shift+Enter\"\n",
            )
            .expect("custom UI shortcut should parse");

            assert_eq!(
                config.ui.shortcuts.focus_window,
                Some(crate::config::UiShortcutConfig {
                    key: Some("Enter".to_owned()),
                    code: None,
                    alt: false,
                    ctrl: false,
                    meta: true,
                    shift: false,
                })
            );
            assert_eq!(
                config.ui.shortcuts.activate_all_windows,
                Some(crate::config::UiShortcutConfig {
                    key: Some("Enter".to_owned()),
                    code: None,
                    alt: false,
                    ctrl: false,
                    meta: true,
                    shift: true,
                })
            );
        }

        #[test]
        fn accepts_disabled_ui_shortcut() {
            let config: Config = toml::from_str(
                "[ui.shortcuts]\nfocus_window = \"none\"\nactivate_all_windows = \"none\"\n",
            )
            .expect("disabled UI shortcut should parse");

            assert_eq!(config.ui.shortcuts.focus_window, None);
            assert_eq!(config.ui.shortcuts.activate_all_windows, None);
        }

        #[test]
        fn accepts_full_ui_color_overrides() {
            let config: Config = toml::from_str(
                "[ui]\ncolorscheme = \"gruvbox\"\n[ui.colorschemes.builtin_light]\naccent = \"#111111\"\nshell_bg = \"linear-gradient(180deg, #111, #222)\"\nconfig_error_title = \"#fefefe\"\ncanvas_hidden_input_bg = \"#222222\"\n[ui.colorschemes.builtin_dark]\nitem_bg = \"rgba(1,2,3,0.4)\"\nbadge_icon_bg = \"rgba(0,0,0,0.9)\"\n[ui.colorschemes.gruvbox]\nbase = \"builtin_dark\"\npanel = \"#282828\"\ntext = \"#ebdbb2\"\n",
            )
            .expect("custom ui color overrides should parse");

            assert_eq!(config.ui.colorscheme, "gruvbox");
            assert_eq!(
                config
                    .ui
                    .colorschemes
                    .get("builtin_light")
                    .and_then(|scheme| scheme.overrides.accent.as_deref()),
                Some("#111111")
            );
            assert_eq!(
                config
                    .ui
                    .colorschemes
                    .get("builtin_light")
                    .and_then(|scheme| scheme.overrides.shell_bg.as_deref()),
                Some("linear-gradient(180deg, #111, #222)")
            );
            assert_eq!(
                config
                    .ui
                    .colorschemes
                    .get("builtin_light")
                    .and_then(|scheme| scheme.overrides.config_error_title.as_deref()),
                Some("#fefefe")
            );
            assert_eq!(
                config
                    .ui
                    .colorschemes
                    .get("builtin_light")
                    .and_then(|scheme| scheme.overrides.canvas_hidden_input_bg.as_deref()),
                Some("#222222")
            );
            assert_eq!(
                config
                    .ui
                    .colorschemes
                    .get("builtin_dark")
                    .and_then(|scheme| scheme.overrides.item_bg.as_deref()),
                Some("rgba(1,2,3,0.4)")
            );
            assert_eq!(
                config
                    .ui
                    .colorschemes
                    .get("builtin_dark")
                    .and_then(|scheme| scheme.overrides.badge_icon_bg.as_deref()),
                Some("rgba(0,0,0,0.9)")
            );
            assert_eq!(
                config.ui.colorschemes.get("gruvbox"),
                Some(&UiColorschemeConfig {
                    base: Some("builtin_dark".to_owned()),
                    overrides: UiColorOverridesConfig {
                        panel: Some("#282828".to_owned()),
                        text: Some("#ebdbb2".to_owned()),
                        ..UiColorOverridesConfig::default()
                    },
                })
            );
        }

        #[test]
        fn rejects_legacy_color_override_fields() {
            for raw in [
                "[ui]\naccent = \"#333333\"\n",
                "[ui.colors]\naccent = \"#111111\"\n",
                "[ui.dark_colors]\nitem_bg = \"rgba(1,2,3,0.4)\"\n",
            ] {
                let error =
                    toml::from_str::<Config>(raw).expect_err("legacy color override should fail");
                assert!(error.message().contains("unknown field"));
            }
        }

        #[test]
        fn rejects_unknown_selected_colorscheme() {
            let mut config = Config::default();
            config.ui.colorscheme = "gruvbox".to_owned();

            let error = validate_config(&config).expect_err("unknown colorscheme should fail");
            assert!(
                error
                    .to_string()
                    .contains("[ui].colorscheme must be `system`")
            );
        }

        #[test]
        fn rejects_invalid_custom_colorscheme_base() {
            let mut config = Config::default();
            config.ui.colorschemes.insert(
                "gruvbox".to_owned(),
                UiColorschemeConfig {
                    base: Some("nope".to_owned()),
                    ..UiColorschemeConfig::default()
                },
            );

            let error = validate_config(&config).expect_err("invalid colorscheme base should fail");
            assert!(error.to_string().contains(
                "[ui.colorschemes.gruvbox].base must be one of: builtin_light, builtin_dark"
            ));
        }

        #[test]
        fn accepts_complete_custom_colorscheme_without_base() {
            let mut config = Config::default();
            config.ui.colorschemes.insert(
                "gruvbox".to_owned(),
                UiColorschemeConfig {
                    base: None,
                    overrides: complete_color_overrides(),
                },
            );

            validate_config(&config).expect("complete custom colorscheme should pass");
        }

        #[test]
        fn rejects_partial_custom_colorscheme_without_base() {
            let mut config = Config::default();
            config.ui.colorschemes.insert(
                "gruvbox".to_owned(),
                UiColorschemeConfig {
                    base: None,
                    overrides: UiColorOverridesConfig {
                        accent: Some("#fabd2f".to_owned()),
                        ..UiColorOverridesConfig::default()
                    },
                },
            );

            let error = validate_config(&config)
                .expect_err("partial custom colorscheme without base should fail");
            assert!(
                error
                    .to_string()
                    .contains("[ui.colorschemes.gruvbox] has no base")
            );
            assert!(error.to_string().contains("background"));
        }
    }

    mod diagnostics_tests {
        use super::{Config, Path, render_toml_parse_error};
        use crate::config::{RawConfigSpans, validate_config_with_spans};

        #[test]
        fn toml_parse_errors_include_source_context() {
            let raw = "[[ranking.score_rules]]\nboost = \"oops\"\n";
            let error = toml::from_str::<Config>(raw).expect_err("config should fail to parse");
            let rendered = render_toml_parse_error(Path::new("/tmp/runx-config.toml"), raw, &error);

            assert!(rendered.contains("failed to parse /tmp/runx-config.toml"));
            assert!(rendered.contains("boost = \"oops\""));
            assert!(rendered.contains("expected i64"));
        }

        #[test]
        fn validation_errors_include_source_context() {
            let raw = "[ranking]\nprovider_order = [\"windows\", \"asdfasdf\"]\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("invalid configuration /tmp/runx-config.toml"));
            assert!(rendered.contains("provider_order = [\"windows\", \"asdfasdf\"]"));
            assert!(rendered.contains("unknown provider `asdfasdf`"));
        }

        #[test]
        fn canvas_opacity_validation_errors_include_source_context() {
            let raw = "[ui.canvas]\nopacity = 1.2\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("invalid configuration /tmp/runx-config.toml"));
            assert!(rendered.contains("opacity = 1.2"));
            assert!(rendered.contains("[ui.canvas].opacity must be between 0.0 and 1.0"));
        }

        #[test]
        fn canvas_background_opacity_validation_errors_include_source_context() {
            let raw = "[ui.canvas]\nbackground_opacity = 1.2\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("invalid configuration /tmp/runx-config.toml"));
            assert!(rendered.contains("background_opacity = 1.2"));
            assert!(
                rendered.contains("[ui.canvas].background_opacity must be between 0.0 and 1.0",)
            );
        }

        #[test]
        fn entry_opacity_validation_errors_include_source_context() {
            let raw = "[ui.entries]\nopacity = -0.1\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("invalid configuration /tmp/runx-config.toml"));
            assert!(rendered.contains("opacity = -0.1"));
            assert!(rendered.contains("[ui.entries].opacity must be between 0.0 and 1.0"));
        }

        #[test]
        fn colorscheme_validation_errors_include_source_context() {
            let raw = "[ui]\ncolorscheme = \"gruvbox\"\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("invalid configuration /tmp/runx-config.toml"));
            assert!(rendered.contains("colorscheme = \"gruvbox\""));
            assert!(rendered.contains("[ui].colorscheme must be `system`"));
        }

        #[test]
        fn raw_spans_accept_full_ui_block_for_validation() {
            let raw = r##"
[ui]
show_header = true
cycle_selection = false
colorscheme = "system"
font_family = "\"SF Pro Display\", \"Avenir Next\", \"Helvetica Neue\", sans-serif"

[ui.colorschemes.builtin_light]
accent = "#c77b49"
background = "#f3ede5"
panel = "#fffaf3"
text = "#1f1a16"
muted = "#756759"

[ui.canvas]
show = true
radius = 24
opacity = 1.0
background_opacity = 0.97

[ui.entries]
opacity = 0.92

[ui.font_sizes]
label = 10
input = 20
title = 16
subtitle = 12
badge = 11
accelerator = 12
config_error_title = 24
config_error_body = 15

[ui.layout]
section_gap = 14
input_padding_y = 8
input_padding_x = 8
input_radius = 0
list_gap = 2
entry_padding_y = 5
entry_padding_x = 5
entry_gap = 8
row_radius = 0
badge_size = 46
badge_radius = 14
icon_size = 46
"##;
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans should accept a valid ui block");
            validate_config_with_spans(Path::new("/tmp/runx-config.toml"), raw, &spans)
                .expect("validation should pass");
        }

        #[test]
        fn raw_spans_reject_unknown_ui_fields() {
            let raw = r##"
[ui]
show_header = true
colorscheme = "system"
font_family = "\"SF Pro Display\", \"Avenir Next\", \"Helvetica Neue\", sans-serif"
bogus = true
"##;

            let error =
                toml::from_str::<RawConfigSpans>(raw).expect_err("unknown ui field should fail");
            assert!(error.message().contains("unknown field `bogus`"));
        }
    }
}
