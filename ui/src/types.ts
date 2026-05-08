export interface RenderItem {
  title: string;
  subtitle?: string;
  badge?: string;
  icon?: string;
  compact?: boolean;
  accelerator?: string;
}

export interface RenderPayload {
  query?: string;
  config_error?: string | null;
  items?: RenderItem[];
  layout_version?: number;
}

export interface Shortcut {
  key?: string | null;
  code?: string | null;
  alt?: boolean;
  ctrl?: boolean;
  meta?: boolean;
  shift?: boolean;
}

export type LauncherMessage =
  | { type: "ready" }
  | { type: "query_changed"; query: string }
  | { type: "activate"; index: number; all_windows: boolean }
  | { type: "copy_text"; text: string }
  | { type: "paste_text" }
  | { type: "hide" }
  | { type: "preferred_height"; height: number; layout_version: number };

export type SettingsMessage =
  | { type: "ready" }
  | { type: "open_url"; url: string }
  | { type: "reload" }
  | { type: "save"; draft: SettingsDraft }
  | { type: "save_raw"; raw: string }
  | { type: "close" }
  | { type: "update_plugins" };

export interface DisplayOptionPayload {
  key: string;
  label: string;
  built_in: boolean;
  vendor: number | null;
  model: number | null;
  serial: number | null;
}

export interface PluginInstallEntry {
  source: string;
  ref?: string;
  branch?: string;
}

export interface ScoreRule {
  providers: string[];
  field: string;
  match_kind: string;
  pattern: string;
  boost: number;
}

export interface ColorschemeEntry {
  name: string;
  base: string;
  tokens: Record<string, string>;
}

export interface SettingsDraft {
  debug_log: boolean;
  hotkey: { shortcut: string };
  window: {
    width_fraction: number;
    visible_rows: number;
    min_width: number | null;
    max_width: number | null;
    min_height: number | null;
    max_height: number | null;
    hide_when_inactive: boolean;
    always_on_top: boolean;
    show_animation: boolean;
    show_on: string;
    scale: number;
  };
  display_overrides: DisplayOverride[];
  providers: {
    disabled: string[];
    windows: {
      include_other_desktops: boolean;
      show_on_empty_query: boolean;
    };
    apps: {
      exact_name_boost: number;
      prefix_name_boost: number;
    };
  };
  ranking: {
    tie_threshold: number;
    provider_order: string[];
    provider_score_boosts: Record<string, number>;
    score_rules: ScoreRule[];
    result_limit: number;
  };
  timing: { search_debounce_ms: number };
  plugins: {
    directories: string[];
    search_paths: string[];
    install: PluginInstallEntry[];
    plugin_toml: string;
  };
  ui: {
    show_header: boolean;
    cycle_selection: boolean;
    colorscheme: string;
    font_family: string;
    canvas: {
      show: boolean;
      radius: number;
      background_opacity: number;
      chrome_opacity: number;
    };
    entries: { opacity: number };
    shortcuts: {
      focus_window: string;
      activate_all_windows: string;
    };
    colorschemes: ColorschemeEntry[];
    font_sizes: {
      label: number;
      input: number;
      title: number;
      subtitle: number;
      badge: number;
      accelerator: number;
      config_error_title: number;
      config_error_body: number;
    };
    layout: {
      section_gap: number;
      input_padding_y: number;
      input_padding_x: number;
      input_radius: number;
      list_gap: number;
      entry_padding_y: number;
      entry_padding_x: number;
      entry_gap: number;
      row_radius: number;
      badge_size: number;
      badge_radius: number;
      icon_size: number;
    };
  };
}

export interface DisplayOverride {
  built_in: boolean | null;
  vendor: number | null;
  model: number | null;
  serial: number | null;
  width_fraction?: number | null;
  visible_rows?: number | null;
  min_width?: number | null;
  max_width?: number | null;
  min_height?: number | null;
  max_height?: number | null;
  scale?: number | null;
}

export interface SettingsPayload {
  config_path: string;
  raw: string;
  error: string | null;
  draft: SettingsDraft | null;
  known_providers: string[];
  displays: DisplayOptionPayload[];
  builtin_colorschemes: string[];
  colorschemes: string[];
  color_tokens: string[];
  color_presets: Record<string, Record<string, string>>;
}

declare global {
  interface Window {
    ipc: { postMessage(message: string): void };
    __RUNX_RENDER?: (payload: RenderPayload) => void;
    __RUNX_FOCUS?: () => void;
    __RUNX_PASTE_TEXT?: (text?: string) => void;
    __RUNX_REQUEST_PREFERRED_HEIGHT?: () => void;
    __RUNX_CYCLE_SELECTION__?: boolean;
    __RUNX_FOCUS_WINDOW_SHORTCUT__?: Shortcut | null;
    __RUNX_ACTIVATE_ALL_WINDOWS_SHORTCUT__?: Shortcut | null;
    __RUNX_VISIBLE_ROWS__?: number | string;
    __RUNX_LAYOUT_VERSION__?: number | string;
    __RUNX_INITIAL_SETTINGS__?: SettingsPayload;
    __RUNX_SETTINGS_STATE__?: (payload: SettingsPayload) => void;
    __RUNX_SETTINGS_STATUS__?: (message: string, isError?: boolean) => void;
    RunxUi?: unknown;
  }
}
