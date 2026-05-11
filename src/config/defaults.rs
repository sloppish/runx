pub const DEFAULT_CONFIG: &str = r##"# Runx configuration
#
# `provider_order` controls which provider wins when scores are close.
# `tie_threshold` is the raw fuzzy-score delta that still counts as "similar".
# `show_on_empty_query` controls whether open windows are listed before you type anything.
# `provider_score_boosts` lets you nudge merged scores per provider.
# `score_rules` lets you boost or demote specific result text patterns.
# `search_debounce_ms` tunes search scheduling.

debug_log = false

[hotkey]
shortcut = "Option+Space"
quick_switch = "Option+Tab"

[window]
width_fraction = 0.4
visible_rows = 5
# min_width = 700
# max_width = 980
# min_height = 420
# max_height = 720
hide_when_inactive = true
always_on_top = true
show_on = "cursor"
scale = 1.0
show_animation = true

# Optional per-display size overrides.
# Matching prefers serial number, then vendor/model, then built-in/external.
# Example:
# [[display_overrides]]
# built_in = true
# vendor = 610
# model = 41171
# width_fraction = 0.46
# visible_rows = 6
# min_width = 720
# max_width = 960
# min_height = 420
# max_height = 720

[providers]
disabled = []

[providers.windows]
include_other_desktops = false
show_on_empty_query = true

[providers.apps]
exact_name_boost = 200
prefix_name_boost = 100

[ranking]
tie_threshold = 120
provider_order = ["windows", "apps", "settings"]
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
quick_switch_show_delay_ms = 90

[plugins]
directories = []
search_paths = []

# Per-plugin configuration can live under `[plugin.<id>]`.
# Command routing can be configured under `[plugin.<id>.commands]`.
[ui]
show_header = true
cycle_selection = false
colorscheme = "system"
font_family = "\"SF Pro Display\", \"Avenir Next\", \"Helvetica Neue\", sans-serif"
# Custom schemes are individual files in the colorschemes/ directory.
# Example: colorschemes/gruvbox.toml with base = "builtin_dark"

[ui.canvas]
show = true
radius = 24
background_opacity = 0.97
chrome_opacity = 1.0

[ui.entries]
opacity = 1.0
show_hover = true

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

pub const KNOWN_PROVIDER_NAMES: [&str; 4] = ["windows", "apps", "settings", "plugins"];
pub const BUILTIN_COLORSCHEME_NAMES: [&str; 2] = ["builtin_light", "builtin_dark"];
