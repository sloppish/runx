use std::{
    env, fs,
    io::{self, Stdout, Write},
    path::PathBuf,
    process::Command,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Row, Table as UiTable,
        TableState, Wrap,
    },
};
use runx::config::{
    BUILTIN_COLORSCHEME_NAMES, Config, DisplayOverrideConfig, KNOWN_PROVIDER_NAMES,
    UI_COLOR_TOKEN_NAMES, ensure_user_config, validate_config_toml,
};
use runx::displays::{DisplayProfile, active_displays, current_display};
use runx::ui::builtin_colorscheme_token_values;
use tempfile::Builder as TempFileBuilder;
use toml_edit::{
    Array, ArrayOfTables, Decor, Document as SpannedDocument, DocumentMut as Document, Item, Key,
    Table as TomlTable, Value, value,
};

const SECTIONS: [Section; 9] = [
    Section::Hotkey,
    Section::Window,
    Section::DisplayOverrides,
    Section::Providers,
    Section::Ranking,
    Section::UiBasics,
    Section::Colorschemes,
    Section::Validate,
    Section::Quit,
];

const PATH_HOTKEY_KEY: &[&str] = &["hotkey", "key"];
const PATH_HOTKEY_MODIFIERS: &[&str] = &["hotkey", "modifiers"];
const PATH_WINDOW_WIDTH_FRACTION: &[&str] = &["window", "width_fraction"];
const PATH_WINDOW_VISIBLE_ROWS: &[&str] = &["window", "visible_rows"];
const PATH_WINDOW_MIN_WIDTH: &[&str] = &["window", "min_width"];
const PATH_WINDOW_MAX_WIDTH: &[&str] = &["window", "max_width"];
const PATH_WINDOW_MIN_HEIGHT: &[&str] = &["window", "min_height"];
const PATH_WINDOW_MAX_HEIGHT: &[&str] = &["window", "max_height"];
const PATH_WINDOW_HIDE_ON_BLUR: &[&str] = &["window", "hide_on_blur"];
const PATH_WINDOW_ALWAYS_ON_TOP: &[&str] = &["window", "always_on_top"];
const PATH_WINDOW_SHOW_ON: &[&str] = &["window", "show_on"];
const PATH_PROVIDERS_DISABLED: &[&str] = &["providers", "disabled"];
const PATH_WINDOWS_INCLUDE_OTHER_DESKTOPS: &[&str] =
    &["providers", "windows", "include_other_desktops"];
const PATH_WINDOWS_SHOW_ON_EMPTY_QUERY: &[&str] = &["providers", "windows", "show_on_empty_query"];
const PATH_APPS_EXACT_NAME_BOOST: &[&str] = &["providers", "apps", "exact_name_boost"];
const PATH_APPS_PREFIX_NAME_BOOST: &[&str] = &["providers", "apps", "prefix_name_boost"];
const PATH_RANKING_TIE_THRESHOLD: &[&str] = &["ranking", "tie_threshold"];
const PATH_RANKING_RESULT_LIMIT: &[&str] = &["ranking", "result_limit"];
const PATH_RANKING_PROVIDER_ORDER: &[&str] = &["ranking", "provider_order"];
const PATH_UI_SHOW_HEADER: &[&str] = &["ui", "show_header"];
const PATH_UI_CYCLE_SELECTION: &[&str] = &["ui", "cycle_selection"];
const PATH_UI_FONT_FAMILY: &[&str] = &["ui", "font_family"];
const PATH_UI_SCALE: &[&str] = &["ui", "scale"];
const PATH_UI_COLORSCHEME: &[&str] = &["ui", "colorscheme"];
const PATH_UI_CANVAS_SHOW: &[&str] = &["ui", "canvas", "show"];
const PATH_UI_CANVAS_RADIUS: &[&str] = &["ui", "canvas", "radius"];
const PATH_UI_CANVAS_BACKGROUND_OPACITY: &[&str] = &["ui", "canvas", "background_opacity"];
const PATH_UI_CANVAS_CHROME_OPACITY: &[&str] = &["ui", "canvas", "chrome_opacity"];
const PATH_UI_CANVAS_LEGACY_OPACITY: &[&str] = &["ui", "canvas", "opacity"];
const PATH_UI_ENTRIES_OPACITY: &[&str] = &["ui", "entries", "opacity"];

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let path = ensure_user_config()?;
    let editor = ConfigEditor::load(path)?;
    let mut app = App::new(editor);
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable terminal raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
    Terminal::new(CrosstermBackend::new(stdout)).context("failed to initialize terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed to disable terminal raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")?;
    Ok(())
}

fn suspend_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed to disable terminal raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")?;
    Ok(())
}

fn resume_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    execute!(terminal.backend_mut(), EnterAlternateScreen)
        .context("failed to enter alternate screen")?;
    enable_raw_mode().context("failed to enable terminal raw mode")?;
    terminal.clear().context("failed to clear terminal")?;
    Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|frame| app.draw(frame))?;
        if app.should_quit {
            return Ok(());
        }

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        let action = match app.handle_key(key) {
            Ok(action) => action,
            Err(error) => {
                app.set_error(error);
                AppAction::None
            }
        };

        match action {
            AppAction::None => {}
            AppAction::Quit => app.should_quit = true,
            AppAction::ExternalEditor(request) => {
                if let Err(error) = suspend_terminal(terminal) {
                    app.set_error(error);
                    continue;
                }
                let edit_result = run_external_editor(app, request);
                let resume_result = resume_terminal(terminal);
                resume_result?;
                if let Err(error) = edit_result {
                    app.set_error(error);
                }
            }
        }
    }
}

struct ConfigEditor {
    path: PathBuf,
    raw: String,
    doc: Document,
    config: Config,
}

impl ConfigEditor {
    fn load(path: PathBuf) -> Result<Self> {
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config = validate_config_toml(&path, &raw)?;
        let doc = raw
            .parse::<Document>()
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(Self {
            path,
            raw,
            doc,
            config,
        })
    }

    fn reload(&mut self) -> Result<()> {
        let next = Self::load(self.path.clone())?;
        self.raw = next.raw;
        self.doc = next.doc;
        self.config = next.config;
        Ok(())
    }

    fn apply(&mut self, edit: impl FnOnce(&mut Document) -> Result<()>) -> Result<()> {
        let previous = self.doc.clone();
        if let Err(error) = edit(&mut self.doc) {
            self.doc = previous;
            return Err(error);
        }
        if let Err(error) = self.save() {
            self.doc = previous;
            return Err(error);
        }
        Ok(())
    }

    fn save(&mut self) -> Result<()> {
        let raw = self.doc.to_string();
        self.save_raw(raw)
    }

    fn save_raw(&mut self, raw: String) -> Result<()> {
        let config = validate_config_toml(&self.path, &raw)?;
        let doc = raw
            .parse::<Document>()
            .with_context(|| format!("failed to parse {}", self.path.display()))?;
        fs::write(&self.path, &raw)
            .with_context(|| format!("failed to write {}", self.path.display()))?;
        self.raw = raw;
        self.doc = doc;
        self.config = config;
        Ok(())
    }

    fn validate_current(&self) -> Result<()> {
        validate_config_toml(&self.path, &self.doc.to_string())?;
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Hotkey,
    Window,
    DisplayOverrides,
    Providers,
    Ranking,
    UiBasics,
    Colorschemes,
    Validate,
    Quit,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Self::Hotkey => "Hotkey",
            Self::Window => "Window",
            Self::DisplayOverrides => "Display overrides",
            Self::Providers => "Providers",
            Self::Ranking => "Ranking",
            Self::UiBasics => "UI basics",
            Self::Colorschemes => "Colorschemes",
            Self::Validate => "Validate config",
            Self::Quit => "Quit",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Sections,
    Fields,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    Info,
    Success,
    Error,
}

struct Status {
    text: String,
    kind: StatusKind,
}

impl Status {
    fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Info,
        }
    }

    fn success(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Success,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Error,
        }
    }
}

struct App {
    editor: ConfigEditor,
    selected_section: usize,
    selected_field: usize,
    focus: Focus,
    mode: Mode,
    status: Status,
    should_quit: bool,
}

impl App {
    fn new(editor: ConfigEditor) -> Self {
        Self {
            editor,
            selected_section: 0,
            selected_field: 0,
            focus: Focus::Sections,
            mode: Mode::Normal,
            status: Status::info("Loaded config"),
            should_quit: false,
        }
    }

    fn section(&self) -> Section {
        SECTIONS[self.selected_section]
    }

    fn fields(&self) -> Vec<Field> {
        fields_for(&self.editor, self.section())
    }

