mod cmux;
mod files;
mod navigation;
mod ui;

use std::{
    env, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::Result;
use cmux_client::{ClientConfig, CmuxClient};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use files::{FileEntry, filtered_indices, list_directory};
use navigation::Navigation;
use ratatui::{Terminal, backend::CrosstermBackend};

const REFRESH_EVERY: Duration = Duration::from_secs(2);
const POLL_EVERY: Duration = Duration::from_millis(100);
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_millis(500);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(8);

fn main() -> Result<()> {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));
    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new()?;
    app.connect_or_schedule();
    loop {
        terminal.draw(|frame| ui::draw(frame, &app.view()))?;
        if event::poll(POLL_EVERY)?
            && let Event::Key(key) = event::read()?
            && app.handle_key(key)
        {
            break;
        }
        app.tick();
    }
    Ok(())
}

struct App {
    navigation: Navigation,
    fallback_cwd: PathBuf,
    entries: Vec<FileEntry>,
    visible: Vec<usize>,
    selected: usize,
    show_hidden: bool,
    filter_mode: bool,
    query: String,
    listing_error: Option<String>,
    client: Option<CmuxClient>,
    socket_path: Option<PathBuf>,
    status: Status,
    last_refresh: Instant,
    next_reconnect: Instant,
    reconnect_delay: Duration,
}

enum Status {
    Ready { message: Option<String> },
    Reconnecting { message: String },
}

impl App {
    fn new() -> Result<Self> {
        let fallback_cwd = env::current_dir()?;
        let mut app = Self {
            navigation: Navigation::new(fallback_cwd.clone()),
            fallback_cwd,
            entries: Vec::new(),
            visible: Vec::new(),
            selected: 0,
            show_hidden: false,
            filter_mode: false,
            query: String::new(),
            listing_error: None,
            client: None,
            socket_path: None,
            status: Status::Reconnecting {
                message: "connecting".into(),
            },
            last_refresh: Instant::now(),
            next_reconnect: Instant::now(),
            reconnect_delay: INITIAL_RECONNECT_DELAY,
        };
        app.reload_directory();
        Ok(app)
    }

    fn view(&self) -> ui::View<'_> {
        ui::View {
            path: self.navigation.current_dir().to_string_lossy().into_owned(),
            rows: self
                .visible
                .iter()
                .map(|index| &self.entries[*index])
                .collect(),
            total_rows: self.entries.len(),
            selected: self.selected,
            filter_mode: self.filter_mode,
            query: &self.query,
            show_hidden: self.show_hidden,
            pinned: self.navigation.is_pinned(),
            status: match &self.status {
                Status::Ready { message } => ui::ViewStatus::Ready {
                    message: message.as_deref(),
                },
                Status::Reconnecting { message } => ui::ViewStatus::Reconnecting { message },
            },
            listing_error: self.listing_error.as_deref(),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }

