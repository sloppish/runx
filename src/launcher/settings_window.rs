//! Webview-backed settings editor.
//!
//! The settings surface runs in a standalone process so it can behave like a
//! regular macOS app, separate from the accessory launcher process.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tao::{
    dpi::LogicalSize,
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use toml_edit::{
    Array, ArrayOfTables, DocumentMut as Document, Item, Table as TomlTable, Value, value,
};
use wry::WebViewBuilder;

use super::config_reload_ipc;

use crate::{
    config::{
        BUILTIN_COLORSCHEME_NAMES, Config, KNOWN_PROVIDER_NAMES, RankingScoreRuleField,
        RankingScoreRuleMatchKind, UI_COLOR_TOKEN_NAMES, UiColorOverridesConfig, UiShortcutConfig,
        WindowDisplayTarget, ensure_user_config, validate_config_toml,
    },
    displays::active_displays,
    types::{
        AppEvent, AppsProviderSettingsDraft, DisplayOverrideSettingsDraft, HotkeySettingsDraft,
        PluginInstallSettingsDraft, PluginsSettingsDraft, ProvidersSettingsDraft,
        RankingScoreRuleSettingsDraft, RankingSettingsDraft, SettingsCommand, SettingsDraft,
        TimingSettingsDraft, UiCanvasSettingsDraft, UiColorschemeSettingsDraft,
        UiEntriesSettingsDraft, UiFontSizesSettingsDraft, UiLayoutSettingsDraft, UiSettingsDraft,
        UiShortcutsSettingsDraft, WindowSettingsDraft, WindowsProviderSettingsDraft,
    },
    ui::builtin_colorscheme_token_values,
};

const HTML_TEMPLATE: &str = include_str!("../../ui/settings.html");
const STYLE_SOURCE: &str = include_str!("../../ui/settings.css");
const SCRIPT_SOURCE: &str = include_str!("../../ui/settings.js");
pub(crate) const SETTINGS_WINDOW_TITLE: &str = "Runx Settings";

/// Owns the native settings window and its webview for as long as it is open.
pub(crate) struct SettingsWindow {
    window: Window,
    webview: wry::WebView,
    ready: bool,
    pending_refresh: Option<String>,
    pending_status: Option<String>,
}

impl SettingsWindow {
    pub(crate) fn new(
        event_loop: &EventLoopWindowTarget<AppEvent>,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Result<Self> {
        let payload = settings_payload()?;
        let html = settings_html(&payload)?;
        let window = WindowBuilder::new()
            .with_title(SETTINGS_WINDOW_TITLE)
            .with_visible(false)
            .with_resizable(true)
            .with_inner_size(LogicalSize::new(1020.0, 922.0))
            .build(event_loop)
            .context("failed to build the settings window")?;

        let webview = WebViewBuilder::new()
            .with_html(&html)
            .with_ipc_handler(move |request| {
                let _ = proxy.send_event(settings_ipc_event(request.body()));
            })
            .build(&window)
            .context("failed to build the settings webview")?;

        Ok(Self {
            window,
            webview,
            ready: false,
            pending_refresh: None,
            pending_status: None,
        })
    }

    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn show(&self) -> Result<()> {
        self.window.set_visible(true);
        self.window.set_focus();
        self.webview
            .focus()
            .context("failed to focus the settings webview")
    }

    pub(crate) fn mark_ready(&mut self) -> Result<()> {
        self.ready = true;
        if let Some(script) = self.pending_refresh.take() {
            self.webview
                .evaluate_script(&script)
                .context("failed to flush pending settings refresh on Ready")?;
        }
        if let Some(script) = self.pending_status.take() {
            self.webview
                .evaluate_script(&script)
                .context("failed to flush pending settings status on Ready")?;
        }
        Ok(())
    }

    pub(crate) fn refresh(&mut self) -> Result<()> {
        let payload = settings_payload()?;
        let script = format!(
            "window.__RUNX_SETTINGS_STATE__ && window.__RUNX_SETTINGS_STATE__({});",
            safe_json(&payload)?
        );
        if self.ready {
            self.webview
                .evaluate_script(&script)
                .context("failed to refresh the settings window")?;
        } else {
            self.pending_refresh = Some(script);
        }
        Ok(())
    }

    pub(crate) fn set_status(&mut self, message: &str, is_error: bool) -> Result<()> {
        let script = format!(
            "window.__RUNX_SETTINGS_STATUS__ && window.__RUNX_SETTINGS_STATUS__({}, {});",
            serde_json::to_string(message)?,
            if is_error { "true" } else { "false" }
        );
        if self.ready {
            self.webview
                .evaluate_script(&script)
                .context("failed to update the settings status")?;
        } else {
            self.pending_status = Some(script);
        }
        Ok(())
    }
}

pub(crate) fn run_standalone_app() -> Result<()> {
    use objc2_foundation::{NSProcessInfo, NSString};
    let name = NSString::from_str(SETTINGS_WINDOW_TITLE);
    NSProcessInfo::processInfo().setProcessName(&name);

    let mut event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Regular);
    event_loop.set_dock_visibility(true);

    let proxy = event_loop.create_proxy();
    let mut settings = SettingsWindow::new(&event_loop, proxy)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        let result = match event {
            Event::NewEvents(StartCause::Init) => {
                override_app_menu_title(SETTINGS_WINDOW_TITLE);
                settings.show()
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } if window_id == settings.window_id() => {
                *control_flow = ControlFlow::Exit;
                Ok(())
            }
            Event::UserEvent(AppEvent::Settings(command)) => {
                handle_standalone_settings_command(&mut settings, command, control_flow)
            }
            _ => Ok(()),
        };

        if let Err(error) = result {
            let message = format!("{error:#}");
            eprintln!("Settings error: {message}");
            let _ = settings.set_status(&message, true);
        }
    });
}

