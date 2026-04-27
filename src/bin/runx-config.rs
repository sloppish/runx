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
    BUILTIN_COLORSCHEME_NAMES, Config, KNOWN_PROVIDER_NAMES, ensure_user_config,
    validate_config_toml,
};
use tempfile::Builder as TempFileBuilder;
use toml_edit::{Array, Document, Item, Key, Table as TomlTable, Value, value};

const SECTIONS: [Section; 8] = [
    Section::Hotkey,
    Section::Window,
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
const PATH_APPS_EXACT_NAME_BOOST: &[&str] = &["providers", "apps", "exact_name_boost"];
const PATH_APPS_PREFIX_NAME_BOOST: &[&str] = &["providers", "apps", "prefix_name_boost"];
const PATH_RANKING_TIE_THRESHOLD: &[&str] = &["ranking", "tie_threshold"];
const PATH_RANKING_RESULT_LIMIT: &[&str] = &["ranking", "result_limit"];
const PATH_RANKING_PROVIDER_ORDER: &[&str] = &["ranking", "provider_order"];
const PATH_RANKING_EMPTY_QUERY_PROVIDERS: &[&str] = &["ranking", "empty_query_providers"];
const PATH_UI_SHOW_HEADER: &[&str] = &["ui", "show_header"];
const PATH_UI_CYCLE_SELECTION: &[&str] = &["ui", "cycle_selection"];
const PATH_UI_FONT_FAMILY: &[&str] = &["ui", "font_family"];
const PATH_UI_SCALE: &[&str] = &["ui", "scale"];
const PATH_UI_COLORSCHEME: &[&str] = &["ui", "colorscheme"];
const PATH_UI_CANVAS_SHOW: &[&str] = &["ui", "canvas", "show"];
const PATH_UI_CANVAS_RADIUS: &[&str] = &["ui", "canvas", "radius"];
const PATH_UI_CANVAS_OPACITY: &[&str] = &["ui", "canvas", "opacity"];
const PATH_UI_CANVAS_BACKGROUND_OPACITY: &[&str] = &["ui", "canvas", "background_opacity"];
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
        Ok(Self { path, doc, config })
    }

    fn reload(&mut self) -> Result<()> {
        let next = Self::load(self.path.clone())?;
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
        let config = validate_config_toml(&self.path, &raw)?;
        fs::write(&self.path, raw)
            .with_context(|| format!("failed to write {}", self.path.display()))?;
        self.config = config;
        Ok(())
    }

    fn validate_current(&self) -> Result<()> {
        validate_config_toml(&self.path, &self.doc.to_string())?;
        Ok(())
    }

    fn source_for(&self, path: &[&str]) -> &'static str {
        if item_exists(&self.doc, path) {
            "config.toml"
        } else {
            "default"
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Hotkey,
    Window,
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
            | FieldId::UiCanvasOpacity
            | FieldId::UiCanvasBackgroundOpacity
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
            FieldId::RankingProviderOrder => {
                self.mode = Mode::ProviderOrder(ProviderOrderMode::new(&self.editor.config));
                Ok(AppAction::None)
            }
            FieldId::RankingEmptyQueryProviders => {
                self.mode = Mode::Toggle(ToggleMode::providers(
                    "Empty-query providers",
                    ToggleTarget::EmptyQueryProviders,
                    &self.editor.config.ranking.empty_query_providers,
                ));
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
                self.mode = Mode::Choice(ChoiceMode::edit_colorscheme(&self.editor.config));
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
            FieldId::UiCanvasOpacity => {
                let parsed = parse_f64(&raw, "canvas opacity")?;
                self.apply(|doc| set_item(doc, &["ui", "canvas"], "opacity", value(parsed)))?;
                self.success("Saved canvas.opacity");
            }
            FieldId::UiCanvasBackgroundOpacity => {
                let parsed = parse_f64(&raw, "canvas background opacity")?;
                self.apply(|doc| {
                    set_item(doc, &["ui", "canvas"], "background_opacity", value(parsed))
                })?;
                self.success("Saved canvas.background_opacity");
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
                    let reset_selection = self.editor.config.ui.colorscheme == name;
                    self.apply(|doc| {
                        remove_item(doc, &["ui", "colorschemes", name.as_str()])?;
                        if reset_selection {
                            set_item(doc, &["ui"], "colorscheme", value("system"))?;
                        }
                        Ok(())
                    })?;
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
            ToggleTarget::EmptyQueryProviders => {
                self.apply(|doc| {
                    set_item(
                        doc,
                        &["ranking"],
                        "empty_query_providers",
                        string_array(&selected),
                    )
                })?;
                self.success("Saved empty-query providers");
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
        if !item_exists(&self.editor.doc, path) {
            self.info("Field already uses the default");
            return Ok(());
        }
        self.apply(|doc| remove_item(doc, path))?;
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
}

#[derive(Clone, Copy)]
enum InputTarget {
    Field(FieldId),
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

    fn edit_colorscheme(config: &Config) -> Self {
        let mut names = BUILTIN_COLORSCHEME_NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        for name in config.ui.colorschemes.keys() {
            if !names.iter().any(|existing| existing == name) {
                names.push(name.clone());
            }
        }
        names.sort();
        Self {
            title: "Edit colorscheme".to_owned(),
            options: names.into_iter().map(ChoiceOption::from).collect(),
            selected: 0,
            target: ChoiceTarget::EditColorscheme,
        }
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
    ConfirmDeleteColorscheme { name: String },
    ColorschemeBase { name: String },
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
    EmptyQueryProviders,
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
    ProvidersDisabled,
    WindowsIncludeOtherDesktops,
    AppsExactNameBoost,
    AppsPrefixNameBoost,
    RankingTieThreshold,
    RankingResultLimit,
    RankingProviderOrder,
    RankingEmptyQueryProviders,
    UiShowHeader,
    UiCycleSelection,
    UiFontFamily,
    UiScale,
    UiColorscheme,
    UiCanvasShow,
    UiCanvasRadius,
    UiCanvasOpacity,
    UiCanvasBackgroundOpacity,
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
            Self::ProvidersDisabled => "Disabled providers",
            Self::WindowsIncludeOtherDesktops => "Include windows from other desktops",
            Self::AppsExactNameBoost => "Exact app name boost",
            Self::AppsPrefixNameBoost => "Prefix app name boost",
            Self::RankingTieThreshold => "Tie threshold",
            Self::RankingResultLimit => "Result limit",
            Self::RankingProviderOrder => "Provider order",
            Self::RankingEmptyQueryProviders => "Empty-query providers",
            Self::UiShowHeader => "Show header",
            Self::UiCycleSelection => "Cycle selection",
            Self::UiFontFamily => "Font family",
            Self::UiScale => "UI scale",
            Self::UiColorscheme => "Selected colorscheme",
            Self::UiCanvasShow => "Show canvas",
            Self::UiCanvasRadius => "Canvas radius",
            Self::UiCanvasOpacity => "Canvas opacity",
            Self::UiCanvasBackgroundOpacity => "Canvas background opacity",
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
            Self::ProvidersDisabled => Some(PATH_PROVIDERS_DISABLED),
            Self::WindowsIncludeOtherDesktops => Some(PATH_WINDOWS_INCLUDE_OTHER_DESKTOPS),
            Self::AppsExactNameBoost => Some(PATH_APPS_EXACT_NAME_BOOST),
            Self::AppsPrefixNameBoost => Some(PATH_APPS_PREFIX_NAME_BOOST),
            Self::RankingTieThreshold => Some(PATH_RANKING_TIE_THRESHOLD),
            Self::RankingResultLimit => Some(PATH_RANKING_RESULT_LIMIT),
            Self::RankingProviderOrder => Some(PATH_RANKING_PROVIDER_ORDER),
            Self::RankingEmptyQueryProviders => Some(PATH_RANKING_EMPTY_QUERY_PROVIDERS),
            Self::UiShowHeader => Some(PATH_UI_SHOW_HEADER),
            Self::UiCycleSelection => Some(PATH_UI_CYCLE_SELECTION),
            Self::UiFontFamily => Some(PATH_UI_FONT_FAMILY),
            Self::UiScale => Some(PATH_UI_SCALE),
            Self::UiColorscheme => Some(PATH_UI_COLORSCHEME),
            Self::UiCanvasShow => Some(PATH_UI_CANVAS_SHOW),
            Self::UiCanvasRadius => Some(PATH_UI_CANVAS_RADIUS),
            Self::UiCanvasOpacity => Some(PATH_UI_CANVAS_OPACITY),
            Self::UiCanvasBackgroundOpacity => Some(PATH_UI_CANVAS_BACKGROUND_OPACITY),
            Self::UiEntriesOpacity => Some(PATH_UI_ENTRIES_OPACITY),
            Self::ColorschemeCreate | Self::ColorschemeEdit | Self::ColorschemeDelete => None,
        }
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
            Self::ProvidersDisabled => list_summary(&config.providers.disabled),
            Self::WindowsIncludeOtherDesktops => {
                bool_summary(config.providers.windows.include_other_desktops).to_owned()
            }
            Self::AppsExactNameBoost => config.providers.apps.exact_name_boost.to_string(),
            Self::AppsPrefixNameBoost => config.providers.apps.prefix_name_boost.to_string(),
            Self::RankingTieThreshold => config.ranking.tie_threshold.to_string(),
            Self::RankingResultLimit => config.ranking.result_limit.to_string(),
            Self::RankingProviderOrder => list_summary(&config.ranking.provider_order),
            Self::RankingEmptyQueryProviders => list_summary(&config.ranking.empty_query_providers),
            Self::UiShowHeader => bool_summary(config.ui.show_header).to_owned(),
            Self::UiCycleSelection => bool_summary(config.ui.cycle_selection).to_owned(),
            Self::UiFontFamily => config.ui.font_family.clone(),
            Self::UiScale => format_float(config.ui.scale),
            Self::UiColorscheme => config.ui.colorscheme.clone(),
            Self::UiCanvasShow => bool_summary(config.ui.canvas.show).to_owned(),
            Self::UiCanvasRadius => config.ui.canvas.radius.to_string(),
            Self::UiCanvasOpacity => format_float(config.ui.canvas.opacity),
            Self::UiCanvasBackgroundOpacity => format_float(config.ui.canvas.background_opacity),
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
        Section::Providers => &[
            FieldId::ProvidersDisabled,
            FieldId::WindowsIncludeOtherDesktops,
            FieldId::AppsExactNameBoost,
            FieldId::AppsPrefixNameBoost,
        ],
        Section::Ranking => &[
            FieldId::RankingTieThreshold,
            FieldId::RankingResultLimit,
            FieldId::RankingProviderOrder,
            FieldId::RankingEmptyQueryProviders,
        ],
        Section::UiBasics => &[
            FieldId::UiShowHeader,
            FieldId::UiCycleSelection,
            FieldId::UiFontFamily,
            FieldId::UiScale,
            FieldId::UiCanvasShow,
            FieldId::UiCanvasRadius,
            FieldId::UiCanvasOpacity,
            FieldId::UiCanvasBackgroundOpacity,
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
            let source = id
                .path()
                .map(|path| editor.source_for(path).to_owned())
                .unwrap_or_else(|| "action".to_owned());
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
            let initial = new_colorscheme_snippet(&name, base.as_deref());
            let edited = edit_in_external_editor(&initial)?;
            let table = colorscheme_table_from_snippet(&edited, &name)?;
            app.apply(|doc| {
                set_colorscheme_table(doc, &name, table)?;
                set_item(doc, &["ui"], "colorscheme", value(name.clone()))?;
                Ok(())
            })?;
            app.success(format!("Created and selected colorscheme `{name}`"));
        }
        ExternalEditorRequest::EditColorscheme { name } => {
            let initial = colorscheme_snippet(&app.editor.doc, &name)?;
            let edited = edit_in_external_editor(&initial)?;
            let table = colorscheme_table_from_snippet(&edited, &name)?;
            app.apply(|doc| set_colorscheme_table(doc, &name, table))?;
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
    match base {
        Some(base) => format!(
            r#"[ui.colorschemes.{table_name}]
base = "{base}"

# Add only the color tokens you want to override.
"#
        ),
        None => format!(
            r#"[ui.colorschemes.{table_name}]

# No base is selected. Define every color token before saving, or add:
# base = "builtin_dark"
# base = "builtin_light"

# Add color tokens here.
"#
        ),
    }
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

fn colorscheme_snippet(doc: &Document, name: &str) -> Result<String> {
    let Some(table) = doc
        .get("ui")
        .and_then(Item::as_table)
        .and_then(|ui| ui.get("colorschemes"))
        .and_then(Item::as_table)
        .and_then(|colorschemes| colorschemes.get(name))
        .and_then(Item::as_table)
    else {
        return Ok(format!("[ui.colorschemes.{}]\n", table_header_key(name)));
    };

    let mut snippet = Document::new();
    set_colorscheme_table(&mut snippet, name, table.clone())?;
    Ok(snippet.to_string())
}

fn colorscheme_table_from_snippet(raw: &str, name: &str) -> Result<TomlTable> {
    let doc = raw
        .parse::<Document>()
        .context("colorscheme snippet is not valid TOML")?;
    doc.get("ui")
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
        })
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