    fn draw(&self, frame: &mut Frame) {
        let root = frame.area();
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(4),
            ])
            .split(root);

        self.draw_header(frame, vertical[0]);
        self.draw_body(frame, vertical[1]);
        self.draw_footer(frame, vertical[2]);
        self.draw_mode(frame, root);
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let title = Line::from(vec![
            Span::styled("runx-config", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(
                self.editor.path.display().to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(title).block(Block::default().borders(Borders::ALL)),
            area,
        );
    }

    fn draw_body(&self, frame: &mut Frame, area: Rect) {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(26), Constraint::Min(30)])
            .split(area);
        self.draw_sections(frame, columns[0]);
        self.draw_fields(frame, columns[1]);
    }

    fn draw_sections(&self, frame: &mut Frame, area: Rect) {
        let items = SECTIONS
            .iter()
            .map(|section| ListItem::new(section.title()))
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        state.select(Some(self.selected_section));
        let highlight_style = if self.focus == Focus::Sections && matches!(self.mode, Mode::Normal)
        {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };
        let list = List::new(items)
            .block(Block::default().title("Sections").borders(Borders::ALL))
            .highlight_style(highlight_style)
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, area, &mut state);
    }

    fn draw_fields(&self, frame: &mut Frame, area: Rect) {
        let fields = self.fields();
        if fields.is_empty() {
            let text = match self.section() {
                Section::Validate => "Press Enter to validate the current config.",
                Section::Quit => "Press Enter to quit.",
                _ => "",
            };
            frame.render_widget(
                Paragraph::new(text).wrap(Wrap { trim: true }).block(
                    Block::default()
                        .title(self.section().title())
                        .borders(Borders::ALL),
                ),
                area,
            );
            return;
        }

        let rows = fields.iter().map(|field| {
            Row::new(vec![
                field.label.clone(),
                field.value.clone(),
                field.source.clone(),
            ])
        });
        let header = Row::new(vec!["Setting", "Value", "Source"]).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
        let mut state = TableState::default();
        state.select(Some(
            self.selected_field.min(fields.len().saturating_sub(1)),
        ));
        let highlight_style = if self.focus == Focus::Fields && matches!(self.mode, Mode::Normal) {
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };
        let table = UiTable::new(
            rows,
            [
                Constraint::Length(34),
                Constraint::Min(18),
                Constraint::Length(12),
            ],
        )
        .header(header)
        .block(
            Block::default()
                .title(self.section().title())
                .borders(Borders::ALL),
        )
        .row_highlight_style(highlight_style)
        .highlight_symbol("> ");
        frame.render_stateful_widget(table, area, &mut state);
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let status_style = match self.status.kind {
            StatusKind::Info => Style::default().fg(Color::Gray),
            StatusKind::Success => Style::default().fg(Color::Green),
            StatusKind::Error => Style::default().fg(Color::Red),
        };
        let mut status_line = vec![Span::styled(self.status.text.clone(), status_style)];
        if let Some(context) = self.selected_field_context() {
            status_line.push(Span::styled(
                format!("  {context}"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        let help = match self.mode {
            Mode::Normal => {
                "Tab focus  Up/Down move  Enter edit  d default  r reload  v validate  q quit"
            }
            Mode::Input(_) => "Enter save  Esc cancel  Ctrl-U clear  Backspace delete",
            Mode::Choice(_) => "Up/Down choose  Enter save  Esc cancel",
            Mode::Toggle(_) => "Space toggle  Enter save  Esc cancel",
            Mode::ProviderOrder(_) => {
                "Space include/exclude  u move up  d move down  Enter save  Esc cancel"
            }
            Mode::DisplayOverride(_) => {
                "Up/Down move  Enter edit  d unset  Esc back  unset values fall back to global settings"
            }
        };
        let text = vec![
            Line::from(status_line),
            Line::from(Span::styled(help, Style::default().fg(Color::DarkGray))),
        ];
        frame.render_widget(
            Paragraph::new(text).block(Block::default().borders(Borders::ALL)),
            area,
        );
    }

    fn draw_mode(&self, frame: &mut Frame, root: Rect) {
        match &self.mode {
            Mode::Normal => {}
            Mode::Input(input) => draw_input(frame, root, input),
            Mode::Choice(choice) => draw_choice(frame, root, choice),
            Mode::Toggle(toggle) => draw_toggle(frame, root, toggle),
            Mode::ProviderOrder(order) => draw_provider_order(frame, root, order),
            Mode::DisplayOverride(display_override) => {
                draw_display_override(frame, root, display_override, &self.editor.config)
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<AppAction> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.mode = Mode::Normal;
            return Ok(AppAction::Quit);
        }

        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Input(input) => self.handle_input_key(input, key),
            Mode::Choice(choice) => self.handle_choice_key(choice, key),
            Mode::Toggle(toggle) => self.handle_toggle_key(toggle, key),
            Mode::ProviderOrder(order) => self.handle_provider_order_key(order, key),
            Mode::DisplayOverride(display_override) => {
                self.handle_display_override_key(display_override, key)
            }
        }
    }

    fn selected_field_context(&self) -> Option<String> {
        if self.focus != Focus::Fields || !matches!(self.mode, Mode::Normal) {
            return None;
        }
        let fields = self.fields();
        let field = fields.get(self.selected_field)?;
        let path = field.id.path()?;
        Some(format!("key: {}  source: {}", path.join("."), field.source))
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> Result<AppAction> {
        match key.code {
            KeyCode::Char('q') => Ok(AppAction::Quit),
            KeyCode::Char('v') => {
                self.validate()?;
                Ok(AppAction::None)
            }
            KeyCode::Char('r') => {
                self.editor.reload()?;
                self.success("Reloaded config.toml");
                Ok(AppAction::None)
            }
            KeyCode::Char('d') => {
                self.reset_selected_field()?;
                Ok(AppAction::None)
            }
            KeyCode::Tab => {
                self.toggle_focus();
                Ok(AppAction::None)
            }
            KeyCode::Left => {
                self.focus = Focus::Sections;
                Ok(AppAction::None)
            }
            KeyCode::Right => {
                if !self.fields().is_empty() {
                    self.focus = Focus::Fields;
                }
                Ok(AppAction::None)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                Ok(AppAction::None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                Ok(AppAction::None)
            }
            KeyCode::Enter => self.activate(),
            _ => Ok(AppAction::None),
        }
    }

    fn handle_input_key(&mut self, mut input: InputMode, key: KeyEvent) -> Result<AppAction> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.info("Edit canceled");
                Ok(AppAction::None)
            }
            KeyCode::Enter => self.finish_input(input),
            KeyCode::Backspace => {
                input.value.pop();
                self.mode = Mode::Input(input);
                Ok(AppAction::None)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                input.value.clear();
                self.mode = Mode::Input(input);
                Ok(AppAction::None)
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                input.value.push(ch);
                self.mode = Mode::Input(input);
                Ok(AppAction::None)
            }
            _ => {
                self.mode = Mode::Input(input);
                Ok(AppAction::None)
            }
        }
    }

    fn handle_choice_key(&mut self, mut choice: ChoiceMode, key: KeyEvent) -> Result<AppAction> {
        match key.code {
            KeyCode::Esc => {
                self.info("Choice canceled");
                Ok(AppAction::None)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if choice.selected > 0 {
                    choice.selected -= 1;
                }
                self.mode = Mode::Choice(choice);
                Ok(AppAction::None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if choice.selected + 1 < choice.options.len() {
                    choice.selected += 1;
                }
                self.mode = Mode::Choice(choice);
                Ok(AppAction::None)
            }
            KeyCode::Enter => self.finish_choice(choice),
            _ => {
                self.mode = Mode::Choice(choice);
                Ok(AppAction::None)
            }
        }
    }

    fn handle_toggle_key(&mut self, mut toggle: ToggleMode, key: KeyEvent) -> Result<AppAction> {
        match key.code {
            KeyCode::Esc => {
                self.info("Edit canceled");
                Ok(AppAction::None)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if toggle.selected > 0 {
                    toggle.selected -= 1;
                }
                self.mode = Mode::Toggle(toggle);
                Ok(AppAction::None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if toggle.selected + 1 < toggle.options.len() {
                    toggle.selected += 1;
                }
                self.mode = Mode::Toggle(toggle);
                Ok(AppAction::None)
            }
            KeyCode::Char(' ') => {
                if let Some(option) = toggle.options.get_mut(toggle.selected) {
                    option.checked = !option.checked;
                }
                self.mode = Mode::Toggle(toggle);
                Ok(AppAction::None)
            }
            KeyCode::Enter => self.finish_toggle(toggle),
            _ => {
                self.mode = Mode::Toggle(toggle);
                Ok(AppAction::None)
            }
        }
    }

    fn handle_provider_order_key(
        &mut self,
        mut order: ProviderOrderMode,
        key: KeyEvent,
    ) -> Result<AppAction> {
        match key.code {
            KeyCode::Esc => {
                self.info("Provider order edit canceled");
                Ok(AppAction::None)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if order.selected > 0 {
                    order.selected -= 1;
                }
                self.mode = Mode::ProviderOrder(order);
                Ok(AppAction::None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if order.selected + 1 < order.items.len() {
                    order.selected += 1;
                }
                self.mode = Mode::ProviderOrder(order);
                Ok(AppAction::None)
            }
            KeyCode::Char(' ') => {
                if let Some(item) = order.items.get_mut(order.selected) {
                    item.enabled = !item.enabled;
                }
                self.mode = Mode::ProviderOrder(order);
                Ok(AppAction::None)
            }
            KeyCode::Char('u') => {
                if order.selected > 0 {
                    order.items.swap(order.selected, order.selected - 1);
                    order.selected -= 1;
                }
                self.mode = Mode::ProviderOrder(order);
                Ok(AppAction::None)
            }
            KeyCode::Char('d') => {
                if order.selected + 1 < order.items.len() {
                    order.items.swap(order.selected, order.selected + 1);
                    order.selected += 1;
                }
                self.mode = Mode::ProviderOrder(order);
                Ok(AppAction::None)
            }
            KeyCode::Enter => {
                let provider_order = order
                    .items
                    .into_iter()
                    .filter(|item| item.enabled)
                    .map(|item| item.name)
                    .collect::<Vec<_>>();
                self.apply(|doc| {
                    set_item(
                        doc,
                        &["ranking"],
                        "provider_order",
                        string_array(&provider_order),
                    )
                })?;
                self.success("Saved provider order");
                Ok(AppAction::None)
            }
            _ => {
                self.mode = Mode::ProviderOrder(order);
                Ok(AppAction::None)
            }
        }
    }

    fn handle_display_override_key(
        &mut self,
        mut display_override: DisplayOverrideMode,
        key: KeyEvent,
    ) -> Result<AppAction> {
        match key.code {
            KeyCode::Esc => {
                self.info("Display override edit canceled");
                Ok(AppAction::None)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if display_override.selected > 0 {
                    display_override.selected -= 1;
                }
                self.mode = Mode::DisplayOverride(display_override);
                Ok(AppAction::None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if display_override.selected + 1 < DisplayOverrideField::ALL.len() {
                    display_override.selected += 1;
                }
                self.mode = Mode::DisplayOverride(display_override);
                Ok(AppAction::None)
            }
            KeyCode::Char('d') => {
                let field = display_override.field();
                if !field.is_set(&display_override.override_config) {
                    self.info(format!("{} already uses the global setting", field.label()));
                    self.mode = Mode::DisplayOverride(display_override);
                    return Ok(AppAction::None);
                }

                let mut next_override = display_override.override_config.clone();
                field.clear(&mut next_override);
                let existing_index = self
                    .editor
                    .config
                    .display_override_index_for(Some(&display_override.display));
                self.apply(|doc| save_display_override(doc, existing_index, &next_override))?;
                self.mode = Mode::DisplayOverride(DisplayOverrideMode::with_selected(
                    &self.editor.config,
                    display_override.display.clone(),
                    display_override.selected,
                ));
                self.success(format!(
                    "Cleared {} override for {}",
                    field.label(),
                    display_override.display.label()
                ));
                Ok(AppAction::None)
            }
            KeyCode::Enter => {
                let field = display_override.field();
                self.mode = Mode::Input(InputMode::for_display_override(field, &display_override));
                Ok(AppAction::None)
            }
            _ => {
                self.mode = Mode::DisplayOverride(display_override);
                Ok(AppAction::None)
            }
        }
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Sections if self.fields().is_empty() => Focus::Sections,
            Focus::Sections => Focus::Fields,
            Focus::Fields => Focus::Sections,
        };
    }

    fn move_selection(&mut self, delta: isize) {
        match self.focus {
            Focus::Sections => {
                self.selected_section = move_index(self.selected_section, SECTIONS.len(), delta);
                self.selected_field = 0;
                if self.fields().is_empty() {
                    self.focus = Focus::Sections;
                }
            }
            Focus::Fields => {
                let field_count = self.fields().len();
                if field_count == 0 {
                    self.focus = Focus::Sections;
                } else {
                    self.selected_field = move_index(self.selected_field, field_count, delta);
                }
            }
        }
    }

    fn activate(&mut self) -> Result<AppAction> {
        if self.focus == Focus::Sections {
            match self.section() {
                Section::Validate => {
                    self.validate()?;
                    return Ok(AppAction::None);
                }
                Section::Quit => return Ok(AppAction::Quit),
                _ => {
                    if !self.fields().is_empty() {
                        self.focus = Focus::Fields;
                    }
                    return Ok(AppAction::None);
                }
            }
        }

        let fields = self.fields();
        let Some(field) = fields.get(self.selected_field) else {
            return Ok(AppAction::None);
        };
        self.activate_field(field.id)
    }

    fn activate_field(&mut self, id: FieldId) -> Result<AppAction> {
        match id {
            FieldId::HotkeyKey
            | FieldId::WindowWidthFraction
            | FieldId::WindowVisibleRows
            | FieldId::WindowMinWidth
            | FieldId::WindowMaxWidth
            | FieldId::WindowMinHeight
            | FieldId::WindowMaxHeight
            | FieldId::AppsExactNameBoost
            | FieldId::AppsPrefixNameBoost
            | FieldId::RankingTieThreshold
            | FieldId::RankingResultLimit
            | FieldId::UiFontFamily
            | FieldId::UiScale
            | FieldId::UiCanvasRadius
            | FieldId::UiCanvasBackgroundOpacity
            | FieldId::UiCanvasChromeOpacity
            | FieldId::UiEntriesOpacity => {
                self.mode = Mode::Input(InputMode::for_field(id, &self.editor.config));
                Ok(AppAction::None)
            }
            FieldId::WindowHideOnBlur => {
                let next = !self.editor.config.window.hide_on_blur;
                self.apply(|doc| set_item(doc, &["window"], "hide_on_blur", value(next)))?;
                self.success("Saved hide_on_blur");
                Ok(AppAction::None)
            }
            FieldId::WindowAlwaysOnTop => {
                let next = !self.editor.config.window.always_on_top;
                self.apply(|doc| set_item(doc, &["window"], "always_on_top", value(next)))?;
                self.success("Saved always_on_top");
                Ok(AppAction::None)
            }
            FieldId::WindowShowOn => {
                self.mode = Mode::Choice(ChoiceMode::show_on(&self.editor.config));
                Ok(AppAction::None)
            }
            FieldId::DisplayOverridesChoose => {
                let Some(choice) = ChoiceMode::display_override_displays(&self.editor.config)
                else {
                    self.info("No displays are currently available");
                    return Ok(AppAction::None);
                };
                self.mode = Mode::Choice(choice);
                Ok(AppAction::None)
            }
            FieldId::HotkeyModifiers => {
                self.mode = Mode::Toggle(ToggleMode::modifiers(&self.editor.config));
                Ok(AppAction::None)
            }
            FieldId::ProvidersDisabled => {
                self.mode = Mode::Toggle(ToggleMode::providers(
                    "Disabled providers",
                    ToggleTarget::DisabledProviders,
                    &self.editor.config.providers.disabled,
                ));
                Ok(AppAction::None)
            }
            FieldId::WindowsIncludeOtherDesktops => {
                let next = !self.editor.config.providers.windows.include_other_desktops;
                self.apply(|doc| {
                    set_item(
                        doc,
                        &["providers", "windows"],
                        "include_other_desktops",
                        value(next),
                    )
                })?;
                self.success("Saved include_other_desktops");
                Ok(AppAction::None)
            }
            FieldId::WindowsShowOnEmptyQuery => {
                let next = !self.editor.config.providers.windows.show_on_empty_query;
                self.apply(|doc| {
                    set_item(
                        doc,
                        &["providers", "windows"],
                        "show_on_empty_query",
                        value(next),
                    )
                })?;
                self.success("Saved show_on_empty_query");
                Ok(AppAction::None)
            }
            FieldId::RankingProviderOrder => {
                self.mode = Mode::ProviderOrder(ProviderOrderMode::new(&self.editor.config));
                Ok(AppAction::None)
            }
            FieldId::UiShowHeader => {
                let next = !self.editor.config.ui.show_header;
                self.apply(|doc| set_item(doc, &["ui"], "show_header", value(next)))?;
                self.success("Saved show_header");
                Ok(AppAction::None)
            }
            FieldId::UiCycleSelection => {
                let next = !self.editor.config.ui.cycle_selection;
                self.apply(|doc| set_item(doc, &["ui"], "cycle_selection", value(next)))?;
                self.success("Saved cycle_selection");
                Ok(AppAction::None)
            }
            FieldId::UiCanvasShow => {
                let next = !self.editor.config.ui.canvas.show;
                self.apply(|doc| set_item(doc, &["ui", "canvas"], "show", value(next)))?;
                self.success("Saved canvas.show");
                Ok(AppAction::None)
            }
            FieldId::UiColorscheme => {
                self.mode = Mode::Choice(ChoiceMode::colorschemes(&self.editor.config));
                Ok(AppAction::None)
            }
            FieldId::ColorschemeCreate => {
                self.mode = Mode::Input(InputMode::new_colorscheme());
                Ok(AppAction::None)
            }
            FieldId::ColorschemeEdit => {
                let Some(choice) = ChoiceMode::edit_colorscheme(&self.editor.config) else {
                    self.info("No custom colorschemes to edit");
                    return Ok(AppAction::None);
                };
                self.mode = Mode::Choice(choice);
                Ok(AppAction::None)
            }
            FieldId::ColorschemeDelete => {
                let Some(choice) = ChoiceMode::delete_colorscheme(&self.editor.config) else {
                    self.info("No custom colorschemes to delete");
                    return Ok(AppAction::None);
                };
                self.mode = Mode::Choice(choice);
                Ok(AppAction::None)
            }
        }
    }

    fn finish_input(&mut self, input: InputMode) -> Result<AppAction> {
        match input.target {
            InputTarget::Field(id) => {
                self.apply_input_field(id, input.value)?;
                Ok(AppAction::None)
            }
            InputTarget::DisplayOverrideField {
                display,
                field,
                selected,
            } => {
                let mut display_override = self
                    .editor
                    .config
                    .display_override_for(Some(&display))
                    .cloned()
                    .unwrap_or_else(|| DisplayOverrideConfig::for_display(&display));
                let existing_index = self
                    .editor
                    .config
                    .display_override_index_for(Some(&display));
                field.apply_input(&mut display_override, &input.value)?;
                self.apply(|doc| save_display_override(doc, existing_index, &display_override))?;
                self.mode = Mode::DisplayOverride(DisplayOverrideMode::with_selected(
                    &self.editor.config,
                    display.clone(),
                    selected,
                ));
                self.success(format!(
                    "Saved {} override for {}",
                    field.label(),
                    display.label()
                ));
                Ok(AppAction::None)
            }
            InputTarget::NewColorschemeName => {
                let name = input.value.trim().to_owned();
                validate_custom_colorscheme_name(&name)?;
                if self.editor.config.ui.colorschemes.contains_key(&name) {
                    bail!("colorscheme `{name}` already exists");
                }
                self.mode = Mode::Choice(ChoiceMode::colorscheme_base(name));
                Ok(AppAction::None)
            }
        }
    }

    fn apply_input_field(&mut self, id: FieldId, raw: String) -> Result<()> {
        match id {
            FieldId::HotkeyKey => {
                self.apply(|doc| set_item(doc, &["hotkey"], "key", value(raw)))?;
                self.success("Saved hotkey key");
            }
            FieldId::WindowWidthFraction => {
                let parsed = parse_f64(&raw, "window width fraction")?;
                self.apply(|doc| set_item(doc, &["window"], "width_fraction", value(parsed)))?;
                self.success("Saved window.width_fraction");
            }
            FieldId::WindowVisibleRows => {
                let parsed = parse_usize(&raw, "window visible rows")?;
                let parsed = i64::try_from(parsed).context("visible rows is too large")?;
                self.apply(|doc| set_item(doc, &["window"], "visible_rows", value(parsed)))?;
                self.success("Saved window.visible_rows");
            }
            FieldId::WindowMinWidth => {
                let parsed = parse_f64(&raw, "window min width")?;
                self.apply(|doc| set_item(doc, &["window"], "min_width", value(parsed)))?;
                self.success("Saved window.min_width");
            }
            FieldId::WindowMaxWidth => {
                let parsed = parse_f64(&raw, "window max width")?;
                self.apply(|doc| set_item(doc, &["window"], "max_width", value(parsed)))?;
                self.success("Saved window.max_width");
            }
            FieldId::WindowMinHeight => {
                let parsed = parse_f64(&raw, "window min height")?;
                self.apply(|doc| set_item(doc, &["window"], "min_height", value(parsed)))?;
                self.success("Saved window.min_height");
            }
            FieldId::WindowMaxHeight => {
                let parsed = parse_f64(&raw, "window max height")?;
                self.apply(|doc| set_item(doc, &["window"], "max_height", value(parsed)))?;
                self.success("Saved window.max_height");
            }
            FieldId::AppsExactNameBoost => {
                let parsed = parse_i64(&raw, "exact app name boost")?;
                self.apply(|doc| {
                    set_item(
                        doc,
                        &["providers", "apps"],
                        "exact_name_boost",
                        value(parsed),
                    )
                })?;
                self.success("Saved exact_name_boost");
            }
            FieldId::AppsPrefixNameBoost => {
                let parsed = parse_i64(&raw, "prefix app name boost")?;
                self.apply(|doc| {
                    set_item(
                        doc,
                        &["providers", "apps"],
                        "prefix_name_boost",
                        value(parsed),
                    )
                })?;
                self.success("Saved prefix_name_boost");
            }
            FieldId::RankingTieThreshold => {
                let parsed = parse_i64(&raw, "tie threshold")?;
                self.apply(|doc| set_item(doc, &["ranking"], "tie_threshold", value(parsed)))?;
                self.success("Saved tie_threshold");
            }
            FieldId::RankingResultLimit => {
                let parsed = parse_usize(&raw, "result limit")?;
                let parsed = i64::try_from(parsed).context("result limit is too large")?;
                self.apply(|doc| set_item(doc, &["ranking"], "result_limit", value(parsed)))?;
                self.success("Saved result_limit");
            }
            FieldId::UiFontFamily => {
                self.apply(|doc| set_item(doc, &["ui"], "font_family", value(raw)))?;
                self.success("Saved font_family");
            }
            FieldId::UiScale => {
                let parsed = parse_f64(&raw, "ui scale")?;
                self.apply(|doc| set_item(doc, &["ui"], "scale", value(parsed)))?;
                self.success("Saved ui.scale");
            }
            FieldId::UiCanvasRadius => {
                let parsed = parse_u16(&raw, "canvas radius")?;
                self.apply(|doc| {
                    set_item(doc, &["ui", "canvas"], "radius", value(i64::from(parsed)))
                })?;
                self.success("Saved canvas.radius");
            }
            FieldId::UiCanvasBackgroundOpacity => {
                let parsed = parse_f64(&raw, "canvas background opacity")?;
                self.apply(|doc| {
                    set_item(doc, &["ui", "canvas"], "background_opacity", value(parsed))
                })?;
                self.success("Saved canvas.background_opacity");
            }
            FieldId::UiCanvasChromeOpacity => {
                let parsed = parse_f64(&raw, "canvas chrome opacity")?;
                self.apply(|doc| {
                    remove_item(doc, PATH_UI_CANVAS_LEGACY_OPACITY)?;
                    set_item(doc, &["ui", "canvas"], "chrome_opacity", value(parsed))
                })?;
                self.success("Saved canvas.chrome_opacity");
            }
            FieldId::UiEntriesOpacity => {
                let parsed = parse_f64(&raw, "entries opacity")?;
                self.apply(|doc| set_item(doc, &["ui", "entries"], "opacity", value(parsed)))?;
                self.success("Saved entries.opacity");
            }
            _ => bail!("field is not editable as text"),
        }
        Ok(())
    }

    fn finish_choice(&mut self, choice: ChoiceMode) -> Result<AppAction> {
        let Some(option) = choice.options.get(choice.selected) else {
            return Ok(AppAction::None);
        };
        match choice.target {
            ChoiceTarget::WindowShowOn => {
                let selected = option.value.clone();
                self.apply(|doc| set_item(doc, &["window"], "show_on", value(selected)))?;
                self.success("Saved show_on");
                Ok(AppAction::None)
            }
            ChoiceTarget::UiColorscheme => {
                let selected = option.value.clone();
                self.apply(|doc| set_item(doc, &["ui"], "colorscheme", value(selected)))?;
                self.success("Saved colorscheme");
                Ok(AppAction::None)
            }
            ChoiceTarget::EditColorscheme => Ok(AppAction::ExternalEditor(
                ExternalEditorRequest::EditColorscheme {
                    name: option.value.clone(),
                },
            )),
            ChoiceTarget::DeleteColorscheme => {
                self.mode =
                    Mode::Choice(ChoiceMode::confirm_delete_colorscheme(option.value.clone()));
                Ok(AppAction::None)
            }
            ChoiceTarget::ConfirmDeleteColorscheme { name } => {
                if option.value == "delete" {
                    self.editor
                        .reload()
                        .context("failed to reload config.toml before deleting colorscheme")?;
                    if !self.editor.config.ui.colorschemes.contains_key(&name) {
                        bail!("colorscheme `{name}` no longer exists");
                    }

                    let reset_selection = self.editor.config.ui.colorscheme == name;
                    let raw = remove_colorscheme_snippet(&self.editor.raw, &name)?;
                    if reset_selection {
                        let mut doc = raw
                            .parse::<Document>()
                            .context("failed to parse config after deleting colorscheme")?;
                        set_item(&mut doc, &["ui"], "colorscheme", value("system"))?;
                        self.editor.save_raw(doc.to_string())?;
                    } else {
                        self.editor.save_raw(raw)?;
                    }
                    if reset_selection {
                        self.success(format!(
                            "Deleted colorscheme `{name}` and reset selection to `system`"
                        ));
                    } else {
                        self.success(format!("Deleted colorscheme `{name}`"));
                    }
                } else {
                    self.info("Delete canceled");
                }
                Ok(AppAction::None)
            }
            ChoiceTarget::SelectDisplayOverrideDisplay { displays } => {
                let index = option
                    .value
                    .parse::<usize>()
                    .context("invalid display selection")?;
                let display = displays
                    .get(index)
                    .cloned()
                    .context("selected display is no longer available")?;
                self.mode = Mode::Choice(ChoiceMode::display_override_actions(display));
                Ok(AppAction::None)
            }
            ChoiceTarget::DisplayOverrideActions { display } => {
                match option.value.as_str() {
                    "edit" => {
                        self.mode = Mode::DisplayOverride(DisplayOverrideMode::new(
                            &self.editor.config,
                            display,
                        ));
                    }
                    "delete" => {
                        let Some(index) = self
                            .editor
                            .config
                            .display_override_index_for(Some(&display))
                        else {
                            self.info(format!("No override configured for {}", display.label()));
                            return Ok(AppAction::None);
                        };
                        let label = self
                            .editor
                            .config
                            .display_overrides
                            .get(index)
                            .map(DisplayOverrideConfig::label)
                            .unwrap_or_else(|| display.label());
                        self.mode = Mode::Choice(ChoiceMode::confirm_delete_display_override(
                            display, index, label,
                        ));
                    }
                    other => bail!("unsupported display override action `{other}`"),
                }
                Ok(AppAction::None)
            }
            ChoiceTarget::ConfirmDeleteDisplayOverride {
                display,
                index,
                label,
            } => {
                if option.value == "delete" {
                    self.apply(|doc| remove_display_override(doc, index))?;
                    self.success(format!(
                        "Deleted override `{label}` for {}",
                        display.label()
                    ));
                } else {
                    self.info("Delete canceled");
                }
                Ok(AppAction::None)
            }
            ChoiceTarget::ColorschemeBase { name } => {
                let base = match option.value.as_str() {
                    "none" => None,
                    other => Some(other.to_owned()),
                };
                Ok(AppAction::ExternalEditor(
                    ExternalEditorRequest::CreateColorscheme { name, base },
                ))
            }
        }
    }

    fn finish_toggle(&mut self, toggle: ToggleMode) -> Result<AppAction> {
        let selected = toggle
            .options
            .into_iter()
            .filter(|option| option.checked)
            .map(|option| option.value)
            .collect::<Vec<_>>();
        match toggle.target {
            ToggleTarget::HotkeyModifiers => {
                self.apply(|doc| set_item(doc, &["hotkey"], "modifiers", string_array(&selected)))?;
                self.success("Saved hotkey modifiers");
            }
            ToggleTarget::DisabledProviders => {
                self.apply(|doc| {
                    set_item(doc, &["providers"], "disabled", string_array(&selected))
                })?;
                self.success("Saved disabled providers");
            }
        }
        Ok(AppAction::None)
    }

    fn reset_selected_field(&mut self) -> Result<()> {
        if self.focus != Focus::Fields {
            self.info("Select a field to reset");
            return Ok(());
        }
        let fields = self.fields();
        let Some(field) = fields.get(self.selected_field) else {
            self.info("No field selected");
            return Ok(());
        };
        let Some(path) = field.id.path() else {
            self.info("Selected row is an action, not a config key");
            return Ok(());
        };
        if !field.id.exists_in(&self.editor.doc) {
            self.info("Field already uses the default");
            return Ok(());
        }
        self.apply(|doc| {
            remove_item(doc, path)?;
            for legacy_path in field.id.legacy_paths() {
                remove_item(doc, legacy_path)?;
            }
            Ok(())
        })?;
        self.success(format!("Reset {} to default", field.label));
        Ok(())
    }

    fn validate(&mut self) -> Result<()> {
        self.editor.validate_current()?;
        self.success("Config is valid");
        Ok(())
    }

    fn apply(&mut self, edit: impl FnOnce(&mut Document) -> Result<()>) -> Result<()> {
        self.editor.apply(edit)
    }

    fn info(&mut self, text: impl Into<String>) {
        self.status = Status::info(text);
    }

    fn success(&mut self, text: impl Into<String>) {
        self.status = Status::success(text);
    }

    fn set_error(&mut self, error: anyhow::Error) {
        self.status = Status::error(error.to_string());
        self.mode = Mode::Normal;
    }
}

#[derive(Clone)]
enum Mode {
    Normal,
    Input(InputMode),
    Choice(ChoiceMode),
    Toggle(ToggleMode),
    ProviderOrder(ProviderOrderMode),
    DisplayOverride(DisplayOverrideMode),
}

#[derive(Clone)]
struct InputMode {
    title: String,
    value: String,
    target: InputTarget,
}

impl InputMode {
    fn for_field(id: FieldId, config: &Config) -> Self {
        Self {
            title: id.label().to_owned(),
            value: id.value(config),
            target: InputTarget::Field(id),
        }
    }

    fn new_colorscheme() -> Self {
        Self {
            title: "New colorscheme name".to_owned(),
            value: String::new(),
            target: InputTarget::NewColorschemeName,
        }
    }

    fn for_display_override(field: DisplayOverrideField, mode: &DisplayOverrideMode) -> Self {
        Self {
            title: format!("{} override", field.label()),
            value: field.input_value(&mode.override_config),
            target: InputTarget::DisplayOverrideField {
                display: mode.display.clone(),
                field,
                selected: mode.selected,
            },
        }
    }
}

#[derive(Clone)]
enum InputTarget {
    Field(FieldId),
    DisplayOverrideField {
        display: DisplayProfile,
        field: DisplayOverrideField,
        selected: usize,
    },
    NewColorschemeName,
}

#[derive(Clone)]
struct ChoiceMode {
    title: String,
    options: Vec<ChoiceOption>,
    selected: usize,
    target: ChoiceTarget,
}

impl ChoiceMode {
    fn show_on(config: &Config) -> Self {
        let options = ["cursor", "primary"]
            .into_iter()
            .map(ChoiceOption::from)
            .collect::<Vec<_>>();
        let current = show_on_summary(config);
        let selected = options
            .iter()
            .position(|option| option.value == current)
            .unwrap_or(0);
        Self {
            title: "Show on display".to_owned(),
            options,
            selected,
            target: ChoiceTarget::WindowShowOn,
        }
    }

    fn colorschemes(config: &Config) -> Self {
        let mut names = vec!["system".to_owned()];
        names.extend(
            BUILTIN_COLORSCHEME_NAMES
                .iter()
                .map(|name| (*name).to_owned()),
        );
        for name in config.ui.colorschemes.keys() {
            if !names.iter().any(|existing| existing == name) {
                names.push(name.clone());
            }
        }
        names.sort();
        let selected = names
            .iter()
            .position(|name| name == &config.ui.colorscheme)
            .unwrap_or(0);
        Self {
            title: "Selected colorscheme".to_owned(),
            options: names.into_iter().map(ChoiceOption::from).collect(),
            selected,
            target: ChoiceTarget::UiColorscheme,
        }
    }

    fn edit_colorscheme(config: &Config) -> Option<Self> {
        let mut names = config
            .ui
            .colorschemes
            .keys()
            .filter(|name| !BUILTIN_COLORSCHEME_NAMES.contains(&name.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        if names.is_empty() {
            return None;
        }
        Some(Self {
            title: "Edit colorscheme".to_owned(),
            options: names.into_iter().map(ChoiceOption::from).collect(),
            selected: 0,
            target: ChoiceTarget::EditColorscheme,
        })
    }

    fn delete_colorscheme(config: &Config) -> Option<Self> {
        let mut names = config
            .ui
            .colorschemes
            .keys()
            .filter(|name| !BUILTIN_COLORSCHEME_NAMES.contains(&name.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        if names.is_empty() {
            return None;
        }
        Some(Self {
            title: "Delete custom colorscheme".to_owned(),
            options: names.into_iter().map(ChoiceOption::from).collect(),
            selected: 0,
            target: ChoiceTarget::DeleteColorscheme,
        })
    }

    fn display_override_displays(config: &Config) -> Option<Self> {
        let displays = active_displays();
        if displays.is_empty() {
            return None;
        }

        let current_display_id = current_display().map(|display| display.native_id);
        Some(Self {
            title: "Choose display".to_owned(),
            options: displays
                .iter()
                .enumerate()
                .map(|(index, display)| ChoiceOption {
                    label: display_choice_label(display, config, current_display_id),
                    value: index.to_string(),
                })
                .collect(),
            selected: 0,
            target: ChoiceTarget::SelectDisplayOverrideDisplay { displays },
        })
    }

    fn confirm_delete_colorscheme(name: String) -> Self {
        Self {
            title: format!("Delete colorscheme `{name}`?"),
            options: vec![
                ChoiceOption {
                    label: "Cancel".to_owned(),
                    value: "cancel".to_owned(),
                },
                ChoiceOption {
                    label: format!("Delete `{name}`"),
                    value: "delete".to_owned(),
                },
            ],
            selected: 0,
            target: ChoiceTarget::ConfirmDeleteColorscheme { name },
        }
    }

    fn display_override_actions(display: DisplayProfile) -> Self {
        Self {
            title: format!("{} override", display.label()),
            options: vec![
                ChoiceOption {
                    label: "Edit override settings".to_owned(),
                    value: "edit".to_owned(),
                },
                ChoiceOption {
                    label: "Delete override".to_owned(),
                    value: "delete".to_owned(),
                },
            ],
            selected: 0,
            target: ChoiceTarget::DisplayOverrideActions { display },
        }
    }

    fn confirm_delete_display_override(
        display: DisplayProfile,
        index: usize,
        label: String,
    ) -> Self {
        Self {
            title: format!("Delete display override `{label}`?"),
            options: vec![
                ChoiceOption {
                    label: "Cancel".to_owned(),
                    value: "cancel".to_owned(),
                },
                ChoiceOption {
                    label: format!("Delete `{label}`"),
                    value: "delete".to_owned(),
                },
            ],
            selected: 0,
            target: ChoiceTarget::ConfirmDeleteDisplayOverride {
                display,
                index,
                label,
            },
        }
    }

    fn colorscheme_base(name: String) -> Self {
        let options = ["none", "builtin_dark", "builtin_light"]
            .into_iter()
            .map(ChoiceOption::from)
            .collect();
        Self {
            title: format!("Base for {name}"),
            options,
            selected: 1,
            target: ChoiceTarget::ColorschemeBase { name },
        }
    }
}

#[derive(Clone)]
struct ChoiceOption {
    label: String,
    value: String,
}

impl From<&str> for ChoiceOption {
    fn from(value: &str) -> Self {
        Self {
            label: value.to_owned(),
            value: value.to_owned(),
        }
    }
}

impl From<String> for ChoiceOption {
    fn from(value: String) -> Self {
        Self {
            label: value.clone(),
            value,
        }
    }
}

#[derive(Clone)]
enum ChoiceTarget {
    WindowShowOn,
    UiColorscheme,
    EditColorscheme,
    DeleteColorscheme,
    ConfirmDeleteColorscheme {
        name: String,
    },
    SelectDisplayOverrideDisplay {
        displays: Vec<DisplayProfile>,
    },
    DisplayOverrideActions {
        display: DisplayProfile,
    },
    ConfirmDeleteDisplayOverride {
        display: DisplayProfile,
        index: usize,
        label: String,
    },
    ColorschemeBase {
        name: String,
    },
}

#[derive(Clone)]
struct DisplayOverrideMode {
    display: DisplayProfile,
    override_config: DisplayOverrideConfig,
    selected: usize,
}

impl DisplayOverrideMode {
    fn new(config: &Config, display: DisplayProfile) -> Self {
        Self::with_selected(config, display, 0)
    }

    fn with_selected(config: &Config, display: DisplayProfile, selected: usize) -> Self {
        let override_config = config
            .display_override_for(Some(&display))
            .cloned()
            .unwrap_or_else(|| DisplayOverrideConfig::for_display(&display));
        Self {
            display,
            override_config,
            selected: selected.min(DisplayOverrideField::ALL.len().saturating_sub(1)),
        }
    }

    fn field(&self) -> DisplayOverrideField {
        DisplayOverrideField::ALL[self.selected]
    }
}

#[derive(Clone, Copy)]
enum DisplayOverrideField {
    WidthFraction,
    VisibleRows,
    MinWidth,
    MaxWidth,
    MinHeight,
    MaxHeight,
    UiScale,
}

impl DisplayOverrideField {
    const ALL: [Self; 7] = [
        Self::WidthFraction,
        Self::VisibleRows,
        Self::MinWidth,
        Self::MaxWidth,
        Self::MinHeight,
        Self::MaxHeight,
        Self::UiScale,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::WidthFraction => "Width fraction",
            Self::VisibleRows => "Visible rows",
            Self::MinWidth => "Min width",
            Self::MaxWidth => "Max width",
            Self::MinHeight => "Min height",
            Self::MaxHeight => "Max height",
            Self::UiScale => "UI scale",
        }
    }

    fn input_value(self, display_override: &DisplayOverrideConfig) -> String {
        match self {
            Self::WidthFraction => display_override
                .width_fraction
                .map(format_float)
                .unwrap_or_default(),
            Self::VisibleRows => display_override
                .visible_rows
                .map(|value| value.to_string())
                .unwrap_or_default(),
            Self::MinWidth => display_override
                .min_width
                .map(format_float)
                .unwrap_or_default(),
            Self::MaxWidth => display_override
                .max_width
                .map(format_float)
                .unwrap_or_default(),
            Self::MinHeight => display_override
                .min_height
                .map(format_float)
                .unwrap_or_default(),
            Self::MaxHeight => display_override
                .max_height
                .map(format_float)
                .unwrap_or_default(),
            Self::UiScale => display_override
                .ui_scale
                .map(format_float)
                .unwrap_or_default(),
        }
    }

    fn value(self, display_override: &DisplayOverrideConfig) -> String {
        let value = self.input_value(display_override);
        if value.is_empty() {
            "unset".to_owned()
        } else {
            value
        }
    }

    fn fallback(self, config: &Config) -> String {
        match self {
            Self::WidthFraction => format_float(config.window.width_fraction),
            Self::VisibleRows => config.window.visible_rows.to_string(),
            Self::MinWidth => format_float(config.window.min_width),
            Self::MaxWidth => format_float(config.window.max_width),
            Self::MinHeight => format_float(config.window.min_height),
            Self::MaxHeight => format_float(config.window.max_height),
            Self::UiScale => format_float(config.ui.scale),
        }
    }

    fn is_set(self, display_override: &DisplayOverrideConfig) -> bool {
        match self {
            Self::WidthFraction => display_override.width_fraction.is_some(),
            Self::VisibleRows => display_override.visible_rows.is_some(),
            Self::MinWidth => display_override.min_width.is_some(),
            Self::MaxWidth => display_override.max_width.is_some(),
            Self::MinHeight => display_override.min_height.is_some(),
            Self::MaxHeight => display_override.max_height.is_some(),
            Self::UiScale => display_override.ui_scale.is_some(),
        }
    }

    fn clear(self, display_override: &mut DisplayOverrideConfig) {
        match self {
            Self::WidthFraction => display_override.width_fraction = None,
            Self::VisibleRows => display_override.visible_rows = None,
            Self::MinWidth => display_override.min_width = None,
            Self::MaxWidth => display_override.max_width = None,
            Self::MinHeight => display_override.min_height = None,
            Self::MaxHeight => display_override.max_height = None,
            Self::UiScale => display_override.ui_scale = None,
        }
    }

    fn apply_input(self, display_override: &mut DisplayOverrideConfig, raw: &str) -> Result<()> {
        match self {
            Self::WidthFraction => {
                display_override.width_fraction =
                    Some(parse_f64(raw, "display override width fraction")?);
            }
            Self::VisibleRows => {
                display_override.visible_rows =
                    Some(parse_usize(raw, "display override visible rows")?);
            }
            Self::MinWidth => {
                display_override.min_width = Some(parse_f64(raw, "display override min width")?);
            }
            Self::MaxWidth => {
                display_override.max_width = Some(parse_f64(raw, "display override max width")?);
            }
            Self::MinHeight => {
                display_override.min_height = Some(parse_f64(raw, "display override min height")?);
            }
            Self::MaxHeight => {
                display_override.max_height = Some(parse_f64(raw, "display override max height")?);
            }
            Self::UiScale => {
                display_override.ui_scale = Some(parse_f64(raw, "display override ui scale")?);
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct ToggleMode {
    title: String,
    options: Vec<ToggleOption>,
    selected: usize,
    target: ToggleTarget,
}

impl ToggleMode {
    fn modifiers(config: &Config) -> Self {
        let values = ["Alt", "Control", "Shift", "Command"];
        Self {
            title: "Hotkey modifiers".to_owned(),
            options: values
                .into_iter()
                .map(|value| ToggleOption {
                    label: value.to_owned(),
                    value: value.to_owned(),
                    checked: config
                        .hotkey
                        .modifiers
                        .iter()
                        .any(|current| modifier_matches(current, value)),
                })
                .collect(),
            selected: 0,
            target: ToggleTarget::HotkeyModifiers,
        }
    }

    fn providers(title: &str, target: ToggleTarget, selected_values: &[String]) -> Self {
        Self {
            title: title.to_owned(),
            options: KNOWN_PROVIDER_NAMES
                .into_iter()
                .map(|value| ToggleOption {
                    label: value.to_owned(),
                    value: value.to_owned(),
                    checked: selected_values.iter().any(|selected| selected == value),
                })
                .collect(),
            selected: 0,
            target,
        }
    }
}

#[derive(Clone)]
struct ToggleOption {
    label: String,
    value: String,
    checked: bool,
}

#[derive(Clone, Copy)]
enum ToggleTarget {
    HotkeyModifiers,
    DisabledProviders,
}

#[derive(Clone)]
struct ProviderOrderMode {
    items: Vec<ProviderOrderItem>,
    selected: usize,
}

impl ProviderOrderMode {
    fn new(config: &Config) -> Self {
        let mut items = Vec::new();
        for provider in &config.ranking.provider_order {
            if KNOWN_PROVIDER_NAMES.contains(&provider.as_str())
                && !items
                    .iter()
                    .any(|item: &ProviderOrderItem| item.name == *provider)
            {
                items.push(ProviderOrderItem {
                    name: provider.clone(),
                    enabled: true,
                });
            }
        }
        for provider in KNOWN_PROVIDER_NAMES {
            if !items.iter().any(|item| item.name == provider) {
                items.push(ProviderOrderItem {
                    name: provider.to_owned(),
                    enabled: false,
                });
            }
        }
        Self { items, selected: 0 }
    }
}

#[derive(Clone)]
struct ProviderOrderItem {
    name: String,
    enabled: bool,
}

enum AppAction {
    None,
    Quit,
    ExternalEditor(ExternalEditorRequest),
}

enum ExternalEditorRequest {
    CreateColorscheme { name: String, base: Option<String> },
    EditColorscheme { name: String },
}

struct Field {
    id: FieldId,
    label: String,
    value: String,
    source: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldId {
    HotkeyKey,
    HotkeyModifiers,
    WindowWidthFraction,
    WindowVisibleRows,
    WindowMinWidth,
    WindowMaxWidth,
    WindowMinHeight,
    WindowMaxHeight,
    WindowHideOnBlur,
    WindowAlwaysOnTop,
    WindowShowOn,
    DisplayOverridesChoose,
    ProvidersDisabled,
    WindowsIncludeOtherDesktops,
    WindowsShowOnEmptyQuery,
    AppsExactNameBoost,
    AppsPrefixNameBoost,
    RankingTieThreshold,
    RankingResultLimit,
    RankingProviderOrder,
    UiShowHeader,
    UiCycleSelection,
    UiFontFamily,
    UiScale,
    UiColorscheme,
    UiCanvasShow,
    UiCanvasRadius,
    UiCanvasBackgroundOpacity,
    UiCanvasChromeOpacity,
    UiEntriesOpacity,
    ColorschemeCreate,
    ColorschemeEdit,
    ColorschemeDelete,
}

impl FieldId {
    fn label(self) -> &'static str {
        match self {
            Self::HotkeyKey => "Key",
            Self::HotkeyModifiers => "Modifiers",
            Self::WindowWidthFraction => "Width fraction",
            Self::WindowVisibleRows => "Visible rows",
            Self::WindowMinWidth => "Min width",
            Self::WindowMaxWidth => "Max width",
            Self::WindowMinHeight => "Min height",
            Self::WindowMaxHeight => "Max height",
            Self::WindowHideOnBlur => "Hide on blur",
            Self::WindowAlwaysOnTop => "Always on top",
            Self::WindowShowOn => "Show on display",
            Self::DisplayOverridesChoose => "Choose display",
            Self::ProvidersDisabled => "Disabled providers",
            Self::WindowsIncludeOtherDesktops => "Include windows from other desktops",
            Self::WindowsShowOnEmptyQuery => "Show windows on empty query",
            Self::AppsExactNameBoost => "Exact app name boost",
            Self::AppsPrefixNameBoost => "Prefix app name boost",
            Self::RankingTieThreshold => "Tie threshold",
            Self::RankingResultLimit => "Result limit",
            Self::RankingProviderOrder => "Provider order",
            Self::UiShowHeader => "Show header",
            Self::UiCycleSelection => "Cycle selection",
            Self::UiFontFamily => "Font family",
            Self::UiScale => "UI scale",
            Self::UiColorscheme => "Selected colorscheme",
            Self::UiCanvasShow => "Show canvas",
            Self::UiCanvasRadius => "Canvas radius",
            Self::UiCanvasBackgroundOpacity => "Canvas background opacity",
            Self::UiCanvasChromeOpacity => "Canvas chrome opacity",
            Self::UiEntriesOpacity => "Entry background opacity",
            Self::ColorschemeCreate => "Create custom colorscheme",
            Self::ColorschemeEdit => "Edit colorscheme in $EDITOR",
            Self::ColorschemeDelete => "Delete custom colorscheme",
        }
    }

    fn path(self) -> Option<&'static [&'static str]> {
        match self {
            Self::HotkeyKey => Some(PATH_HOTKEY_KEY),
            Self::HotkeyModifiers => Some(PATH_HOTKEY_MODIFIERS),
            Self::WindowWidthFraction => Some(PATH_WINDOW_WIDTH_FRACTION),
            Self::WindowVisibleRows => Some(PATH_WINDOW_VISIBLE_ROWS),
            Self::WindowMinWidth => Some(PATH_WINDOW_MIN_WIDTH),
            Self::WindowMaxWidth => Some(PATH_WINDOW_MAX_WIDTH),
            Self::WindowMinHeight => Some(PATH_WINDOW_MIN_HEIGHT),
            Self::WindowMaxHeight => Some(PATH_WINDOW_MAX_HEIGHT),
            Self::WindowHideOnBlur => Some(PATH_WINDOW_HIDE_ON_BLUR),
            Self::WindowAlwaysOnTop => Some(PATH_WINDOW_ALWAYS_ON_TOP),
            Self::WindowShowOn => Some(PATH_WINDOW_SHOW_ON),
            Self::DisplayOverridesChoose => None,
            Self::ProvidersDisabled => Some(PATH_PROVIDERS_DISABLED),
            Self::WindowsIncludeOtherDesktops => Some(PATH_WINDOWS_INCLUDE_OTHER_DESKTOPS),
            Self::WindowsShowOnEmptyQuery => Some(PATH_WINDOWS_SHOW_ON_EMPTY_QUERY),
            Self::AppsExactNameBoost => Some(PATH_APPS_EXACT_NAME_BOOST),
            Self::AppsPrefixNameBoost => Some(PATH_APPS_PREFIX_NAME_BOOST),
            Self::RankingTieThreshold => Some(PATH_RANKING_TIE_THRESHOLD),
            Self::RankingResultLimit => Some(PATH_RANKING_RESULT_LIMIT),
            Self::RankingProviderOrder => Some(PATH_RANKING_PROVIDER_ORDER),
            Self::UiShowHeader => Some(PATH_UI_SHOW_HEADER),
            Self::UiCycleSelection => Some(PATH_UI_CYCLE_SELECTION),
            Self::UiFontFamily => Some(PATH_UI_FONT_FAMILY),
            Self::UiScale => Some(PATH_UI_SCALE),
            Self::UiColorscheme => Some(PATH_UI_COLORSCHEME),
            Self::UiCanvasShow => Some(PATH_UI_CANVAS_SHOW),
            Self::UiCanvasRadius => Some(PATH_UI_CANVAS_RADIUS),
            Self::UiCanvasBackgroundOpacity => Some(PATH_UI_CANVAS_BACKGROUND_OPACITY),
            Self::UiCanvasChromeOpacity => Some(PATH_UI_CANVAS_CHROME_OPACITY),
            Self::UiEntriesOpacity => Some(PATH_UI_ENTRIES_OPACITY),
            Self::ColorschemeCreate | Self::ColorschemeEdit | Self::ColorschemeDelete => None,
        }
    }

    fn legacy_paths(self) -> &'static [&'static [&'static str]] {
        match self {
            Self::UiCanvasChromeOpacity => &[PATH_UI_CANVAS_LEGACY_OPACITY],
            _ => &[],
        }
    }

    fn exists_in(self, doc: &Document) -> bool {
        self.path().is_some_and(|path| item_exists(doc, path))
            || self
                .legacy_paths()
                .iter()
                .any(|path| item_exists(doc, path))
    }

    fn value(self, config: &Config) -> String {
        match self {
            Self::HotkeyKey => config.hotkey.key.clone(),
            Self::HotkeyModifiers => list_summary(&config.hotkey.modifiers),
            Self::WindowWidthFraction => format_float(config.window.width_fraction),
            Self::WindowVisibleRows => config.window.visible_rows.to_string(),
            Self::WindowMinWidth => format_float(config.window.min_width),
            Self::WindowMaxWidth => format_float(config.window.max_width),
            Self::WindowMinHeight => format_float(config.window.min_height),
            Self::WindowMaxHeight => format_float(config.window.max_height),
            Self::WindowHideOnBlur => bool_summary(config.window.hide_on_blur).to_owned(),
            Self::WindowAlwaysOnTop => bool_summary(config.window.always_on_top).to_owned(),
            Self::WindowShowOn => show_on_summary(config),
            Self::DisplayOverridesChoose => display_picker_summary(config),
            Self::ProvidersDisabled => list_summary(&config.providers.disabled),
            Self::WindowsIncludeOtherDesktops => {
                bool_summary(config.providers.windows.include_other_desktops).to_owned()
            }
            Self::WindowsShowOnEmptyQuery => {
                bool_summary(config.providers.windows.show_on_empty_query).to_owned()
            }
            Self::AppsExactNameBoost => config.providers.apps.exact_name_boost.to_string(),
            Self::AppsPrefixNameBoost => config.providers.apps.prefix_name_boost.to_string(),
            Self::RankingTieThreshold => config.ranking.tie_threshold.to_string(),
            Self::RankingResultLimit => config.ranking.result_limit.to_string(),
            Self::RankingProviderOrder => list_summary(&config.ranking.provider_order),
            Self::UiShowHeader => bool_summary(config.ui.show_header).to_owned(),
            Self::UiCycleSelection => bool_summary(config.ui.cycle_selection).to_owned(),
            Self::UiFontFamily => config.ui.font_family.clone(),
            Self::UiScale => format_float(config.ui.scale),
            Self::UiColorscheme => config.ui.colorscheme.clone(),
            Self::UiCanvasShow => bool_summary(config.ui.canvas.show).to_owned(),
            Self::UiCanvasRadius => config.ui.canvas.radius.to_string(),
            Self::UiCanvasBackgroundOpacity => format_float(config.ui.canvas.background_opacity),
            Self::UiCanvasChromeOpacity => format_float(config.ui.canvas.chrome_opacity),
            Self::UiEntriesOpacity => format_float(config.ui.entries.opacity),
            Self::ColorschemeCreate => "choose base, then open $EDITOR".to_owned(),
            Self::ColorschemeEdit => "opens $EDITOR".to_owned(),
            Self::ColorschemeDelete => "select custom scheme".to_owned(),
        }
    }
}

fn fields_for(editor: &ConfigEditor, section: Section) -> Vec<Field> {
    let ids: &[FieldId] = match section {
        Section::Hotkey => &[FieldId::HotkeyKey, FieldId::HotkeyModifiers],
        Section::Window => &[
            FieldId::WindowWidthFraction,
            FieldId::WindowVisibleRows,
            FieldId::WindowMinWidth,
            FieldId::WindowMaxWidth,
            FieldId::WindowMinHeight,
            FieldId::WindowMaxHeight,
            FieldId::WindowHideOnBlur,
            FieldId::WindowAlwaysOnTop,
            FieldId::WindowShowOn,
        ],
        Section::DisplayOverrides => &[FieldId::DisplayOverridesChoose],
        Section::Providers => &[
            FieldId::ProvidersDisabled,
            FieldId::WindowsIncludeOtherDesktops,
            FieldId::WindowsShowOnEmptyQuery,
            FieldId::AppsExactNameBoost,
            FieldId::AppsPrefixNameBoost,
        ],
        Section::Ranking => &[
            FieldId::RankingTieThreshold,
            FieldId::RankingResultLimit,
            FieldId::RankingProviderOrder,
        ],
        Section::UiBasics => &[
            FieldId::UiShowHeader,
            FieldId::UiCycleSelection,
            FieldId::UiFontFamily,
            FieldId::UiScale,
            FieldId::UiCanvasShow,
            FieldId::UiCanvasRadius,
            FieldId::UiCanvasBackgroundOpacity,
            FieldId::UiCanvasChromeOpacity,
            FieldId::UiEntriesOpacity,
        ],
        Section::Colorschemes => &[
            FieldId::UiColorscheme,
            FieldId::ColorschemeCreate,
            FieldId::ColorschemeEdit,
            FieldId::ColorschemeDelete,
        ],
        Section::Validate | Section::Quit => &[],
    };

    ids.iter()
        .map(|id| {
            let source = if id.path().is_some() {
                if id.exists_in(&editor.doc) {
                    "config.toml".to_owned()
                } else {
                    "default".to_owned()
                }
            } else {
                "action".to_owned()
            };
            Field {
                id: *id,
                label: id.label().to_owned(),
                value: id.value(&editor.config),
                source,
            }
        })
        .collect()
}

fn draw_input(frame: &mut Frame, root: Rect, input: &InputMode) {
    let area = centered_rect(70, 30, root);
    frame.render_widget(Clear, area);
    let text = vec![
        Line::from(input.value.as_str()),
        Line::from(""),
        Line::from(Span::styled(
            "Enter saves, Esc cancels",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(input.title.as_str())
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn draw_choice(frame: &mut Frame, root: Rect, choice: &ChoiceMode) {
    let area = centered_rect(60, 55, root);
    frame.render_widget(Clear, area);
    let items = choice
        .options
        .iter()
        .map(|option| ListItem::new(option.label.clone()))
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    state.select(Some(choice.selected));
    let list = List::new(items)
        .block(
            Block::default()
                .title(choice.title.as_str())
                .borders(Borders::ALL),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_toggle(frame: &mut Frame, root: Rect, toggle: &ToggleMode) {
    let area = centered_rect(60, 55, root);
    frame.render_widget(Clear, area);
    let items = toggle
        .options
        .iter()
        .map(|option| {
            ListItem::new(format!(
                "{} {}",
                if option.checked { "[x]" } else { "[ ]" },
                option.label
            ))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    state.select(Some(toggle.selected));
    let list = List::new(items)
        .block(
            Block::default()
                .title(toggle.title.as_str())
                .borders(Borders::ALL),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_provider_order(frame: &mut Frame, root: Rect, order: &ProviderOrderMode) {
    let area = centered_rect(65, 65, root);
    frame.render_widget(Clear, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)])
        .split(area);
    let items = order
        .items
        .iter()
        .map(|item| {
            ListItem::new(format!(
                "{} {}",
                if item.enabled { "[x]" } else { "[ ]" },
                item.name
            ))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    state.select(Some(order.selected));
    let list = List::new(items)
        .block(
            Block::default()
                .title("Provider order")
                .borders(Borders::ALL),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[0], &mut state);
    frame.render_widget(
        Paragraph::new("Space toggles inclusion. u/d moves the selected provider. Enter saves.")
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL)),
        chunks[1],
    );
}

fn draw_display_override(
    frame: &mut Frame,
    root: Rect,
    display_override: &DisplayOverrideMode,
    config: &Config,
) {
    let area = centered_rect(78, 72, root);
    frame.render_widget(Clear, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(3)])
        .split(area);

    let rows = DisplayOverrideField::ALL.iter().map(|field| {
        Row::new(vec![
            field.label().to_owned(),
            field.value(&display_override.override_config),
            field.fallback(config),
        ])
    });
    let header = Row::new(vec!["Setting", "Override", "Global fallback"]).style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = TableState::default();
    state.select(Some(display_override.selected));
    let table = UiTable::new(
        rows,
        [
            Constraint::Length(26),
            Constraint::Length(18),
            Constraint::Min(18),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(format!(
                "Override settings - {}",
                display_override.display.label()
            ))
            .borders(Borders::ALL),
    )
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("> ");
    frame.render_stateful_widget(table, chunks[0], &mut state);
    frame.render_widget(
        Paragraph::new("Unset values inherit the current global window or UI scale setting.")
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL)),
        chunks[1],
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn run_external_editor(app: &mut App, request: ExternalEditorRequest) -> Result<()> {
    match request {
        ExternalEditorRequest::CreateColorscheme { name, base } => {
            app.editor
                .reload()
                .context("failed to reload config.toml before creating colorscheme")?;
            if app.editor.config.ui.colorschemes.contains_key(&name) {
                bail!("colorscheme `{name}` already exists");
            }

            let initial = new_colorscheme_snippet(&name, base.as_deref());
            let edited = edit_in_external_editor(&initial)?;
            let table = colorscheme_table_from_snippet(&edited, &name)?;
            app.editor
                .reload()
                .context("failed to reload config.toml before saving colorscheme")?;
            app.apply(|doc| {
                set_colorscheme_table(doc, &name, table)?;
                set_item(doc, &["ui"], "colorscheme", value(name.clone()))?;
                Ok(())
            })?;
            app.success(format!("Created and selected colorscheme `{name}`"));
        }
        ExternalEditorRequest::EditColorscheme { name } => {
            app.editor
                .reload()
                .context("failed to reload config.toml before editing colorscheme")?;
            if !app.editor.config.ui.colorschemes.contains_key(&name) {
                bail!("colorscheme `{name}` no longer exists");
            }

            let initial = colorscheme_snippet(&app.editor.raw, &name)?;
            let edited = edit_in_external_editor(&initial)?;
            colorscheme_table_from_snippet(&edited, &name)?;
            app.editor
                .reload()
                .context("failed to reload config.toml before saving colorscheme")?;
            let raw = replace_colorscheme_snippet(&app.editor.raw, &name, &edited)?;
            app.editor.save_raw(raw)?;
            app.success(format!("Saved colorscheme `{name}`"));
        }
    }
    Ok(())
}

fn edit_in_external_editor(initial: &str) -> Result<String> {
    let mut file = TempFileBuilder::new()
        .prefix("runx-colorscheme-")
        .suffix(".toml")
        .tempfile()
        .context("failed to create temporary colorscheme file")?;
    file.write_all(initial.as_bytes())
        .context("failed to write temporary colorscheme file")?;
    file.flush()
        .context("failed to flush temporary colorscheme file")?;

    let editor = env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".to_owned());
    let mut parts = shell_words::split(&editor)
        .with_context(|| format!("failed to parse editor command `{editor}`"))?;
    if parts.is_empty() {
        parts.push("vi".to_owned());
    }
    let program = parts.remove(0);
    let status = Command::new(&program)
        .args(parts)
        .arg(file.path())
        .status()
        .with_context(|| format!("failed to launch editor `{program}`"))?;
    if !status.success() {
        bail!("editor `{program}` exited with {status}");
    }

    fs::read_to_string(file.path()).context("failed to read edited colorscheme snippet")
}

fn new_colorscheme_snippet(name: &str, base: Option<&str>) -> String {
    let table_name = table_header_key(name);
    let preset_base = base.unwrap_or("builtin_dark");
    let mut snippet = format!("[ui.colorschemes.{table_name}]\n");
    if let Some(base) = base {
        snippet.push_str(&format!("base = \"{base}\"\n\n"));
        snippet.push_str(&format!(
            "# Full {base} token preset. Uncomment only the tokens you want to override.\n"
        ));
    } else {
        snippet.push_str(
            r#"
# No base is selected. Uncomment every token before saving, or add one of:
# base = "builtin_dark"
# base = "builtin_light"

# Full builtin_dark token preset.
"#,
        );
    }

    for (name, value) in colorscheme_preset_values(preset_base) {
        snippet.push_str(&format!("# {name} = {}\n", toml_string(&value)));
    }
    snippet
}

fn colorscheme_preset_values(base: &str) -> Vec<(&'static str, String)> {
    builtin_colorscheme_token_values(base).unwrap_or_else(|| {
        UI_COLOR_TOKEN_NAMES
            .iter()
            .map(|token| (*token, String::new()))
            .collect()
    })
}

fn toml_string(value: &str) -> String {
    Value::from(value).to_string()
}

fn move_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        (current + delta as usize).min(len - 1)
    }
}

fn bool_summary(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn format_float(value: f64) -> String {
    value.to_string()
}

fn list_summary(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_owned()
    } else {
        values.join(", ")
    }
}

fn show_on_summary(config: &Config) -> String {
    format!("{:?}", config.window.show_on).to_lowercase()
}

fn display_picker_summary(config: &Config) -> String {
    let displays = active_displays();
    if displays.is_empty() {
        return "no displays detected".to_owned();
    }

    let display_count = displays.len();
    let override_count = config.display_overrides.len();
    let display_suffix = if display_count == 1 {
        "display"
    } else {
        "displays"
    };
    let override_suffix = if override_count == 1 {
        "override"
    } else {
        "overrides"
    };
    format!("{display_count} {display_suffix}, {override_count} {override_suffix}")
}

fn display_choice_label(
    display: &DisplayProfile,
    config: &Config,
    current_display_id: Option<u32>,
) -> String {
    let mut label = display.label();
    let mut flags = Vec::new();
    if current_display_id == Some(display.native_id) {
        flags.push("current");
    }
    if display.primary {
        flags.push("primary");
    }
    if !flags.is_empty() {
        label.push_str(&format!(" [{}]", flags.join(", ")));
    }

    let summary = config
        .display_override_for(Some(display))
        .map(display_override_values_summary)
        .unwrap_or_else(|| "global settings".to_owned());
    format!("{label} - {summary}")
}

fn display_override_values_summary(display_override: &DisplayOverrideConfig) -> String {
    let mut parts = Vec::new();
    if let Some(value) = display_override.width_fraction {
        parts.push(format!("w {}", format_float(value)));
    }
    if let Some(value) = display_override.visible_rows {
        parts.push(format!("rows {value}"));
    }
    if let Some(value) = display_override.ui_scale {
        parts.push(format!("scale {}", format_float(value)));
    }

    if parts.is_empty() {
        "custom override".to_owned()
    } else {
        parts.join(", ")
    }
}

fn parse_f64(raw: &str, label: &str) -> Result<f64> {
    raw.trim()
        .parse::<f64>()
        .with_context(|| format!("{label} must be a number"))
}

fn parse_i64(raw: &str, label: &str) -> Result<i64> {
    raw.trim()
        .parse::<i64>()
        .with_context(|| format!("{label} must be an integer"))
}

fn parse_usize(raw: &str, label: &str) -> Result<usize> {
    raw.trim()
        .parse::<usize>()
        .with_context(|| format!("{label} must be a non-negative integer"))
}

fn parse_u16(raw: &str, label: &str) -> Result<u16> {
    raw.trim()
        .parse::<u16>()
        .with_context(|| format!("{label} must be an integer between 0 and 65535"))
}

fn modifier_matches(current: &str, canonical: &str) -> bool {
    match canonical {
        "Alt" => matches!(current, "Alt" | "Option"),
        "Control" => matches!(current, "Control" | "Ctrl"),
        "Command" => matches!(current, "Command" | "Cmd" | "Super" | "Meta"),
        _ => current == canonical,
    }
}

fn item_exists(doc: &Document, path: &[&str]) -> bool {
    let Some((first, rest)) = path.split_first() else {
        return false;
    };
    let mut item = doc.get(first);
    for segment in rest {
        item = item
            .and_then(Item::as_table)
            .and_then(|table| table.get(segment));
    }
    item.is_some()
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

fn upsert_display_override(
    doc: &mut Document,
    existing_index: Option<usize>,
    display_override: &DisplayOverrideConfig,
) -> Result<()> {
    let table = display_override_table(display_override)?;
    let overrides = display_overrides_array_mut(doc)?;

    if let Some(index) = existing_index {
        let Some(existing) = overrides.get_mut(index) else {
            bail!("display override index {index} is no longer valid");
        };
        *existing = table;
    } else {
        overrides.push(table);
    }

    Ok(())
}

fn save_display_override(
    doc: &mut Document,
    existing_index: Option<usize>,
    display_override: &DisplayOverrideConfig,
) -> Result<()> {
    if display_override.has_override_values() {
        upsert_display_override(doc, existing_index, display_override)
    } else if let Some(index) = existing_index {
        remove_display_override(doc, index)
    } else {
        Ok(())
    }
}

fn remove_display_override(doc: &mut Document, index: usize) -> Result<()> {
    let remove_root = {
        let overrides = display_overrides_array_mut(doc)?;
        if index >= overrides.len() {
            bail!("display override index {index} is out of range");
        }
        overrides.remove(index);
        overrides.is_empty()
    };

    if remove_root {
        doc.as_table_mut().remove("display_overrides");
    }

    Ok(())
}

fn display_overrides_array_mut(doc: &mut Document) -> Result<&mut ArrayOfTables> {
    let item = doc
        .as_table_mut()
        .entry("display_overrides")
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    if !item.is_array_of_tables() {
        *item = Item::ArrayOfTables(ArrayOfTables::new());
    }
    item.as_array_of_tables_mut()
        .context("failed to create [[display_overrides]] array")
}

fn display_override_table(display_override: &DisplayOverrideConfig) -> Result<TomlTable> {
    let mut table = TomlTable::new();

    if let Some(built_in) = display_override.built_in {
        table.insert("built_in", value(built_in));
    }
    if let Some(vendor) = display_override.vendor {
        table.insert("vendor", value(i64::from(vendor)));
    }
    if let Some(model) = display_override.model {
        table.insert("model", value(i64::from(model)));
    }
    if let Some(serial) = display_override.serial {
        table.insert("serial", value(i64::from(serial)));
    }
    if let Some(width_fraction) = display_override.width_fraction {
        table.insert("width_fraction", value(width_fraction));
    }
    if let Some(visible_rows) = display_override.visible_rows {
        table.insert(
            "visible_rows",
            value(i64::try_from(visible_rows).context("visible_rows is too large")?),
        );
    }
    if let Some(min_width) = display_override.min_width {
        table.insert("min_width", value(min_width));
    }
    if let Some(max_width) = display_override.max_width {
        table.insert("max_width", value(max_width));
    }
    if let Some(min_height) = display_override.min_height {
        table.insert("min_height", value(min_height));
    }
    if let Some(max_height) = display_override.max_height {
        table.insert("max_height", value(max_height));
    }
    if let Some(ui_scale) = display_override.ui_scale {
        table.insert("ui_scale", value(ui_scale));
    }

    Ok(table)
}

#[derive(Debug)]
struct TableHeader {
    path: Vec<String>,
    start: usize,
}

#[derive(Debug, Clone)]
struct TomlTableSection {
    range: std::ops::Range<usize>,
}

impl TomlTableSection {
    fn find(raw: &str, target_path: &[&str]) -> Result<Option<Self>> {
        let doc = raw
            .parse::<SpannedDocument<String>>()
            .context("failed to parse config while locating TOML table section")?;
        let mut headers = Vec::new();
        collect_table_headers(doc.as_table(), &mut Vec::new(), &mut headers);
        headers.sort_unstable_by_key(|header| header.start);

        let target_path = target_path
            .iter()
            .map(|segment| (*segment).to_owned())
            .collect::<Vec<_>>();
        let Some(target) = headers.iter().find(|header| header.path == target_path) else {
            return Ok(None);
        };

        let start = line_start(raw, target.start);
        let end = headers
            .iter()
            .map(|header| line_start(raw, header.start))
            .filter(|header_start| *header_start > start)
            .min()
            .unwrap_or(raw.len());
        Ok(Some(Self { range: start..end }))
    }

    fn extract(&self, raw: &str) -> String {
        raw[self.range.clone()].to_owned()
    }

    fn replace(&self, raw: &str, replacement: &str) -> String {
        let mut replacement = replacement.to_owned();
        if !replacement.ends_with('\n') {
            replacement.push('\n');
        }

        let mut next = String::with_capacity(
            raw.len() - (self.range.end - self.range.start) + replacement.len(),
        );
        next.push_str(&raw[..self.range.start]);
        next.push_str(&replacement);
        next.push_str(&raw[self.range.end..]);
        next
    }

    fn remove(&self, raw: &str) -> String {
        let mut next = String::with_capacity(raw.len() - (self.range.end - self.range.start));
        next.push_str(&raw[..self.range.start]);
        next.push_str(&raw[self.range.end..]);
        next
    }
}

fn colorscheme_snippet(raw: &str, name: &str) -> Result<String> {
    let path = colorscheme_table_path(name);
    let Some(section) = TomlTableSection::find(raw, &path)? else {
        return Ok(format!("[ui.colorschemes.{}]\n", table_header_key(name)));
    };

    Ok(section.extract(raw))
}

fn replace_colorscheme_snippet(raw: &str, name: &str, snippet: &str) -> Result<String> {
    let path = colorscheme_table_path(name);
    let Some(section) = TomlTableSection::find(raw, &path)? else {
        bail!("colorscheme `{name}` no longer exists");
    };

    Ok(section.replace(raw, snippet))
}

fn remove_colorscheme_snippet(raw: &str, name: &str) -> Result<String> {
    let path = colorscheme_table_path(name);
    let Some(section) = TomlTableSection::find(raw, &path)? else {
        bail!("colorscheme `{name}` no longer exists");
    };

    remove_empty_colorschemes_parent(&section.remove(raw))
}

fn remove_empty_colorschemes_parent(raw: &str) -> Result<String> {
    let doc = raw
        .parse::<Document>()
        .context("failed to parse config after deleting colorscheme section")?;
    let Some(colorschemes) = doc
        .get("ui")
        .and_then(Item::as_table)
        .and_then(|ui| ui.get("colorschemes"))
        .and_then(Item::as_table)
    else {
        return Ok(raw.to_owned());
    };
    if !colorschemes.is_empty() {
        return Ok(raw.to_owned());
    }

    let Some(section) = TomlTableSection::find(raw, &["ui", "colorschemes"])? else {
        return Ok(raw.to_owned());
    };
    Ok(section.remove(raw))
}

fn colorscheme_table_path(name: &str) -> [&str; 3] {
    ["ui", "colorschemes", name]
}

fn collect_table_headers(
    table: &TomlTable,
    path: &mut Vec<String>,
    headers: &mut Vec<TableHeader>,
) {
    if let Some(span) = table.span()
        && span.start < span.end
        && !path.is_empty()
    {
        headers.push(TableHeader {
            path: path.clone(),
            start: span.start,
        });
    }

    for (key, item) in table.iter() {
        if let Some(table) = item.as_table() {
            path.push(key.to_owned());
            collect_table_headers(table, path, headers);
            path.pop();
        } else if let Some(array) = item.as_array_of_tables() {
            path.push(key.to_owned());
            for table in array.iter() {
                collect_table_headers(table, path, headers);
            }
            path.pop();
        }
    }
}

fn line_start(raw: &str, index: usize) -> usize {
    raw[..index].rfind('\n').map_or(0, |position| position + 1)
}

fn colorscheme_table_from_snippet(raw: &str, name: &str) -> Result<TomlTable> {
    let doc = raw
        .parse::<Document>()
        .context("colorscheme snippet is not valid TOML")?;
    let trailing = doc.trailing().as_str().unwrap_or("").to_owned();
    let mut table = doc
        .get("ui")
        .and_then(Item::as_table)
        .and_then(|ui| ui.get("colorschemes"))
        .and_then(Item::as_table)
        .and_then(|colorschemes| colorschemes.get(name))
        .and_then(Item::as_table)
        .cloned()
        .with_context(|| {
            format!(
                "snippet must contain [ui.colorschemes.{}]",
                table_header_key(name)
            )
        })?;
    append_colorscheme_trailing(&mut table, &trailing);
    Ok(table)
}

fn append_colorscheme_trailing(table: &mut TomlTable, trailing: &str) {
    if trailing.trim().is_empty() {
        return;
    }

    if let Some((_, item)) = table.iter_mut().last()
        && let Some(value) = item.as_value_mut()
    {
        append_decor_suffix(value.decor_mut(), trailing);
        return;
    }

    append_decor_suffix(table.decor_mut(), trailing);
}

fn append_decor_suffix(decor: &mut Decor, trailing: &str) {
    let mut suffix = decor
        .suffix()
        .and_then(|raw| raw.as_str())
        .unwrap_or("")
        .to_owned();
    if !trailing.starts_with(['\n', '\r']) {
        suffix.push('\n');
    }
    suffix.push_str(trailing);
    decor.set_suffix(suffix);
}

fn set_colorscheme_table(doc: &mut Document, name: &str, table: TomlTable) -> Result<()> {
    table_mut(doc, &["ui", "colorschemes"])?.insert(name, Item::Table(table));
    Ok(())
}

fn validate_custom_colorscheme_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("colorscheme name must not be empty");
    }
    if name == "system" || BUILTIN_COLORSCHEME_NAMES.contains(&name) {
        bail!("custom colorscheme name must not be `system` or a built-in name");
    }
    Ok(())
}

fn table_header_key(key: &str) -> String {
    Key::new(key).display_repr().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_colorscheme_snippet_includes_full_commented_base_preset() {
        let snippet = new_colorscheme_snippet("solarized", Some("builtin_light"));

        assert!(snippet.contains("[ui.colorschemes.solarized]"));
        assert!(snippet.contains("base = \"builtin_light\""));
        for (token, value) in colorscheme_preset_values("builtin_light") {
            assert!(
                snippet.contains(&format!("# {token} = {}", toml_string(&value))),
                "missing commented token `{token}`"
            );
        }
    }

    #[test]
    fn no_base_colorscheme_preset_can_be_uncommented_into_complete_scheme() {
        let snippet = new_colorscheme_snippet("gruvbox", None);
        let uncommented = uncomment_color_token_lines(&snippet);
        let raw = format!("[ui]\ncolorscheme = \"gruvbox\"\n{uncommented}");

        validate_config_toml(std::path::Path::new("/tmp/runx-config.toml"), &raw)
            .expect("uncommented full preset should be a complete colorscheme");
    }

    #[test]
    fn edit_colorscheme_snippet_starts_at_selected_table() {
        let raw = r##"[ui]
colorscheme = "gruvbox"

[ui.colorschemes.gruvbox]
base = "builtin_dark"

# keep me
accent = "#fabd2f"
input_bg = "linear-gradient(180deg, rgba(50, 48, 67, 0.98), rgba(40, 40, 60, 0.98))"
# item_hover = "rgba(235, 219, 178, 0.085)"
# item_selected_bg = "linear-gradient(135deg, rgba(215, 153, 33, 0.24), rgba(60, 56, 54, 0.98))"

[window]
width_fraction = 0.6
"##;

        let snippet =
            colorscheme_snippet(raw, "gruvbox").expect("colorscheme snippet should render");

        assert!(snippet.starts_with("[ui.colorschemes.gruvbox]\n"));
        assert!(!snippet.starts_with("[ui]\n"));
        assert!(!snippet.contains("\n[ui.colorschemes]\n"));
        assert!(snippet.contains("# keep me"));
        assert!(snippet.contains("accent = \"#fabd2f\""));
        assert!(snippet.contains("input_bg = \"linear-gradient"));
        assert!(snippet.contains("# item_hover = \"rgba(235, 219, 178, 0.085)\""));
        assert!(snippet.contains("# item_selected_bg = \"linear-gradient"));
        assert!(!snippet.contains("[window]"));
    }

    #[test]
    fn edit_colorscheme_snippet_uses_parsed_table_headers_as_boundaries() {
        let raw = r##"[ui]
colorscheme = "gruvbox"

[ui.colorschemes.gruvbox]
base = "builtin_dark"
input_bg = """
[window]
"""
# item_hover = "rgba(235, 219, 178, 0.085)"

[window]
width_fraction = 0.6
"##;

        let snippet =
            colorscheme_snippet(raw, "gruvbox").expect("colorscheme snippet should render");

        assert!(snippet.contains("input_bg = \"\"\"\n[window]\n\"\"\""));
        assert!(snippet.contains("# item_hover = \"rgba(235, 219, 178, 0.085)\""));
        assert!(!snippet.contains("width_fraction = 0.6"));
    }

    #[test]
    fn replacing_colorscheme_snippet_does_not_duplicate_trailing_comments() {
        let raw = r##"[ui]
colorscheme = "gruvbox"

[ui.colorschemes.gruvbox]
base = "builtin_dark"
input_bg = "linear-gradient(180deg, #111111, #222222)"
# item_hover = "rgba(235, 219, 178, 0.085)"
# item_selected_bg = "linear-gradient(135deg, rgba(215, 153, 33, 0.24), rgba(60, 56, 54, 0.98))"

[window]
width_fraction = 0.6
"##;
        let snippet =
            colorscheme_snippet(raw, "gruvbox").expect("colorscheme snippet should render");

        let once =
            replace_colorscheme_snippet(raw, "gruvbox", &snippet).expect("snippet should replace");
        let twice = replace_colorscheme_snippet(&once, "gruvbox", &snippet)
            .expect("snippet should replace again");

        assert_eq!(once, twice);
        assert_eq!(twice.matches("# item_hover =").count(), 1);
        assert_eq!(twice.matches("# item_selected_bg =").count(), 1);
    }

    #[test]
    fn removing_last_colorscheme_removes_section_comments_and_empty_parent() {
        let raw = r##"[ui]
colorscheme = "gruvbox"

[ui.colorschemes]

[ui.colorschemes.gruvbox]
base = "builtin_dark"

# Full builtin_dark token preset. Uncomment only the tokens you want to override.
accent = "#fabd2f"
# item_selected_bg = "linear-gradient(135deg, rgba(215, 153, 33, 0.24), rgba(60, 56, 54, 0.98))"

[[display_overrides]]
app = "Terminal"
"##;
        let next =
            remove_colorscheme_snippet(raw, "gruvbox").expect("colorscheme snippet should remove");

        assert!(!next.contains("[ui.colorschemes]"));
        assert!(!next.contains("[ui.colorschemes.gruvbox]"));
        assert!(!next.contains("Full builtin_dark token preset"));
        assert!(!next.contains("item_selected_bg"));
        assert!(next.contains("[[display_overrides]]"));
    }

    #[test]
    fn removing_colorscheme_keeps_sibling_colorschemes() {
        let raw = r##"[ui.colorschemes]
# shared colorscheme notes

[ui.colorschemes.gruvbox]
base = "builtin_dark"
# remove me
accent = "#fabd2f"

[ui.colorschemes.solarized]
base = "builtin_light"
accent = "#268bd2"
"##;
        let next =
            remove_colorscheme_snippet(raw, "gruvbox").expect("colorscheme snippet should remove");

        assert!(next.contains("[ui.colorschemes]"));
        assert!(next.contains("# shared colorscheme notes"));
        assert!(!next.contains("[ui.colorschemes.gruvbox]"));
        assert!(!next.contains("# remove me"));
        assert!(next.contains("[ui.colorschemes.solarized]"));
        assert!(next.contains("accent = \"#268bd2\""));
    }

    #[test]
    fn colorscheme_table_roundtrip_preserves_trailing_comments() {
        let raw = r##"[ui.colorschemes.gruvbox]
base = "builtin_dark"

# keep me
# accent = "#fabd2f"
"##;
        let table = colorscheme_table_from_snippet(raw, "gruvbox").expect("snippet should parse");
        let mut doc = Document::new();
        set_colorscheme_table(&mut doc, "gruvbox", table).expect("table should insert");
        let saved = doc.to_string();

        assert!(saved.contains("# keep me"));
        assert!(saved.contains("# accent = \"#fabd2f\""));
    }

    #[test]
    fn colorscheme_table_roundtrip_does_not_duplicate_trailing_comments() {
        let raw = r##"[ui.colorschemes.gruvbox]
base = "builtin_dark"
input_bg = "linear-gradient(180deg, #111111, #222222)"
# item_hover = "rgba(235, 219, 178, 0.085)"
# item_selected_bg = "linear-gradient(135deg, rgba(215, 153, 33, 0.24), rgba(60, 56, 54, 0.98))"
"##;
        let table = colorscheme_table_from_snippet(raw, "gruvbox").expect("snippet should parse");
        let mut doc = Document::new();
        set_colorscheme_table(&mut doc, "gruvbox", table).expect("table should insert");
        let saved = doc.to_string();

        assert_eq!(saved.matches("# item_hover =").count(), 1);
        assert_eq!(saved.matches("# item_selected_bg =").count(), 1);
    }

    #[test]
    fn colorscheme_table_roundtrip_preserves_empty_table_trailing_comments() {
        let raw = r##"[ui.colorschemes.gruvbox]

# keep me
# accent = "#fabd2f"
"##;
        let table = colorscheme_table_from_snippet(raw, "gruvbox").expect("snippet should parse");
        let mut doc = Document::new();
        set_colorscheme_table(&mut doc, "gruvbox", table).expect("table should insert");
        let saved = doc.to_string();

        assert!(saved.contains("# keep me"));
        assert!(saved.contains("# accent = \"#fabd2f\""));
    }

    #[test]
    fn colorscheme_table_roundtrip_keeps_adjacent_trailing_comments_on_next_line() {
        let raw = r##"[ui.colorschemes.gruvbox]
canvas_bg = "#0000ff"
# panel = "#282828"
"##;
        let table = colorscheme_table_from_snippet(raw, "gruvbox").expect("snippet should parse");
        let mut doc = Document::new();
        set_colorscheme_table(&mut doc, "gruvbox", table).expect("table should insert");
        let saved = doc.to_string();

        assert!(saved.contains("canvas_bg = \"#0000ff\"\n# panel = \"#282828\""));
        assert!(!saved.contains("canvas_bg = \"#0000ff\"# panel"));
    }

    fn uncomment_color_token_lines(raw: &str) -> String {
        let mut lines = Vec::new();
        for line in raw.lines() {
            let Some(uncommented) = line.strip_prefix("# ") else {
                lines.push(line.to_owned());
                continue;
            };
            let Some((token, _)) = uncommented.split_once(" = ") else {
                lines.push(line.to_owned());
                continue;
            };
            if UI_COLOR_TOKEN_NAMES.contains(&token) {
                lines.push(uncommented.to_owned());
            } else {
                lines.push(line.to_owned());
            }
        }
        lines.join("\n")
    }
}