fn override_app_menu_title(title: &str) {
    use objc2::{MainThreadMarker, MainThreadOnly, sel};
    use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let ns_title = NSString::from_str(title);

    let menu_item = |title_str: &str, action: objc2::runtime::Sel, key: &str| unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title_str),
            Some(action),
            &NSString::from_str(key),
        )
    };

    let menu_bar = NSMenu::new(mtm);

    let app_menu_item = NSMenuItem::new(mtm);
    let app_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns_title);
    app_menu.addItem(&menu_item(&format!("Hide {title}"), sel!(hide:), "h"));
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    app_menu.addItem(&menu_item(&format!("Quit {title}"), sel!(terminate:), "q"));
    app_menu_item.setSubmenu(Some(&app_menu));
    menu_bar.addItem(&app_menu_item);

    let edit_menu_item = NSMenuItem::new(mtm);
    let edit_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Edit"));
    edit_menu.addItem(&menu_item("Undo", sel!(undo:), "z"));
    edit_menu.addItem(&menu_item("Redo", sel!(redo:), "Z"));
    edit_menu.addItem(&NSMenuItem::separatorItem(mtm));
    edit_menu.addItem(&menu_item("Cut", sel!(cut:), "x"));
    edit_menu.addItem(&menu_item("Copy", sel!(copy:), "c"));
    edit_menu.addItem(&menu_item("Paste", sel!(paste:), "v"));
    edit_menu.addItem(&menu_item("Select All", sel!(selectAll:), "a"));
    edit_menu_item.setSubmenu(Some(&edit_menu));
    menu_bar.addItem(&edit_menu_item);

    let window_menu_item = NSMenuItem::new(mtm);
    let window_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Window"));
    window_menu.addItem(&menu_item("Close Window", sel!(performClose:), "w"));
    window_menu_item.setSubmenu(Some(&window_menu));
    menu_bar.addItem(&window_menu_item);

    app.setMainMenu(Some(&menu_bar));
}

fn handle_standalone_settings_command(
    settings: &mut SettingsWindow,
    command: SettingsCommand,
    control_flow: &mut ControlFlow,
) -> Result<()> {
    match command {
        SettingsCommand::Ready => {
            settings.mark_ready()?;
            settings.refresh()?;
        }
        SettingsCommand::Reload => {
            settings.refresh()?;
            notify_launcher_reload();
        }
        SettingsCommand::Save { draft } => {
            save_draft(draft.as_ref())?;
            let install_errors = run_plugin_install();
            notify_launcher_reload();
            settings.refresh()?;
            if install_errors.is_empty() {
                settings.set_status("", false)?;
            } else {
                settings.set_status(&install_errors.join("; "), true)?;
            }
        }
        SettingsCommand::SaveRaw { raw } => {
            save_raw(&raw)?;
            let install_errors = run_plugin_install();
            notify_launcher_reload();
            settings.refresh()?;
            if install_errors.is_empty() {
                settings.set_status("", false)?;
            } else {
                settings.set_status(&install_errors.join("; "), true)?;
            }
        }
        SettingsCommand::OpenUrl { url } => {
            let _ = std::process::Command::new("open").arg(&url).spawn();
        }
        SettingsCommand::UpdatePlugins => {
            handle_update_plugins(settings)?;
        }
        SettingsCommand::ClientError { message } => {
            settings.set_status(&message, true)?;
        }
        SettingsCommand::Close => {
            *control_flow = ControlFlow::Exit;
        }
    }
    Ok(())
}

fn run_plugin_install() -> Vec<String> {
    use crate::config::LoadedConfig;
    use crate::plugins::manager::ensure_installed;

    let Ok(loaded) = LoadedConfig::load() else {
        return Vec::new();
    };
    let Some(managed_dir) = loaded.plugin_dirs.first() else {
        return Vec::new();
    };
    ensure_installed(managed_dir, &loaded.config.plugins.install).errors
}

fn notify_launcher_reload() {
    let _ = config_reload_ipc::notify_reload();
}

fn handle_update_plugins(settings: &mut SettingsWindow) -> Result<()> {
    use crate::config::LoadedConfig;
    use crate::plugins::manager::{UpdateResult, update_all};

    settings.set_status("Updating plugins...", false)?;

    let loaded = LoadedConfig::load()?;
    if loaded.config.plugins.install.is_empty() {
        settings.set_status("No managed plugins configured.", false)?;
        return Ok(());
    }

    let plugins_dir = loaded
        .plugin_dirs
        .first()
        .context("no plugin directory available")?;

    let results = update_all(plugins_dir, &loaded.config.plugins.install);

    let mut updated = 0u32;
    let mut up_to_date = 0u32;
    let mut pinned = 0u32;
    let mut failed = Vec::new();

    for (id, result) in &results {
        match result {
            UpdateResult::Updated => updated += 1,
            UpdateResult::AlreadyUpToDate => up_to_date += 1,
            UpdateResult::Pinned => pinned += 1,
            UpdateResult::Failed(msg) => failed.push(format!("{id}: {msg}")),
        }
    }

    let status = if failed.is_empty() {
        let mut parts = Vec::new();
        if updated > 0 {
            parts.push(format!("{updated} updated"));
        }
        if up_to_date > 0 {
            parts.push(format!("{up_to_date} up to date"));
        }
        if pinned > 0 {
            parts.push(format!("{pinned} pinned"));
        }
        if parts.is_empty() {
            "No plugins to update.".to_owned()
        } else {
            parts.join(", ")
        }
    } else {
        format!("Errors: {}", failed.join("; "))
    };

    settings.set_status(&status, !failed.is_empty())?;
    notify_launcher_reload();

    Ok(())
}

