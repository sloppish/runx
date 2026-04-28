//! Loading and normalizing `config.toml`.
//!
//! The config layer owns both user-facing deserialization and the derived
//! runtime values that the rest of the launcher needs, such as resolved plugin
//! directories, command routes, and search paths.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use directories::BaseDirs;

mod defaults;
mod errors;
mod paths;
mod schema;
mod shortcuts;
mod validation;

pub use defaults::{BUILTIN_COLORSCHEME_NAMES, KNOWN_PROVIDER_NAMES};
pub use paths::config_modified_at;
pub use schema::*;

use defaults::DEFAULT_CONFIG;
use errors::render_toml_parse_error;
use paths::{dedup_paths, resolve_path, runtime_paths};
use validation::{RawConfigSpans, validate_config, validate_config_with_spans};

/// Fully loaded configuration together with derived filesystem paths.
pub struct LoadedConfig {
    pub config: Config,
    pub config_path: PathBuf,
    pub plugin_dirs: Vec<PathBuf>,
    pub plugin_search_paths: Vec<PathBuf>,
}

impl LoadedConfig {
    /// Loads the user config, writing the default template on first launch.
    pub fn load() -> Result<Self> {
        let (root_dir, plugin_dir, config_path) = runtime_paths()?;
        ensure_user_config()?;

        let raw = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let config = validate_config_toml(&config_path, &raw)?;

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

/// Ensures that the user config exists and returns its path.
pub fn ensure_user_config() -> Result<PathBuf> {
    let (root_dir, _, config_path) = runtime_paths()?;
    fs::create_dir_all(&root_dir)
        .with_context(|| format!("failed to create {}", root_dir.display()))?;
    if !config_path.exists() {
        fs::write(&config_path, DEFAULT_CONFIG)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
    }
    Ok(config_path)
}

/// Parses and validates a raw Runx config document.
pub fn validate_config_toml(config_path: &Path, raw: &str) -> Result<Config> {
    let config: Config = toml::from_str(raw)
        .map_err(|error| anyhow!(render_toml_parse_error(config_path, raw, &error)))?;
    let raw_spans: RawConfigSpans = toml::from_str(raw)
        .map_err(|error| anyhow!(render_toml_parse_error(config_path, raw, &error)))?;
    validate_config_with_spans(config_path, raw, &raw_spans)?;
    validate_config(&config)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Config, WindowDisplayTarget, render_toml_parse_error, validate_config_toml};

    mod hotkey_tests {
        use std::path::Path;

        #[test]
        fn defaults_to_option_space() {
            let config = super::Config::default();
            assert_eq!(config.hotkey.shortcut, "Option+Space");
            config.hotkey().expect("default hotkey should parse");
        }

        #[test]
        fn accepts_shortcut_string() {
            let config = super::validate_config_toml(
                Path::new("/tmp/runx-test-config.toml"),
                "[hotkey]\nshortcut = \"Option+KeyK\"\n",
            )
            .expect("shortcut hotkey should parse");
            assert_eq!(config.hotkey.shortcut, "Option+KeyK");
        }

        #[test]
        fn rejects_invalid_shortcut_string() {
            let error = super::validate_config_toml(
                Path::new("/tmp/runx-test-config.toml"),
                "[hotkey]\nshortcut = \"Option+NotAKey\"\n",
            )
            .expect_err("invalid hotkey should fail");
            assert!(error.to_string().contains("unsupported hotkey shortcut"));
        }
    }

    mod window_display_target_tests {
        use std::path::Path;

        use super::{Config, WindowDisplayTarget};
        use crate::{config::validate_config_toml, displays::DisplayProfile};

        #[test]
        fn defaults_to_cursor() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert_eq!(config.window.show_on, WindowDisplayTarget::Cursor);
            assert_eq!(config.window.width_fraction, 0.4);
            assert_eq!(config.window.visible_rows, 5);
            assert_eq!(config.window.min_width, None);
            assert_eq!(config.window.max_width, None);
            assert_eq!(config.window.min_height, None);
            assert_eq!(config.window.max_height, None);
        }

        #[test]
        fn accepts_cursor() {
            let cursor: Config =
                toml::from_str("[window]\nshow_on = \"cursor\"\n").expect("cursor should parse");
            assert_eq!(cursor.window.show_on, WindowDisplayTarget::Cursor);
        }

        #[test]
        fn accepts_fractional_window_sizing() {
            let config: Config = toml::from_str(
                "[window]\nwidth_fraction = 0.33\nvisible_rows = 6\nmin_width = 680\nmax_width = 920\nmin_height = 400\nmax_height = 640\n",
            )
            .expect("fractional window sizing should parse");
            assert_eq!(config.window.width_fraction, 0.33);
            assert_eq!(config.window.visible_rows, 6);
            assert_eq!(config.window.min_width, Some(680.0));
            assert_eq!(config.window.max_width, Some(920.0));
            assert_eq!(config.window.min_height, Some(400.0));
            assert_eq!(config.window.max_height, Some(640.0));
        }

        #[test]
        fn accepts_hide_when_inactive() {
            let config = validate_config_toml(
                Path::new("/tmp/runx-test-config.toml"),
                "[window]\nhide_when_inactive = false\n",
            )
            .expect("hide_when_inactive should parse");
            assert!(!config.window.hide_when_inactive);
        }

        #[test]
        fn resolves_window_size_with_clamps() {
            let config: Config = toml::from_str("[window]\nmin_width = 700\nmax_width = 980\n")
                .expect("window clamps should parse");
            assert_eq!(config.window.resolve_width(1600.0), 700.0);
            assert_eq!(config.window.resolve_width(2560.0), 980.0);
            assert_eq!(config.window.fallback_size(&config.ui).1, 570.0);
        }

        #[test]
        fn omitted_window_clamps_leave_size_unclamped() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert_eq!(config.window.resolve_width(1600.0), 640.0);
            assert_eq!(config.window.resolve_width(2560.0), 1024.0);
            assert_eq!(config.window.fallback_size(&config.ui), (768.0, 570.0));
        }