        if self.filter_mode {
            match key.code {
                KeyCode::Esc if !self.query.is_empty() => {
                    let keep = self.selected_entry().map(|entry| entry.path);
                    self.query.clear();
                    self.apply_filter(keep.as_deref());
                }
                KeyCode::Esc => self.filter_mode = false,
                KeyCode::Enter => {
                    // Activate the filtered selection; leave filter input so the
                    // action lands in a normal-mode view.
                    self.filter_mode = false;
                    self.activate_selected();
                }
                KeyCode::Right => {
                    self.filter_mode = false;
                    self.descend_selected();
                }
                KeyCode::Backspace => {
                    let keep = self.selected_entry().map(|entry| entry.path);
                    self.query.pop();
                    self.apply_filter(keep.as_deref());
                }
                KeyCode::Up => self.move_selection(-1),
                KeyCode::Down => self.move_selection(1),
                KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.move_selection(-1)
                }
                KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.move_selection(1)
                }
                KeyCode::Char(ch)
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    let keep = self.selected_entry().map(|entry| entry.path);
                    self.query.push(ch);
                    self.apply_filter(keep.as_deref());
                }
                _ => {}
            }
            return false;
        }

        match key.code {
            KeyCode::Esc => {}
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(-1)
            }
            KeyCode::Down => self.move_selection(1),
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(1)
            }
            KeyCode::Right => self.descend_selected(),
            KeyCode::Enter => self.activate_selected(),
            KeyCode::Left | KeyCode::Char('h') => self.go_parent(),
            KeyCode::Char('.') => {
                self.show_hidden = !self.show_hidden;
                self.reload_directory();
            }
            KeyCode::Char('/') => self.filter_mode = true,
            KeyCode::Char('~') => self.reroot(),
            KeyCode::Char('c') => self.cd_selected(),
            KeyCode::Char('o') => self.browser_selected(),
            _ => {}
        }
        false
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self.client.is_none() {
            if now >= self.next_reconnect {
                self.connect_or_schedule();
            }
            return;
        }
        if now.duration_since(self.last_refresh) >= REFRESH_EVERY {
            self.refresh();
        }
    }

    fn connect_or_schedule(&mut self) {
        let socket_path = match env::var_os("CMUX_TUI_SOCKET")
            .or_else(|| env::var_os("CMUX_MUX_SOCKET"))
        {
            Some(path) if !path.is_empty() => PathBuf::from(path),
            _ => {
                self.socket_path = None;
                self.disconnect_with_backoff(
                    "CMUX_TUI_SOCKET is not set. Launch from cmux or set it for standalone development."
                        .into(),
                );
                return;
            }
        };
        self.socket_path = Some(socket_path.clone());
        match CmuxClient::connect(ClientConfig::from_socket_path(socket_path)) {
            Ok(mut client) => match cmux::focused_target(&mut client) {
                Ok((_, cwd)) => {
                    if let Some(cwd) = cwd {
                        self.navigation.follow_focused_cwd(Path::new(&cwd));
                    }
                    self.client = Some(client);
                    self.status = Status::Ready { message: None };
                    self.reconnect_delay = INITIAL_RECONNECT_DELAY;
                    self.last_refresh = Instant::now();
                    self.reload_directory();
                }
                Err(error) => {
                    self.disconnect_with_backoff(format!("cmux did not respond: {error}"))
                }
            },
            Err(error) => self.disconnect_with_backoff(format!("cannot connect to cmux: {error}")),
        }
    }

    fn refresh(&mut self) {
        let result = self
            .client
            .as_mut()
            .map(cmux::focused_target)
            .expect("refresh requires a client");
        match result {
            Ok((_, cwd)) => {
                if let Some(cwd) = cwd {
                    self.navigation.follow_focused_cwd(Path::new(&cwd));
                }
                self.last_refresh = Instant::now();
                self.reload_directory();
            }
            Err(error) => self.disconnect(format!("cmux socket dropped: {error}")),
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.visible.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self
                .selected
                .saturating_add_signed(delta)
                .min(self.visible.len() - 1);
        }
    }

    fn selected_entry(&self) -> Option<FileEntry> {
        self.visible
            .get(self.selected)
            .map(|index| self.entries[*index].clone())
    }

    fn descend_selected(&mut self) {
        if let Some(entry) = self.selected_entry().filter(FileEntry::is_dir) {
            self.navigation.navigate(entry.path);
            self.query.clear();
            self.reload_directory();
        }
    }

    fn go_parent(&mut self) {
        if let Some(parent) = self.navigation.current_dir().parent() {
            self.navigation.navigate(parent.to_path_buf());
            self.query.clear();
            self.reload_directory();
        }
    }

    fn activate_selected(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        if entry.is_dir() {
            self.navigation.navigate(entry.path);
            self.query.clear();
            self.reload_directory();
            return;
        }
        self.with_focused_target(|client, target| cmux::open_editor(client, target, &entry.path));
    }

    fn cd_selected(&mut self) {
        let Some(entry) = self.selected_entry().filter(FileEntry::is_dir) else {
            self.set_message("select a directory to send cd");
            return;
        };
        self.with_focused_target(|client, target| cmux::send_cd(client, target, &entry.path));
    }

    fn browser_selected(&mut self) {
        let Some(entry) = self.selected_entry().filter(|entry| !entry.is_dir()) else {
            self.set_message("select an .html or .md file to open");
            return;
        };
        let supported = entry
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("md")
            });
        if !supported {
            self.set_message("only .html and .md files open in a browser");
            return;
        }
        self.with_focused_target(|client, target| cmux::open_browser(client, target, &entry.path));
    }

    fn reroot(&mut self) {
        let focused_cwd = self
            .client
            .as_mut()
            .and_then(|client| cmux::focused_target(client).ok())
            .and_then(|(_, cwd)| cwd)
            .map(PathBuf::from)
            .unwrap_or_else(|| self.fallback_cwd.clone());
        self.navigation.reroot(focused_cwd);
        self.query.clear();
        self.reload_directory();
    }

    fn with_focused_target(
        &mut self,
        action: impl FnOnce(&mut CmuxClient, cmux::FocusedTarget) -> Result<()>,
    ) {
        let result = match self.client.as_mut() {
            Some(client) => {
                cmux::focused_target(client).and_then(|(target, _)| action(client, target))
            }
            None => {
                self.set_message("not connected to cmux");
                return;
            }
        };
        match result {
            Ok(()) => self.set_message("sent to focused pane"),
            Err(error) => self.disconnect(format!("cmux command failed: {error}")),
        }
    }

    fn reload_directory(&mut self) {
        let selected_path = self.selected_entry().map(|entry| entry.path);
        match list_directory(self.navigation.current_dir(), self.show_hidden) {
            Ok(entries) => {
                self.entries = entries;
                self.listing_error = None;
            }
            Err(error) => {
                self.entries.clear();
                self.listing_error = Some(error.to_string());
            }
        }
        self.apply_filter(selected_path.as_deref());
    }

    fn apply_filter(&mut self, selected_path: Option<&Path>) {
        self.visible = filtered_indices(&self.entries, &self.query);
        self.selected = selected_path
            .and_then(|path| {
                self.visible
                    .iter()
                    .position(|index| self.entries[*index].path == path)
            })
            .unwrap_or_else(|| {
                if self.visible.is_empty() {
                    0
                } else {
                    self.selected.min(self.visible.len() - 1)
                }
            });
    }

    fn set_message(&mut self, message: impl Into<String>) {
        if self.client.is_some() {
            self.status = Status::Ready {
                message: Some(message.into()),
            };
        }
    }

    fn disconnect(&mut self, message: String) {
        self.client = None;
        self.disconnect_with_backoff(message);
    }

    fn disconnect_with_backoff(&mut self, message: String) {
        self.status = Status::Reconnecting { message };
        self.next_reconnect = Instant::now() + self.reconnect_delay;
        self.reconnect_delay = (self.reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
    }
}