#[derive(Debug, Serialize)]
struct SettingsPayload {
    config_path: String,
    raw: String,
    error: Option<String>,
    draft: Option<SettingsDraft>,
    known_providers: Vec<String>,
    displays: Vec<DisplayOptionPayload>,
    builtin_colorschemes: Vec<String>,
    colorschemes: Vec<String>,
    color_tokens: Vec<String>,
    color_presets: HashMap<String, HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct DisplayOptionPayload {
    key: String,
    label: String,
    built_in: bool,
    vendor: Option<u32>,
    model: Option<u32>,
    serial: Option<u32>,
}

pub(crate) fn save_draft(draft: &SettingsDraft) -> Result<()> {
    let path = ensure_user_config()?;
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let next = apply_settings_draft_to_raw(&path, &raw, draft)?;
    fs::write(&path, next).with_context(|| format!("failed to write {}", path.display()))
}

pub(crate) fn save_raw(raw: &str) -> Result<()> {
    let path = ensure_user_config()?;
    validate_config_toml(&path, raw)?;
    fs::write(&path, raw).with_context(|| format!("failed to write {}", path.display()))
}

fn settings_payload() -> Result<SettingsPayload> {
    let path = ensure_user_config()?;
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let (draft, error) = match validate_config_toml(&path, &raw) {
        Ok(config) => (
            Some(settings_draft_from_config(&config, &raw, &path)?),
            None,
        ),
        Err(error) => (None, Some(error.to_string())),
    };
    let custom_colorschemes = list_colorscheme_files(&path)?;

    let mut colorschemes = vec!["system".to_owned()];
    colorschemes.extend(
        BUILTIN_COLORSCHEME_NAMES
            .iter()
            .map(|name| (*name).to_owned()),
    );
    colorschemes.extend(custom_colorschemes);

    Ok(SettingsPayload {
        config_path: path.display().to_string(),
        raw,
        error,
        draft,
        known_providers: KNOWN_PROVIDER_NAMES
            .iter()
            .map(|provider| (*provider).to_owned())
            .collect(),
        displays: active_displays()
            .into_iter()
            .map(|display| {
                let mut label = display.label();
                let mut flags = Vec::new();
                if display.primary {
                    flags.push("primary");
                }
                if !flags.is_empty() {
                    label.push_str(&format!(" ({})", flags.join(", ")));
                }
                DisplayOptionPayload {
                    key: display_option_key(
                        display.built_in,
                        display.vendor,
                        display.model,
                        display.serial,
                    ),
                    label,
                    built_in: display.built_in,
                    vendor: display.vendor,
                    model: display.model,
                    serial: display.serial,
                }
            })
            .collect(),
        builtin_colorschemes: BUILTIN_COLORSCHEME_NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        colorschemes,
        color_tokens: UI_COLOR_TOKEN_NAMES
            .iter()
            .map(|token| (*token).to_owned())
            .collect(),
        color_presets: color_presets(),
    })
}

fn color_presets() -> HashMap<String, HashMap<String, String>> {
    BUILTIN_COLORSCHEME_NAMES
        .iter()
        .filter_map(|name| {
            builtin_colorscheme_token_values(name).map(|tokens| {
                (
                    (*name).to_owned(),
                    tokens
                        .into_iter()
                        .map(|(token, value)| (token.to_owned(), value))
                        .collect(),
                )
            })
        })
        .collect()
}

fn display_option_key(
    built_in: bool,
    vendor: Option<u32>,
    model: Option<u32>,
    serial: Option<u32>,
) -> String {
    format!(
        "{}:{}:{}:{}",
        built_in,
        vendor.map_or_else(String::new, |value| value.to_string()),
        model.map_or_else(String::new, |value| value.to_string()),
        serial.map_or_else(String::new, |value| value.to_string()),
    )
}

fn settings_html(payload: &SettingsPayload) -> Result<String> {
    let mut html = HTML_TEMPLATE.replace("__SETTINGS_CSS__", STYLE_SOURCE);
    html = html.replace("__SETTINGS_JS__", SCRIPT_SOURCE);
    html = html.replace("__INITIAL_SETTINGS__", &safe_json(payload)?);
    Ok(html)
}

fn safe_json(value: &impl Serialize) -> Result<String> {
    Ok(serde_json::to_string(value)?.replace('<', "\\u003c"))
}

fn settings_ipc_event(payload: &str) -> AppEvent {
    match serde_json::from_str::<SettingsCommand>(payload) {
        Ok(command) => AppEvent::Settings(command),
        Err(error) => AppEvent::Settings(SettingsCommand::ClientError {
            message: format!("Settings IPC error: {error}"),
        }),
    }
}

fn settings_draft_from_config(
    config: &Config,
    raw: &str,
    config_path: &Path,
) -> Result<SettingsDraft> {
    Ok(SettingsDraft {
        debug_log: config.debug_log,
        hotkey: HotkeySettingsDraft {
            shortcut: config.hotkey.shortcut.clone(),
            quick_switch: config.hotkey.quick_switch.clone(),
        },
        window: WindowSettingsDraft {
            width_fraction: config.window.width_fraction,
            visible_rows: config.window.visible_rows,
            min_width: config.window.min_width,
            max_width: config.window.max_width,
            min_height: config.window.min_height,
            max_height: config.window.max_height,
            hide_when_inactive: config.window.hide_when_inactive,
            always_on_top: config.window.always_on_top,
            show_on: match config.window.show_on {
                WindowDisplayTarget::Primary => "primary",
                WindowDisplayTarget::Cursor => "cursor",
            }
            .to_owned(),
            scale: config.window.scale,
            show_animation: config.window.show_animation,
        },
        display_overrides: config
            .display_overrides
            .iter()
            .map(|entry| DisplayOverrideSettingsDraft {
                built_in: entry.built_in,
                vendor: entry.vendor,
                model: entry.model,
                serial: entry.serial,
                width_fraction: entry.width_fraction,
                visible_rows: entry.visible_rows,
                min_width: entry.min_width,
                max_width: entry.max_width,
                min_height: entry.min_height,
                max_height: entry.max_height,
                scale: entry.scale,
            })
            .collect(),
        providers: ProvidersSettingsDraft {
            disabled: config.providers.disabled.clone(),
            windows: WindowsProviderSettingsDraft {
                include_other_desktops: config.providers.windows.include_other_desktops,
                show_on_empty_query: config.providers.windows.show_on_empty_query,
            },
            apps: AppsProviderSettingsDraft {
                exact_name_boost: config.providers.apps.exact_name_boost,
                prefix_name_boost: config.providers.apps.prefix_name_boost,
            },
        },
        ranking: RankingSettingsDraft {
            tie_threshold: config.ranking.tie_threshold,
            provider_order: config.ranking.provider_order.clone(),
            provider_score_boosts: config.ranking.provider_score_boosts.clone(),
            score_rules: config
                .ranking
                .score_rules
                .iter()
                .map(|rule| RankingScoreRuleSettingsDraft {
                    providers: rule.providers.clone(),
                    field: ranking_score_rule_field_name(rule.field).to_owned(),
                    match_kind: ranking_score_rule_match_name(rule.match_kind).to_owned(),
                    pattern: rule.pattern.clone(),
                    boost: rule.boost,
                })
                .collect(),
            result_limit: config.ranking.result_limit,
        },
        timing: TimingSettingsDraft {
            search_debounce_ms: config.timing.search_debounce_ms,
            quick_switch_show_delay_ms: config.timing.quick_switch_show_delay_ms,
        },
        plugins: PluginsSettingsDraft {
            directories: config.plugins.directories.clone(),
            search_paths: config.plugins.search_paths.clone(),
            install: config
                .plugins
                .install
                .iter()
                .map(|e| PluginInstallSettingsDraft {
                    source: e.source.clone(),
                    name: e.name.clone(),
                    git_ref: e.git_ref.clone(),
                    branch: e.branch.clone(),
                })
                .collect(),
            plugin_toml: plugin_toml_from_raw(raw)?,
        },
        ui: UiSettingsDraft {
            show_header: config.ui.show_header,
            cycle_selection: config.ui.cycle_selection,
            colorscheme: config.ui.colorscheme.clone(),
            font_family: config.ui.font_family.clone(),
            canvas: UiCanvasSettingsDraft {
                show: config.ui.canvas.show,
                radius: config.ui.canvas.radius,
                background_opacity: config.ui.canvas.background_opacity,
                chrome_opacity: config.ui.canvas.chrome_opacity,
            },
            entries: UiEntriesSettingsDraft {
                opacity: config.ui.entries.opacity,
                show_hover: config.ui.entries.show_hover,
                transition_ms: config.ui.entries.transition_ms,
            },
            shortcuts: UiShortcutsSettingsDraft {
                focus_window: shortcut_to_text(&config.ui.shortcuts.focus_window),
                activate_all_windows: shortcut_to_text(&config.ui.shortcuts.activate_all_windows),
            },
            colorschemes: load_colorscheme_drafts(config_path)?,
            font_sizes: UiFontSizesSettingsDraft {
                label: config.ui.font_sizes.label,
                input: config.ui.font_sizes.input,
                title: config.ui.font_sizes.title,
                subtitle: config.ui.font_sizes.subtitle,
                badge: config.ui.font_sizes.badge,
                accelerator: config.ui.font_sizes.accelerator,
                config_error_title: config.ui.font_sizes.config_error_title,
                config_error_body: config.ui.font_sizes.config_error_body,
            },
            layout: UiLayoutSettingsDraft {
                section_gap: config.ui.layout.section_gap,
                input_padding_y: config.ui.layout.input_padding_y,
                input_padding_x: config.ui.layout.input_padding_x,
                input_radius: config.ui.layout.input_radius,
                list_gap: config.ui.layout.list_gap,
                entry_padding_y: config.ui.layout.entry_padding_y,
                entry_padding_x: config.ui.layout.entry_padding_x,
                entry_gap: config.ui.layout.entry_gap,
                row_radius: config.ui.layout.row_radius,
                badge_size: config.ui.layout.badge_size,
                badge_radius: config.ui.layout.badge_radius,
                icon_size: config.ui.layout.icon_size,
            },
        },
    })
}

fn ranking_score_rule_field_name(field: RankingScoreRuleField) -> &'static str {
    match field {
        RankingScoreRuleField::Title => "title",
        RankingScoreRuleField::Subtitle => "subtitle",
        RankingScoreRuleField::Badge => "badge",
        RankingScoreRuleField::Id => "id",
    }
}