        #[test]
        fn row_driven_height_respects_max_height() {
            let config: Config = toml::from_str(
                "[window]\nvisible_rows = 10\nmax_height = 640\n[ui]\nscale = 1.25\n",
            )
            .expect("row-driven height config should parse");
            assert_eq!(config.window.resolve_width(2560.0), 1024.0);
            assert_eq!(config.window.fallback_size(&config.ui).1, 640.0);
        }

        #[test]
        fn accepts_display_overrides() {
            let config: Config = toml::from_str(
                "[[display_overrides]]\nbuilt_in = true\nvendor = 610\nmodel = 41171\nwidth_fraction = 0.46\nvisible_rows = 6\nui_scale = 1.08\n",
            )
            .expect("display override should parse");

            assert_eq!(config.display_overrides.len(), 1);
            let display_override = &config.display_overrides[0];
            assert_eq!(display_override.built_in, Some(true));
            assert_eq!(display_override.vendor, Some(610));
            assert_eq!(display_override.model, Some(41171));
            assert_eq!(display_override.width_fraction, Some(0.46));
            assert_eq!(display_override.visible_rows, Some(6));
            assert_eq!(display_override.ui_scale, Some(1.08));
        }

        #[test]
        fn resolves_best_matching_display_override() {
            let config: Config = toml::from_str(
                "[[display_overrides]]\nbuilt_in = false\nwidth_fraction = 0.52\nvisible_rows = 7\nui_scale = 1.04\n\n[[display_overrides]]\nserial = 4242\nwidth_fraction = 0.38\nvisible_rows = 5\nui_scale = 1.16\n",
            )
            .expect("display overrides should parse");
            let display = DisplayProfile {
                name: None,
                native_id: 1,
                built_in: false,
                vendor: Some(10),
                model: Some(20),
                serial: Some(4242),
                primary: false,
            };

            let (window, ui) = config.resolved_window_and_ui(Some(&display));

            assert_eq!(window.width_fraction, 0.38);
            assert_eq!(window.visible_rows, 5);
            assert_eq!(ui.scale, 1.16);
        }

        #[test]
        fn rejects_display_override_without_matcher() {
            let raw = "[[display_overrides]]\nwidth_fraction = 0.46\n";
            let error = validate_config_toml(Path::new("/tmp/runx-test-config.toml"), raw)
                .expect_err("display override without matcher should fail");

            assert!(error.to_string().contains(
                "[[display_overrides]] entry 1 must match at least one display attribute"
            ));
        }

        #[test]
        fn rejects_display_override_without_override_values() {
            let raw = "[[display_overrides]]\nbuilt_in = true\n";
            let error = validate_config_toml(Path::new("/tmp/runx-test-config.toml"), raw)
                .expect_err("display override without values should fail");

            assert!(
                error
                    .to_string()
                    .contains("[[display_overrides]] entry 1 must override at least one setting")
            );
        }

        #[test]
        fn rejects_display_override_vendor_without_model() {
            let raw = "[[display_overrides]]\nvendor = 610\nwidth_fraction = 0.46\n";
            let error = validate_config_toml(Path::new("/tmp/runx-test-config.toml"), raw)
                .expect_err("display override vendor without model should fail");

            assert!(
                error
                    .to_string()
                    .contains("[[display_overrides]] entry 1 must set vendor and model together")
            );
        }

