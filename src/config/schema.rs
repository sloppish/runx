use std::collections::HashMap;

use anyhow::{Context, Result, anyhow, bail};
use global_hotkey::hotkey::{HotKey, Modifiers};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use toml::Table;

use super::shortcuts::{deserialize_ui_shortcut, parse_key, parse_modifier};

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
    /// Include windows from other macOS desktops/spaces in search results. Requires Screen Recording permission.
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
    pub(super) fn missing_required_fields(&self) -> Vec<&'static str> {
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