fn ranking_score_rule_match_name(match_kind: RankingScoreRuleMatchKind) -> &'static str {
    match match_kind {
        RankingScoreRuleMatchKind::Exact => "exact",
        RankingScoreRuleMatchKind::Prefix => "prefix",
        RankingScoreRuleMatchKind::Contains => "contains",
    }
}

fn shortcut_to_text(shortcut: &Option<UiShortcutConfig>) -> String {
    let Some(shortcut) = shortcut else {
        return "none".to_owned();
    };

    let mut parts = Vec::new();
    if shortcut.ctrl {
        parts.push("Ctrl".to_owned());
    }
    if shortcut.alt {
        parts.push("Option".to_owned());
    }
    if shortcut.shift {
        parts.push("Shift".to_owned());
    }
    if shortcut.meta {
        parts.push("Cmd".to_owned());
    }
    if let Some(key) = &shortcut.key {
        parts.push(key.clone());
    } else if let Some(code) = &shortcut.code {
        parts.push(code.clone());
    }

    if parts.is_empty() {
        "none".to_owned()
    } else {
        parts.join("+")
    }
}

fn colorscheme_tokens_from_overrides(
    overrides: &UiColorOverridesConfig,
) -> HashMap<String, String> {
    overrides.to_token_map()
}

fn plugin_toml_from_raw(raw: &str) -> Result<String> {
    let doc = raw
        .parse::<Document>()
        .context("failed to parse config.toml before reading plugin settings")?;
    let Some(plugin) = doc.as_table().get("plugin") else {
        return Ok(String::new());
    };
    let mut plugin_doc = Document::new();
    plugin_doc.as_table_mut().insert("plugin", plugin.clone());
    Ok(plugin_doc.to_string())
}