        #[test]
        fn defaults_to_current_windows_provider_settings() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert!(!config.providers.windows.include_other_desktops);
            assert!(config.providers.windows.show_on_empty_query);
        }

        #[test]
        fn accepts_include_other_desktops() {
            let config: Config =
                toml::from_str("[providers.windows]\ninclude_other_desktops = true\n")
                    .expect("include_other_desktops should parse");
            assert!(config.providers.windows.include_other_desktops);
        }

        #[test]
        fn accepts_show_windows_on_empty_query() {
            let config: Config =
                toml::from_str("[providers.windows]\nshow_on_empty_query = false\n")
                    .expect("show_on_empty_query should parse");
            assert!(!config.providers.windows.show_on_empty_query);
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

    mod ranking_config_tests {
        use super::Config;
        use crate::config::{
            RankingScoreRule, RankingScoreRuleField, RankingScoreRuleMatchKind, validate_config,
        };

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
                panel: token!(),
                text: token!(),
                muted: token!(),
                canvas_bg: token!(),
                canvas_shadow: token!(),
                canvas_border: token!(),
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
            assert_eq!(config.ui.scale, 1.0);
            assert!(config.ui.canvas.show);
            assert_eq!(config.ui.canvas.radius, 24);
            assert_eq!(config.ui.canvas.background_opacity, 0.97);
            assert_eq!(config.ui.canvas.chrome_opacity, 1.0);
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
            assert!(config.ui.colorschemes.is_empty());
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
                "[ui]\nshow_header = false\ncycle_selection = true\ncolorscheme = \"builtin_dark\"\nscale = 1.2\n[ui.canvas]\nshow = false\nradius = 18\nbackground_opacity = 0.9\nchrome_opacity = 0.75\n[ui.entries]\nopacity = 0.64\n[ui.font_sizes]\ninput = 34\ntitle = 18\nsubtitle = 13\nbadge = 12\naccelerator = 13\nconfig_error_title = 28\nconfig_error_body = 17\nlabel = 11\n",
            )
            .expect("custom ui font sizes should parse");

            assert!(!config.ui.show_header);
            assert!(config.ui.cycle_selection);
            assert_eq!(config.ui.colorscheme, "builtin_dark");
            assert_eq!(config.ui.scale, 1.2);
            assert!(!config.ui.canvas.show);
            assert_eq!(config.ui.canvas.radius, 18);
            assert_eq!(config.ui.canvas.background_opacity, 0.9);
            assert_eq!(config.ui.canvas.chrome_opacity, 0.75);
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
        fn accepts_custom_ui_color_overrides() {
            let config: Config = toml::from_str(
                "[ui]\ncolorscheme = \"gruvbox\"\n[ui.colorschemes.gruvbox]\nbase = \"builtin_dark\"\npanel = \"#282828\"\ntext = \"#ebdbb2\"\n",
            )
            .expect("custom ui colorscheme should parse");

            assert_eq!(config.ui.colorscheme, "gruvbox");
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
        fn rejects_builtin_colorscheme_tables() {
            let mut config = Config::default();
            config.ui.colorschemes.insert(
                "builtin_dark".to_owned(),
                UiColorschemeConfig {
                    base: None,
                    overrides: UiColorOverridesConfig {
                        accent: Some("#fabd2f".to_owned()),
                        ..UiColorOverridesConfig::default()
                    },
                },
            );

            let error = validate_config(&config).expect_err("builtin override should fail");
            assert!(error.to_string().contains(
                "[ui.colorschemes.builtin_dark] is read-only; create a custom colorscheme with base = \"builtin_dark\""
            ));
        }

        #[test]
        fn rejects_legacy_color_override_fields() {
            for raw in [
                "[ui]\naccent = \"#333333\"\n",
                "[ui.colors]\naccent = \"#111111\"\n",
                "[ui.dark_colors]\nitem_bg = \"rgba(1,2,3,0.4)\"\n",
                "[ui.colorschemes.gruvbox]\nbase = \"builtin_dark\"\nbackground = \"#000000\"\n",
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
            assert!(error.to_string().contains("canvas_bg"));
        }
    }

    mod diagnostics_tests {
        use super::{Config, Path, render_toml_parse_error};
        use crate::config::{RawConfigSpans, validate_config_with_spans};

        #[test]
        fn toml_parse_errors_include_source_context() {
            let raw = "[[ranking.score_rules]]\nboost = \"oops\"\n";
            let error = toml::from_str::<Config>(raw).expect_err("config should fail to parse");
            let rendered =
                render_toml_parse_error(Path::new("/tmp/runx-test-config.toml"), raw, &error);

            assert!(rendered.contains("failed to parse /tmp/runx-test-config.toml"));
            assert!(rendered.contains("boost = \"oops\""));
            assert!(rendered.contains("expected i64"));
        }

        #[test]
        fn validation_errors_include_source_context() {
            let raw = "[ranking]\nprovider_order = [\"windows\", \"asdfasdf\"]\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-test-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("invalid configuration /tmp/runx-test-config.toml"));
            assert!(rendered.contains("provider_order = [\"windows\", \"asdfasdf\"]"));
            assert!(rendered.contains("unknown provider `asdfasdf`"));
        }

        #[test]
        fn legacy_canvas_opacity_still_configures_chrome_opacity() {
            let config: Config =
                toml::from_str("[ui.canvas]\nopacity = 0.42\n").expect("legacy key should parse");

            assert_eq!(config.ui.canvas.chrome_opacity, 0.42);
        }

        #[test]
        fn canvas_chrome_opacity_validation_errors_include_source_context() {
            let raw = "[ui.canvas]\nchrome_opacity = 1.2\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-test-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("invalid configuration /tmp/runx-test-config.toml"));
            assert!(rendered.contains("chrome_opacity = 1.2"));
            assert!(rendered.contains("[ui.canvas].chrome_opacity must be between 0.0 and 1.0"));
        }

        #[test]
        fn canvas_background_opacity_validation_errors_include_source_context() {
            let raw = "[ui.canvas]\nbackground_opacity = 1.2\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-test-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("invalid configuration /tmp/runx-test-config.toml"));
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
                validate_config_with_spans(Path::new("/tmp/runx-test-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("invalid configuration /tmp/runx-test-config.toml"));
            assert!(rendered.contains("opacity = -0.1"));
            assert!(rendered.contains("[ui.entries].opacity must be between 0.0 and 1.0"));
        }

        #[test]
        fn width_fraction_validation_errors_include_source_context() {
            let raw = "[window]\nwidth_fraction = 1.2\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-test-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("width_fraction = 1.2"));
            assert!(
                rendered
                    .contains("[window].width_fraction must be greater than 0.0 and at most 1.0",)
            );
        }

        #[test]
        fn visible_rows_validation_errors_include_source_context() {
            let raw = "[window]\nvisible_rows = 0\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-test-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("visible_rows = 0"));
            assert!(rendered.contains("[window].visible_rows must be greater than 0"));
        }

        #[test]
        fn ui_scale_validation_errors_include_source_context() {
            let raw = "[ui]\nscale = 0.0\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-test-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("scale = 0.0"));
            assert!(rendered.contains("[ui].scale must be greater than 0.0"));
        }

        #[test]
        fn display_override_validation_errors_include_source_context() {
            let raw = "[[display_overrides]]\nvendor = 610\nwidth_fraction = 0.46\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-test-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("[[display_overrides]]"));
            assert!(rendered.contains("vendor = 610"));
            assert!(
                rendered
                    .contains("[[display_overrides]] entry 1 must set vendor and model together")
            );
        }

        #[test]
        fn colorscheme_validation_errors_include_source_context() {
            let raw = "[ui]\ncolorscheme = \"gruvbox\"\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-test-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("invalid configuration /tmp/runx-test-config.toml"));
            assert!(rendered.contains("colorscheme = \"gruvbox\""));
            assert!(rendered.contains("[ui].colorscheme must be `system`"));
        }

        #[test]
        fn raw_spans_accept_full_ui_block_for_validation() {
            let raw = r##"
[ui]
show_header = true
cycle_selection = false
colorscheme = "gruvbox"
font_family = "\"SF Pro Display\", \"Avenir Next\", \"Helvetica Neue\", sans-serif"

[ui.colorschemes.gruvbox]
base = "builtin_light"
accent = "#c77b49"
canvas_bg = "#f3ede5"
panel = "#fffaf3"
text = "#1f1a16"
muted = "#756759"

[ui.canvas]
show = true
radius = 24
background_opacity = 0.97
chrome_opacity = 1.0

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
            validate_config_with_spans(Path::new("/tmp/runx-test-config.toml"), raw, &spans)
                .expect("validation should pass");
        }

        #[test]
        fn readonly_builtin_colorscheme_errors_include_source_context() {
            let raw = "[ui.colorschemes.builtin_dark]\naccent = \"#fabd2f\"\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-test-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("invalid configuration /tmp/runx-test-config.toml"));
            assert!(rendered.contains("[ui.colorschemes.builtin_dark]"));
            assert!(rendered.contains("is read-only"));
            assert!(rendered.contains("base = \"builtin_dark\""));
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