fn apply_settings_draft_to_raw(
    config_path: &Path,
    raw: &str,
    draft: &SettingsDraft,
) -> Result<String> {
    let mut doc = raw
        .parse::<Document>()
        .context("failed to parse config.toml before saving settings")?;
    let current_draft = validate_config_toml(config_path, raw)
        .ok()
        .and_then(|config| settings_draft_from_config(&config, raw, config_path).ok());

    set_item(&mut doc, &[], "debug_log", value(draft.debug_log))?;

    set_item(
        &mut doc,
        &["hotkey"],
        "shortcut",
        value(draft.hotkey.shortcut.clone()),
    )?;
    set_item(
        &mut doc,
        &["hotkey"],
        "quick_switch",
        value(draft.hotkey.quick_switch.clone()),
    )?;

    set_item(
        &mut doc,
        &["window"],
        "width_fraction",
        value(draft.window.width_fraction),
    )?;
    set_item(
        &mut doc,
        &["window"],
        "visible_rows",
        value(i64::try_from(draft.window.visible_rows).context("visible_rows is too large")?),
    )?;
    set_optional_f64_item(&mut doc, &["window"], "min_width", draft.window.min_width)?;
    set_optional_f64_item(&mut doc, &["window"], "max_width", draft.window.max_width)?;
    set_optional_f64_item(&mut doc, &["window"], "min_height", draft.window.min_height)?;
    set_optional_f64_item(&mut doc, &["window"], "max_height", draft.window.max_height)?;
    set_item(
        &mut doc,
        &["window"],
        "hide_when_inactive",
        value(draft.window.hide_when_inactive),
    )?;
    set_item(
        &mut doc,
        &["window"],
        "always_on_top",
        value(draft.window.always_on_top),
    )?;
    set_item(
        &mut doc,
        &["window"],
        "show_on",
        value(draft.window.show_on.clone()),
    )?;
    if current_draft
        .as_ref()
        .is_none_or(|current| current.display_overrides != draft.display_overrides)
    {
        set_display_overrides(&mut doc, &draft.display_overrides)?;
    }

    set_item(
        &mut doc,
        &["providers"],
        "disabled",
        string_array(&draft.providers.disabled),
    )?;
    set_item(
        &mut doc,
        &["providers", "windows"],
        "include_other_desktops",
        value(draft.providers.windows.include_other_desktops),
    )?;
    set_item(
        &mut doc,
        &["providers", "windows"],
        "show_on_empty_query",
        value(draft.providers.windows.show_on_empty_query),
    )?;
    set_item(
        &mut doc,
        &["providers", "apps"],
        "exact_name_boost",
        value(draft.providers.apps.exact_name_boost),
    )?;
    set_item(
        &mut doc,
        &["providers", "apps"],
        "prefix_name_boost",
        value(draft.providers.apps.prefix_name_boost),
    )?;

    set_item(
        &mut doc,
        &["ranking"],
        "tie_threshold",
        value(draft.ranking.tie_threshold),
    )?;
    set_item(
        &mut doc,
        &["ranking"],
        "provider_order",
        string_array(&draft.ranking.provider_order),
    )?;
    if current_draft.as_ref().is_none_or(|current| {
        current.ranking.provider_score_boosts != draft.ranking.provider_score_boosts
    }) {
        set_provider_score_boosts(&mut doc, &draft.ranking.provider_score_boosts)?;
    }
    if current_draft
        .as_ref()
        .is_none_or(|current| current.ranking.score_rules != draft.ranking.score_rules)
    {
        set_score_rules(&mut doc, &draft.ranking.score_rules)?;
    }
    set_item(
        &mut doc,
        &["ranking"],
        "result_limit",
        value(i64::try_from(draft.ranking.result_limit).context("result_limit is too large")?),
    )?;

    set_item(
        &mut doc,
        &["timing"],
        "search_debounce_ms",
        value(
            i64::try_from(draft.timing.search_debounce_ms)
                .context("search_debounce_ms is too large")?,
        ),
    )?;
    set_item(
        &mut doc,
        &["timing"],
        "quick_switch_show_delay_ms",
        value(
            i64::try_from(draft.timing.quick_switch_show_delay_ms)
                .context("quick_switch_show_delay_ms is too large")?,
        ),
    )?;
    set_plugin_paths(&mut doc, &draft.plugins)?;
    set_plugin_install(&mut doc, &draft.plugins.install)?;
    if current_draft
        .as_ref()
        .is_none_or(|current| current.plugins.plugin_toml != draft.plugins.plugin_toml)
    {
        set_plugin_toml(&mut doc, &draft.plugins.plugin_toml)?;
    }

    set_item(
        &mut doc,
        &["ui"],
        "show_header",
        value(draft.ui.show_header),
    )?;
    set_item(
        &mut doc,
        &["ui"],
        "cycle_selection",
        value(draft.ui.cycle_selection),
    )?;
    set_item(
        &mut doc,
        &["ui"],
        "colorscheme",
        value(draft.ui.colorscheme.clone()),
    )?;
    set_item(
        &mut doc,
        &["ui"],
        "font_family",
        value(draft.ui.font_family.clone()),
    )?;
    set_item(&mut doc, &["window"], "scale", value(draft.window.scale))?;
    set_item(
        &mut doc,
        &["window"],
        "show_animation",
        value(draft.window.show_animation),
    )?;
    set_item(
        &mut doc,
        &["ui", "canvas"],
        "show",
        value(draft.ui.canvas.show),
    )?;
    set_item(
        &mut doc,
        &["ui", "canvas"],
        "radius",
        value(i64::from(draft.ui.canvas.radius)),
    )?;
    set_item(
        &mut doc,
        &["ui", "canvas"],
        "background_opacity",
        value(draft.ui.canvas.background_opacity),
    )?;
    remove_item(&mut doc, &["ui", "canvas", "opacity"])?;
    set_item(
        &mut doc,
        &["ui", "canvas"],
        "chrome_opacity",
        value(draft.ui.canvas.chrome_opacity),
    )?;
    set_item(
        &mut doc,
        &["ui", "entries"],
        "opacity",
        value(draft.ui.entries.opacity),
    )?;
    set_item(
        &mut doc,
        &["ui", "entries"],
        "show_hover",
        value(draft.ui.entries.show_hover),
    )?;
    set_item(
        &mut doc,
        &["ui", "entries"],
        "transition_ms",
        value(i64::from(draft.ui.entries.transition_ms)),
    )?;

    set_ui_shortcuts(&mut doc, &draft.ui.shortcuts)?;
    save_colorscheme_files(config_path, &draft.ui.colorschemes)?;
    set_ui_font_sizes(&mut doc, &draft.ui.font_sizes)?;
    set_ui_layout(&mut doc, &draft.ui.layout)?;

    let next = doc.to_string();
    validate_config_toml(config_path, &next)?;
    Ok(next)
}

fn set_display_overrides(
    doc: &mut Document,
    overrides: &[DisplayOverrideSettingsDraft],
) -> Result<()> {
    if overrides.is_empty() {
        doc.as_table_mut().remove("display_overrides");
        return Ok(());
    }

    let mut tables = ArrayOfTables::new();
    for entry in overrides {
        let mut table = TomlTable::new();
        set_optional_bool(&mut table, "built_in", entry.built_in);
        set_optional_u32(&mut table, "vendor", entry.vendor);
        set_optional_u32(&mut table, "model", entry.model);
        set_optional_u32(&mut table, "serial", entry.serial);
        set_optional_f64(&mut table, "width_fraction", entry.width_fraction);
        set_optional_usize(&mut table, "visible_rows", entry.visible_rows)?;
        set_optional_f64(&mut table, "min_width", entry.min_width);
        set_optional_f64(&mut table, "max_width", entry.max_width);
        set_optional_f64(&mut table, "min_height", entry.min_height);
        set_optional_f64(&mut table, "max_height", entry.max_height);
        set_optional_f64(&mut table, "scale", entry.scale);
        tables.push(table);
    }
    doc.as_table_mut()
        .insert("display_overrides", Item::ArrayOfTables(tables));
    Ok(())
}

fn set_provider_score_boosts(doc: &mut Document, boosts: &HashMap<String, i64>) -> Result<()> {
    let ranking = table_mut(doc, &["ranking"])?;
    if boosts.is_empty() {
        ranking.remove("provider_score_boosts");
        return Ok(());
    }

    let mut table = TomlTable::new();
    let mut providers = boosts.keys().collect::<Vec<_>>();
    providers.sort();
    for provider in providers {
        table.insert(provider, value(boosts[provider]));
    }
    ranking.insert("provider_score_boosts", Item::Table(table));
    Ok(())
}

fn set_score_rules(doc: &mut Document, rules: &[RankingScoreRuleSettingsDraft]) -> Result<()> {
    let ranking = table_mut(doc, &["ranking"])?;
    if rules.is_empty() {
        ranking.remove("score_rules");
        return Ok(());
    }

    let mut tables = ArrayOfTables::new();
    for rule in rules {
        let mut table = TomlTable::new();
        table.insert("providers", string_array(&rule.providers));
        table.insert("field", value(rule.field.clone()));
        table.insert("match", value(rule.match_kind.clone()));
        table.insert("pattern", value(rule.pattern.clone()));
        table.insert("boost", value(rule.boost));
        tables.push(table);
    }
    ranking.insert("score_rules", Item::ArrayOfTables(tables));
    Ok(())
}

fn set_plugin_paths(doc: &mut Document, plugins: &PluginsSettingsDraft) -> Result<()> {
    set_item(
        doc,
        &["plugins"],
        "directories",
        string_array(&plugins.directories),
    )?;
    set_item(
        doc,
        &["plugins"],
        "search_paths",
        string_array(&plugins.search_paths),
    )?;
    Ok(())
}

fn set_plugin_install(doc: &mut Document, install: &[PluginInstallSettingsDraft]) -> Result<()> {
    let plugins = doc
        .as_table_mut()
        .entry("plugins")
        .or_insert_with(|| Item::Table(TomlTable::new()))
        .as_table_mut()
        .context("plugins is not a table")?;

    if install.is_empty() {
        plugins.remove("install");
        return Ok(());
    }

    let mut array_of_tables = ArrayOfTables::new();
    for entry in install {
        let mut table = TomlTable::new();
        table.insert("source", value(&entry.source));
        if let Some(name) = &entry.name {
            table.insert("name", value(name));
        }
        if let Some(git_ref) = &entry.git_ref {
            table.insert("ref", value(git_ref));
        }
        if let Some(branch) = &entry.branch {
            table.insert("branch", value(branch));
        }
        array_of_tables.push(table);
    }
    plugins.insert("install", Item::ArrayOfTables(array_of_tables));
    Ok(())
}

fn set_plugin_toml(doc: &mut Document, plugin_toml: &str) -> Result<()> {
    let plugin_toml = plugin_toml.trim();
    if plugin_toml.is_empty() {
        doc.as_table_mut().remove("plugin");
        return Ok(());
    }

    let parsed = plugin_toml
        .parse::<Document>()
        .context("failed to parse per-plugin TOML")?;
    for (key, _) in parsed.as_table().iter() {
        if key != "plugin" {
            bail!("per-plugin TOML must only contain [plugin.<id>] tables");
        }
    }
    let plugin = parsed
        .as_table()
        .get("plugin")
        .cloned()
        .context("per-plugin TOML must contain at least one [plugin.<id>] table")?;
    doc.as_table_mut().insert("plugin", plugin);
    Ok(())
}

fn set_ui_shortcuts(doc: &mut Document, shortcuts: &UiShortcutsSettingsDraft) -> Result<()> {
    set_item(
        doc,
        &["ui", "shortcuts"],
        "focus_window",
        value(shortcuts.focus_window.clone()),
    )?;
    set_item(
        doc,
        &["ui", "shortcuts"],
        "activate_all_windows",
        value(shortcuts.activate_all_windows.clone()),
    )?;
    Ok(())
}

fn colorschemes_dir_from_config_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("colorschemes")
}

fn list_colorscheme_files(config_path: &Path) -> Result<Vec<String>> {
    let dir = colorschemes_dir_from_config_path(config_path);
    let mut names = Vec::new();
    if dir.exists() {
        for entry in
            fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.is_file()
                && path.extension().and_then(|s| s.to_str()) == Some("toml")
                && let Some(name) = path.file_stem().and_then(|s| s.to_str())
            {
                names.push(name.to_owned());
            }
        }
    }
    names.sort();
    Ok(names)
}

fn load_colorscheme_drafts(config_path: &Path) -> Result<Vec<UiColorschemeSettingsDraft>> {
    let dir = colorschemes_dir_from_config_path(config_path);
    let mut drafts = Vec::new();
    if !dir.exists() {
        return Ok(drafts);
    }
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().is_file() && e.path().extension().and_then(|s| s.to_str()) == Some("toml")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let scheme: crate::config::UiColorschemeConfig =
            toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?;
        drafts.push(UiColorschemeSettingsDraft {
            name,
            base: scheme.base.unwrap_or_default(),
            tokens: colorscheme_tokens_from_overrides(&scheme.overrides),
        });
    }
    Ok(drafts)
}

fn save_colorscheme_files(
    config_path: &Path,
    colorschemes: &[UiColorschemeSettingsDraft],
) -> Result<()> {
    let dir = colorschemes_dir_from_config_path(config_path);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;

    let existing: std::collections::HashSet<String> = if dir.exists() {
        fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().is_file() && e.path().extension().and_then(|s| s.to_str()) == Some("toml")
            })
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_owned())
            })
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    let mut kept = std::collections::HashSet::new();
    for scheme in colorschemes {
        let name = scheme.name.trim();
        if name.is_empty() {
            bail!("custom colorscheme name must not be empty");
        }
        if name == "system" || BUILTIN_COLORSCHEME_NAMES.contains(&name) {
            bail!("custom colorscheme name must not be `system` or a built-in name");
        }

        let mut content = String::new();
        let base = scheme.base.trim();
        if !base.is_empty() {
            content.push_str(&format!("base = \"{base}\"\n"));
        }
        for token in UI_COLOR_TOKEN_NAMES {
            if let Some(value_text) = scheme.tokens.get(token).map(|v| v.trim())
                && !value_text.is_empty()
            {
                content.push_str(&format!("{token} = \"{value_text}\"\n"));
            }
        }

        let path = dir.join(format!("{name}.toml"));
        let current = fs::read_to_string(&path).unwrap_or_default();
        if current != content {
            fs::write(&path, &content)
                .with_context(|| format!("failed to write {}", path.display()))?;
        }
        kept.insert(name.to_owned());
    }

    for name in &existing {
        if !kept.contains(name) {
            let path = dir.join(format!("{name}.toml"));
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }

    Ok(())
}

fn set_ui_font_sizes(doc: &mut Document, font_sizes: &UiFontSizesSettingsDraft) -> Result<()> {
    macro_rules! set_u16 {
        ($field:ident) => {
            set_item(
                doc,
                &["ui", "font_sizes"],
                stringify!($field),
                value(i64::from(font_sizes.$field)),
            )?;
        };
    }
    set_u16!(label);
    set_u16!(input);
    set_u16!(title);
    set_u16!(subtitle);
    set_u16!(badge);
    set_u16!(accelerator);
    set_u16!(config_error_title);
    set_u16!(config_error_body);
    Ok(())
}

fn set_ui_layout(doc: &mut Document, layout: &UiLayoutSettingsDraft) -> Result<()> {
    macro_rules! set_u16 {
        ($field:ident) => {
            set_item(
                doc,
                &["ui", "layout"],
                stringify!($field),
                value(i64::from(layout.$field)),
            )?;
        };
    }
    set_u16!(section_gap);
    set_u16!(input_padding_y);
    set_u16!(input_padding_x);
    set_u16!(input_radius);
    set_u16!(list_gap);
    set_u16!(entry_padding_y);
    set_u16!(entry_padding_x);
    set_u16!(entry_gap);
    set_u16!(row_radius);
    set_u16!(badge_size);
    set_u16!(badge_radius);
    set_u16!(icon_size);
    Ok(())
}

fn set_optional_bool(table: &mut TomlTable, key: &str, item: Option<bool>) {
    if let Some(item) = item {
        table.insert(key, value(item));
    }
}

fn set_optional_u32(table: &mut TomlTable, key: &str, item: Option<u32>) {
    if let Some(item) = item {
        table.insert(key, value(i64::from(item)));
    }
}

fn set_optional_usize(table: &mut TomlTable, key: &str, item: Option<usize>) -> Result<()> {
    if let Some(item) = item {
        table.insert(
            key,
            value(i64::try_from(item).with_context(|| format!("{key} is too large"))?),
        );
    }
    Ok(())
}

fn set_optional_f64(table: &mut TomlTable, key: &str, item: Option<f64>) {
    if let Some(item) = item {
        table.insert(key, value(item));
    }
}

fn set_optional_f64_item(
    doc: &mut Document,
    path: &[&str],
    key: &str,
    item: Option<f64>,
) -> Result<()> {
    if let Some(item) = item {
        set_item(doc, path, key, value(item))
    } else {
        let mut full_path = Vec::with_capacity(path.len() + 1);
        full_path.extend_from_slice(path);
        full_path.push(key);
        remove_item(doc, &full_path)
    }
}

fn remove_item(doc: &mut Document, path: &[&str]) -> Result<()> {
    let Some((key, parent_path)) = path.split_last() else {
        bail!("cannot remove empty config path");
    };
    if let Some(table) = table_at_mut(doc, parent_path) {
        table.remove(key);
    }
    Ok(())
}

fn table_at_mut<'a>(doc: &'a mut Document, path: &[&str]) -> Option<&'a mut TomlTable> {
    let mut table = doc.as_table_mut();
    for segment in path {
        let item = table.get_mut(segment)?;
        table = item.as_table_mut()?;
    }
    Some(table)
}

fn set_item(doc: &mut Document, path: &[&str], key: &str, item: Item) -> Result<()> {
    table_mut(doc, path)?.insert(key, item);
    Ok(())
}

fn table_mut<'a>(doc: &'a mut Document, path: &[&str]) -> Result<&'a mut TomlTable> {
    let mut table = doc.as_table_mut();
    for segment in path {
        let item = table
            .entry(segment)
            .or_insert_with(|| Item::Table(TomlTable::new()));
        if !item.is_table() {
            *item = Item::Table(TomlTable::new());
        }
        table = item
            .as_table_mut()
            .with_context(|| format!("failed to create [{segment}] table"))?;
    }
    Ok(table)
}

fn string_array(values: &[String]) -> Item {
    let mut array = Array::new();
    for item in values {
        array.push(item.as_str());
    }
    Item::Value(Value::Array(array))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use super::{apply_settings_draft_to_raw, settings_draft_from_config, settings_ipc_event};
    use crate::config::{Config, validate_config_toml};
    use crate::types::{
        AppEvent, DisplayOverrideSettingsDraft, RankingScoreRuleSettingsDraft, SettingsCommand,
        UiColorschemeSettingsDraft,
    };

    #[test]
    fn structured_draft_loads_debug_log() {
        let raw = "debug_log = true\n";
        let config = validate_config_toml(Path::new("/tmp/runx-test-config.toml"), raw)
            .expect("debug_log config should validate");

        let draft = settings_draft_from_config(&config, raw, Path::new("/tmp/test.toml"))
            .expect("debug_log config should produce a settings draft");

        assert!(draft.debug_log);
    }

    #[test]
    fn structured_save_preserves_unrelated_comments_and_tables() {
        let raw = r#"# keep header

[plugin.sample]
enabled = true
"#;
        let mut draft =
            settings_draft_from_config(&Config::default(), raw, Path::new("/tmp/test.toml"))
                .expect("default config should produce a settings draft");
        draft.debug_log = true;
        draft.window.visible_rows = 8;
        draft.ui.canvas.show = false;
        draft.providers.disabled = vec!["settings".to_owned()];

        let saved =
            apply_settings_draft_to_raw(Path::new("/tmp/runx-test-config.toml"), raw, &draft)
                .expect("draft should save");

        assert!(saved.contains("# keep header"));
        assert!(saved.contains("[plugin.sample]"));
        assert!(saved.contains("debug_log = true"));
        assert!(saved.contains("visible_rows = 8"));
        assert!(saved.contains("show = false"));
        assert!(saved.contains("disabled = [\"settings\"]"));
    }

    #[test]
    fn structured_save_removes_empty_global_window_clamps() {
        let raw = r#"[window]
width_fraction = 0.4
visible_rows = 5
min_width = 1
max_width = 9999
min_height = 1
max_height = 9999
"#;
        let draft =
            settings_draft_from_config(&Config::default(), raw, Path::new("/tmp/test.toml"))
                .expect("default config should produce a settings draft");

        let saved =
            apply_settings_draft_to_raw(Path::new("/tmp/runx-test-config.toml"), raw, &draft)
                .expect("draft should save");

        assert!(!saved.contains("min_width"));
        assert!(!saved.contains("max_width"));
        assert!(!saved.contains("min_height"));
        assert!(!saved.contains("max_height"));
    }

    #[test]
    fn structured_save_covers_dynamic_config_sections() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let config_path = tmp.path().join("config.toml");
        let raw = r#"[hotkey]
shortcut = "Option+Space"
"#;
        std::fs::write(&config_path, raw).expect("write config");

        let mut draft = settings_draft_from_config(&Config::default(), raw, &config_path)
            .expect("default config should produce a settings draft");

        draft.display_overrides.push(DisplayOverrideSettingsDraft {
            built_in: Some(true),
            vendor: Some(610),
            model: Some(41171),
            serial: None,
            width_fraction: Some(0.46),
            visible_rows: Some(6),
            min_width: Some(720.0),
            max_width: Some(960.0),
            min_height: None,
            max_height: None,
            scale: Some(1.1),
        });
        draft
            .ranking
            .provider_score_boosts
            .insert("apps".to_owned(), 60);
        draft
            .ranking
            .score_rules
            .push(RankingScoreRuleSettingsDraft {
                providers: vec!["apps".to_owned(), "windows".to_owned()],
                field: "title".to_owned(),
                match_kind: "contains".to_owned(),
                pattern: "spotify".to_owned(),
                boost: 120,
            });
        draft.plugins.directories =
            vec!["~/Library/Application Support/runx/more-plugins".to_owned()];
        draft.plugins.search_paths = vec!["/opt/homebrew/bin".to_owned()];
        draft.plugins.plugin_toml = r#"[plugin.terminal]
terminal_app = "Alacritty"

[plugin.terminal.commands]
">" = "search_command"
"#
        .to_owned();
        draft.ui.shortcuts.focus_window = "Cmd+Enter".to_owned();
        draft.ui.shortcuts.activate_all_windows = "Cmd+Shift+Enter".to_owned();
        draft.ui.colorscheme = "gruvbox".to_owned();
        draft.ui.colorschemes.push(UiColorschemeSettingsDraft {
            name: "gruvbox".to_owned(),
            base: "builtin_dark".to_owned(),
            tokens: HashMap::from([
                ("accent".to_owned(), "#fabd2f".to_owned()),
                ("panel".to_owned(), "#282828".to_owned()),
            ]),
        });

        let saved = apply_settings_draft_to_raw(&config_path, raw, &draft)
            .expect("complete draft should save");

        assert!(saved.contains("[[display_overrides]]"));
        assert!(saved.contains("vendor = 610"));
        assert!(saved.contains("[ranking.provider_score_boosts]"));
        assert!(saved.contains("apps = 60"));
        assert!(saved.contains("[[ranking.score_rules]]"));
        assert!(saved.contains("pattern = \"spotify\""));
        assert!(saved.contains("search_paths = [\"/opt/homebrew/bin\"]"));
        assert!(saved.contains("[plugin.terminal.commands]"));
        assert!(saved.contains("[ui.shortcuts]"));
        assert!(saved.contains("focus_window = \"Cmd+Enter\""));
        assert!(!saved.contains("[ui.colorschemes.gruvbox]"));

        let scheme_path = tmp.path().join("colorschemes/gruvbox.toml");
        let scheme_content =
            std::fs::read_to_string(&scheme_path).expect("gruvbox.toml should be written");
        assert!(scheme_content.contains("base = \"builtin_dark\""));
        assert!(scheme_content.contains("accent = \"#fabd2f\""));
        assert!(scheme_content.contains("panel = \"#282828\""));
    }

    #[test]
    fn malformed_settings_ipc_returns_settings_error_event() {
        let event =
            settings_ipc_event(r#"{"type":"save","draft":{"ranking":{"result_limit":-1}}}"#);

        let AppEvent::Settings(SettingsCommand::ClientError { message }) = event else {
            panic!("malformed settings IPC should stay inside settings event path");
        };
        assert!(message.contains("Settings IPC error"));
    }
}
