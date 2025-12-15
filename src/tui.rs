//! Terminal user interface for `huk`.
//!
//! Provides an interactive dashboard for browsing hooks, running them with
//! confirmation and captured output, and editing hook definitions without
//! leaving the terminal.

#![allow(dead_code)]

use std::io::Stdout;
use std::io::{self};
use std::path::Path;
use std::time::Duration;

use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableMouseCapture;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use crossterm::event::{self};
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use derive_more::with_trait::Constructor;
use moos::CowStr;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::Padding;
use ratatui::widgets::Paragraph;

use crate::cli::DashboardOpts;
use crate::config::*;
use crate::constants::VERSION;
use crate::runner::OutputChunk;
use crate::runner::RunnerError;
use crate::runner::TaskRunner;
use crate::runner::mutate_hooks;
use crate::task::TaskSpec;

const LOG_LIMIT: usize = 2000;
const BASE_SCROLL_DELTA: usize = 2;
const FAST_SCROLL_MULTIPLIER: usize = 3;

macro_rules! match_common_input {
  ($state:expr, $prompt:expr, $code:expr) => {{
    use KeyCode::*;
    let _ = match $code {
      Backspace => $prompt.backspace(),
      Delete => $prompt.delete_char(),
      Left => $prompt.move_left(),
      Right => $prompt.move_right(),
      Home => $prompt.move_home(),
      End => $prompt.move_end(),
      Up => $prompt.move_up(),
      Down => $prompt.move_down(),
      Char(c) => $prompt.insert_char(c),
      _ => {}
    };
    $state.set_prompt($prompt)?;
    Ok(true)
  }};
}

/// Launch the dashboard. Returns an error if the terminal cannot be initialized
/// or if configuration loading fails.
pub fn handle_dashboard(_opts: &DashboardOpts) -> Result<(), RunnerError> {
  let cwd = std::env::current_dir()?;
  let cfg = HookConfig::discover(&cwd)?;
  let mut state = DashboardState::from_config(&cfg);

  enable_raw_mode().map_err(RunnerError::Io)?;
  let mut stdout = io::stdout();
  crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
    .map_err(RunnerError::Io)?;

  let backend = CrosstermBackend::new(stdout);
  let mut terminal = Terminal::new(backend).map_err(RunnerError::Io)?;

  let result = state.run(&mut terminal, &cwd);

  // Restore terminal.
  disable_raw_mode().map_err(RunnerError::Io)?;

  crossterm::execute!(
    terminal.backend_mut(),
    LeaveAlternateScreen,
    DisableMouseCapture
  )
  .map_err(RunnerError::Io)?;

  terminal.show_cursor().map_err(RunnerError::Io)?;
  result
}

trait Drawable {
  fn draw(&mut self, f: &mut ratatui::Frame<'_>);
}

trait InputHandler {
  fn handle_input(&mut self, code: KeyCode) -> Result<bool, RunnerError>;
}

trait MouseHandler {
  fn handle_mouse(&mut self, event: MouseEvent);
}

trait Runnable<'a> {
  fn run(
    &mut self,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    cwd: &'a Path,
  ) -> Result<(), RunnerError>;
}

fn wrap_text_lines(text: &str, width: u16) -> Vec<String> {
  let usable_width = width.max(1) as usize;
  let mut lines = Vec::new();
  for raw_line in text.split('\n') {
    if raw_line.is_empty() {
      lines.push(String::new());
      continue;
    }
    let mut current = String::new();
    let mut current_len = 0usize;
    for ch in raw_line.chars() {
      if current_len >= usable_width {
        lines.push(current);
        current = String::new();
        current_len = 0;
      }
      current.push(ch);
      current_len += 1;
    }
    lines.push(current);
  }
  if lines.is_empty() {
    lines.push(String::new());
  }
  lines
}

fn format_spec(spec: &TaskSpec) -> String {
  serde_json::to_string(&spec.to_json()).unwrap_or_else(|_| spec.to_string())
}

fn editable_spec(spec: &TaskSpec) -> String {
  match spec {
    TaskSpec::Single(s) => s.clone(),
    _ => serde_json::to_string(&spec.to_json())
      .unwrap_or_else(|_| spec.to_string()),
  }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum Focus {
  #[default]
  Hooks,
  Output,
}

impl Focus {
  fn next(self) -> Self {
    match self {
      Focus::Hooks => Focus::Output,
      Focus::Output => Focus::Hooks,
    }
  }

  fn prev(self) -> Self {
    self.next()
  }
}


/// Internal state for the dashboard.
#[derive(Clone, Constructor)]
pub struct DashboardState<'a> {
  pub cwd:        &'a Path,
  pub running:    bool,
  pub hooks:      Vec<(String, TaskSpec)>,
  pub index:      usize,
  pub logs:       Vec<LogEntry>,
  pub prompt:     Option<Prompt>,
  pub focus:      Focus,
  pub log_scroll: usize,
  pub source:     String,
}

impl<'a> Default for DashboardState<'a> {
  fn default() -> Self {
    Self {
      cwd:        Path::new("."),
      running:    false,
      hooks:      Vec::new(),
      index:      0,
      logs:       Vec::new(),
      prompt:     None,
      focus:      Focus::Hooks,
      log_scroll: 0,
      source:     String::new(),
    }
  }
}

trait HookManager<'a>
where
  Self: Sized + 'a,
{
  fn selected_hook(&'a self) -> Option<(CowStr<'a>, &'a TaskSpec)>;
  fn cwd(&self) -> &Path;

  fn add_hook<T: TryInto<TaskSpec>>(
    &mut self,
    name: &str,
    spec: T,
  ) -> Result<(), RunnerError>
  where
    <T as TryInto<TaskSpec>>::Error: Into<RunnerError>;
  fn refresh_config(&mut self) -> Result<(), RunnerError>;
  fn remove_hook(&mut self, name: &str) -> Result<(), RunnerError>;
  fn run_hook(&mut self, name: &str) -> Result<(), RunnerError>;
  fn update_hook<T: TryInto<TaskSpec>>(
    &mut self,
    name: &str,
    spec: T,
  ) -> Result<(), RunnerError>
  where
    <T as TryInto<TaskSpec>>::Error: Into<RunnerError>;
}

impl<'a> HookManager<'a> for DashboardState<'a> {
  fn cwd(&self) -> &Path {
    self.cwd
  }

  fn selected_hook(&'a self) -> Option<(CowStr<'a>, &'a TaskSpec)> {
    self.hooks.get(self.index).map(|(name, spec)| {
      (
        CowStr::try_from(name.as_str()).unwrap_or_else(|_| CowStr::from("")),
        spec,
      )
    })
  }

  fn add_hook<T: TryInto<TaskSpec>>(
    &mut self,
    hook: &str,
    spec_input: T,
  ) -> Result<(), RunnerError>
  where
    <T as TryInto<TaskSpec>>::Error: Into<RunnerError>,
  {
    ensure_valid_hook_name(hook)?;
    let spec = spec_input.try_into().map_err(Into::into)?;
    let cfg = HookConfig::discover(self.cwd)?;
    mutate_hooks(&cfg, |hooks| {
      hooks.insert(hook.to_string(), spec.to_json());
      Ok(())
    })?;
    self.refresh_config()?;
    self.select_hook(hook);
    self.push_log(LogLevel::Success, format!("Added hook '{hook}'."));
    Ok(())
  }

  fn remove_hook(&mut self, hook: &str) -> Result<(), RunnerError> {
    let cfg = HookConfig::discover(self.cwd)?;
    mutate_hooks(&cfg, |hooks| {
      hooks.remove(hook);
      Ok(())
    })?;
    self.refresh_config()?;
    self.push_log(LogLevel::Success, format!("Removed hook '{hook}'."));
    Ok(())
  }

  fn update_hook<T: TryInto<TaskSpec>>(
    &mut self,
    hook: &str,
    spec_input: T,
  ) -> Result<(), RunnerError>
  where
    <T as TryInto<TaskSpec>>::Error: Into<RunnerError>,
  {
    let spec = spec_input.try_into().map_err(Into::into)?;
    let cfg = HookConfig::discover(self.cwd)?;
    mutate_hooks(&cfg, |hooks| {
      hooks.insert(hook.to_string(), spec.to_json());
      Ok(())
    })?;
    self.refresh_config()?;
    self.select_hook(hook);
    self.push_log(LogLevel::Success, format!("Updated hook '{hook}'."));
    Ok(())
  }

  fn refresh_config(&mut self) -> Result<(), RunnerError> {
    match HookConfig::discover(self.cwd) {
      Ok(cfg) => {
        self.apply_config(&cfg);
        self.push_log(LogLevel::Info, "Configuration reloaded.");
        Ok(())
      }
      Err(err) => {
        self
          .push_log(LogLevel::Error, format!("Failed to reload config: {err}"));
        Ok(())
      }
    }
  }

  fn run_hook(&mut self, name: &str) -> Result<(), RunnerError> {
    let cfg = HookConfig::discover(self.cwd)?;
    let Some(spec) = cfg.hooks.get(name) else {
      self.push_log(LogLevel::Error, format!("Hook '{name}' not found."));
      return Ok(());
    };
    self.apply_config(&cfg);
    self.select_hook(name);
    let mut runner = TaskRunner::new_with_capture(&cfg);
    self.running = true;
    self.push_log(LogLevel::Info, format!("Running hook '{name}'..."));
    let result = runner.run_spec(spec, name, &[]);
    self.running = false;
    let output = runner.take_output();
    self.append_output(output);
    if let Err(err) = result {
      self.push_log(LogLevel::Error, format!("{err}"));
    } else {
      self.push_log(LogLevel::Success, format!("Hook '{name}' finished."));
    }
    Ok(())
  }
}

impl<'a> Runnable<'a> for DashboardState<'a> {
  fn run(
    &mut self,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    cwd: &'a Path,
  ) -> Result<(), RunnerError> {
    self.cwd = cwd;

    loop {
      terminal.draw(|f| self.draw(f)).map_err(RunnerError::Io)?;

      if event::poll(Duration::from_millis(150)).map_err(RunnerError::Io)? {
        match event::read().map_err(RunnerError::Io)? {
          Event::Key(KeyEvent {
            code, modifiers, ..
          }) => {
            if self.handle_input(code)? {
              continue;
            }
            use KeyCode::*;

            match code {
              Char('q') => break,
              Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                break;
              }
              Char('\x03') | Char('\x1a') | F(4)
                if modifiers.contains(KeyModifiers::ALT) =>
              {
                break; // Ctrl-C or Ctrl-Z
              }
              Char('\x04') => {
                // Ctrl-D should exit if the prompt is empty. otherwise it
                // should be treated as meaning "finish input"
                // for the prompt, similar to Enter but without
                // adding a newline.
                if let Some(prompt) = &self.prompt {
                  if prompt.buffer.is_empty() {
                    break;
                  } else if self.handle_prompt_input(Enter)? {
                    continue;
                  }
                }
              }
              Tab => self.focus = self.focus.next(),
              BackTab => self.focus = self.focus.prev(),
              Up => match self.focus {
                Focus::Hooks => self.move_selection_up(),
                Focus::Output => self.scroll_logs(1),
              },
              Down => match self.focus {
                Focus::Hooks => self.move_selection_down(),
                Focus::Output => self.scroll_logs(-1),
              },
              Home => match self.focus {
                Focus::Hooks => self.index = 0,
                Focus::Output => self.scroll_to_log_start(),
              },
              End => match self.focus {
                Focus::Hooks => self.index = self.hooks.len().saturating_sub(1),
                Focus::Output => self.scroll_to_log_end(),
              },
              PageUp => match self.focus {
                Focus::Hooks => {
                  for _ in 0..3 {
                    self.move_selection_up();
                  }
                }
                Focus::Output => self.scroll_logs(5),
              },
              PageDown => match self.focus {
                Focus::Hooks => {
                  for _ in 0..3 {
                    self.move_selection_down();
                  }
                }
                Focus::Output => self.scroll_logs(-5),
              },
              Char('r') | Char('R') | F(5) => {
                self.refresh_config()?;
              }
              Enter => {
                if let Some((name, _)) = self.current_hook() {
                  let prompt = Prompt::confirm_run(name.to_string());
                  self.set_prompt(prompt)?;
                }
              }
              Char('a') => self.set_prompt(Prompt::add_hook_name())?,
              Char('e') => {
                if let Some((name, spec)) = self.current_hook() {
                  self.set_prompt(Prompt::update_hook(
                    name.to_string(),
                    editable_spec(spec),
                  ))?;
                }
              }
              Char('d') => {
                if let Some((name, _)) = self.current_hook() {
                  self.set_prompt(Prompt::confirm_remove(name.to_string()))?;
                }
              }
              _ => {}
            }
          }
          Event::Mouse(mouse) => self.handle_mouse(mouse),
          Event::Resize(_, _) => {
            // Clamp scrolling when the window shrinks.
            self.normalize_log_scroll();
          }
          _ => {}
        }
      }
    }
    Ok(())
  }
}

impl Drawable for DashboardState<'_> {
  fn draw(&mut self, f: &mut ratatui::Frame<'_>) {
    let status_height = self.status_height(f.area().width);

    let layout = Layout::default()
      .direction(Direction::Vertical)
      .constraints([
        Constraint::Length(3),
        Constraint::Min(10),
        Constraint::Percentage(40),
        Constraint::Length(status_height),
      ])
      .split(f.area());

    let title = format!(
      " huk dashboard — {} — {} hooks",
      self.source,
      self.hooks.len()
    );
    let header = Paragraph::new(Text::from(title))
      .style(Style::default().add_modifier(Modifier::BOLD))
      .block(
        Block::default()
          .borders(Borders::ALL)
          .border_type(BorderType::Rounded)
          .title(format!("huk v{VERSION}")),
      );
    f.render_widget(header, layout[0]);

    // Main area: list + details.
    let main = Layout::default()
      .direction(Direction::Horizontal)
      .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
      .split(layout[1]);

    let hook_items: Vec<ListItem> = self
      .hooks
      .iter()
      .enumerate()
      .map(|(i, (name, _))| {
        let marker = if i == self.index { "›" } else { " " };
        let style = if i == self.index {
          Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
        } else {
          Style::default()
        };
        ListItem::new(Span::styled(format!("{marker} {name}"), style))
      })
      .collect();

    let list = List::new(hook_items).block(
      Block::default()
        .borders(Borders::ALL)
        .border_style(if self.focus == Focus::Hooks {
          Style::default().fg(Color::Yellow)
        } else {
          Style::default()
        })
        .border_type(BorderType::Rounded)
        .padding(Padding::uniform(1))
        .title("Hooks (↑/↓ to move, Enter to run, a/e/d to add/edit/delete, r to reload, q to quit)"),
    );
    f.render_widget(list, main[0]);

    let spec_text = if let Some((name, spec)) = self.current_hook() {
      let mut text = format!("Hook: {name}\n\n");
      text.push_str(&format_spec(spec));
      text
    } else {
      "No hooks configured.".into()
    };
    let detail = Paragraph::new(spec_text)
      .block(
        Block::default()
          .borders(Borders::ALL)
          .border_type(BorderType::Rounded)
          .padding(Padding::uniform(1))
          .title("Task Specification"),
      )
      .wrap(ratatui::widgets::Wrap { trim: true });
    f.render_widget(detail, main[1]);

    // Log panel.
    let log_view_height = layout[2].height.saturating_sub(2).max(1) as usize;
    let max_scroll = self.logs.len().saturating_sub(log_view_height);
    let scroll = self.log_scroll.min(max_scroll);
    let start = self
      .logs
      .len()
      .saturating_sub(log_view_height.saturating_add(scroll));
    let lines: Vec<Line> = self.logs[start..]
      .iter()
      .map(|entry| entry.to_line())
      .collect();
    let log = Paragraph::new(lines)
      .block(
        Block::default()
          .borders(Borders::ALL)
          .border_style(if self.focus == Focus::Output {
            Style::default().fg(Color::Yellow)
          } else {
            Style::default()
          })
          .border_type(BorderType::Rounded)
          .title("Output"),
      )
      .wrap(ratatui::widgets::Wrap { trim: true });

    f.render_widget(log, layout[2]);

    // Status / prompt line.
    let (status_title, status_text) = if let Some(prompt) = &self.prompt {
      let text = if prompt.needs_cursor() {
        Text::from(prompt.buffer.clone())
      } else {
        Text::from("")
      };
      (Some(prompt.label.clone()), text)
    } else if self.running {
      (None, Text::from("Running..."))
    } else {
      (
        None,
        Text::from(
          " Hook Actions:  [enter] run · [a] add · [e] edit · [d] delete  |  [r] reload · [q] quit  |  [tab] toggle focus",
        ),
      )
    };
    let mut status_block = Block::default()
      .borders(Borders::ALL)
      .border_type(BorderType::Rounded);
    if let Some(title) = status_title {
      status_block = status_block.title(title);
    }
    let status = Paragraph::new(status_text)
      .block(status_block)
      .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(status, layout[3]);

    if let Some(prompt) = self.prompt.as_ref()
      && prompt.needs_cursor() {
        let inner_width = layout[3].width.saturating_sub(2).max(1);
        let inner_height = layout[3].height.saturating_sub(2).max(1);
        let (cx, cy) = prompt.visual_cursor(inner_width);
        let x = layout[3].x + 1 + cx.min(inner_width.saturating_sub(1));
        let y = layout[3].y + 1 + cy.min(inner_height.saturating_sub(1));
        f.set_cursor_position((x, y));
      }
  }
}

impl<'a> InputHandler for DashboardState<'a> {
  fn handle_input(&mut self, code: KeyCode) -> Result<bool, RunnerError> {
    self.handle_prompt_input(code)
  }
}

impl<'a> MouseHandler for DashboardState<'a> {
  fn handle_mouse(&mut self, event: MouseEvent) {
    self.handle_mouse_event(event);
  }
}

impl<'a> DashboardState<'a> {
  pub fn from_cwd(cwd: &'a Path) -> Self {
    Self {
      cwd,
      ..Self::default()
    }
  }

  pub fn from_config(cfg: &'a HookConfig) -> Self {
    let mut hooks: Vec<(String, TaskSpec)> = cfg
      .hooks
      .iter()
      .map(|(name, spec)| (name.clone(), spec.clone()))
      .collect();
    hooks.sort_by(|a, b| a.0.cmp(&b.0));

    Self {
      cwd: cfg.source.as_path().parent().unwrap_or(Path::new(".")),
      hooks,
      index: 0,
      running: false,
      logs: Vec::new(),
      prompt: None,
      focus: Focus::Hooks,
      log_scroll: 0,
      source: cfg.source.as_str().to_string(),
    }
  }
}

impl<'a> DashboardState<'a> {
  pub fn apply_config(&mut self, cfg: &HookConfig) {
    let mut hooks: Vec<(String, TaskSpec)> = cfg
      .hooks
      .iter()
      .map(|(name, spec)| (name.clone(), spec.clone()))
      .collect();
    hooks.sort_by(|a, b| a.0.cmp(&b.0));
    self.hooks = hooks;
    if self.index >= self.hooks.len() && !self.hooks.is_empty() {
      self.index = self.hooks.len() - 1;
    }
    self.source = cfg.source.as_str().to_string();
  }

  pub fn current_hook(&self) -> Option<(&String, &TaskSpec)> {
    self.hooks.get(self.index).map(|(name, spec)| (name, spec))
  }

  pub fn move_selection_up(&mut self) {
    self.index = self.index.saturating_sub(1);
  }

  pub fn move_selection_down(&mut self) {
    if self.index + 1 < self.hooks.len() {
      self.index += 1;
    }
  }

  pub fn push_log(&mut self, level: LogLevel, message: impl Into<String>) {
    self.logs.push(LogEntry {
      level,
      message: message.into(),
      timestamp: chrono::Local::now(),
    });
    if self.log_scroll > 0 {
      self.log_scroll += 1;
    }
    if self.logs.len() > LOG_LIMIT {
      let excess = self.logs.len() - LOG_LIMIT;
      self.logs.drain(0..excess);
      self.normalize_log_scroll();
    }
  }

  pub fn append_output(&mut self, chunks: Vec<OutputChunk>) {
    for chunk in chunks {
      match chunk {
        OutputChunk::Stdout(s) => self.push_log(LogLevel::Stdout, s),
        OutputChunk::Stderr(s) => self.push_log(LogLevel::Stderr, s),
      }
    }
  }

  pub fn select_hook(&mut self, name: &str) {
    if let Some((idx, _)) =
      self.hooks.iter().enumerate().find(|(_, (n, _))| n == name)
    {
      self.index = idx;
    }
  }

  pub fn set_prompt(&mut self, prompt: Prompt) -> HukResult<()> {
    if prompt.needs_cursor() {
      self.show_cursor()?;
    } else {
      self.hide_cursor()?;
    }
    self.prompt = Some(prompt);
    Ok(())
  }

  pub fn clear_prompt(&mut self) -> HukResult<()> {
    self.prompt = None;
    self.hide_cursor()?;
    Ok(())
  }

  pub fn scroll_logs(&mut self, delta: isize) {
    if self.logs.is_empty() {
      self.log_scroll = 0;
      return;
    }
    let max = self.logs.len().saturating_sub(1);
    if delta.is_negative() {
      let amount = delta.wrapping_abs() as usize;
      self.log_scroll = self.log_scroll.saturating_sub(amount);
    } else {
      let amount = delta as usize;
      self.log_scroll = (self.log_scroll + amount).min(max);
    }
  }

  pub fn scroll_to_log_start(&mut self) {
    if self.logs.is_empty() {
      self.log_scroll = 0;
    } else {
      self.log_scroll = self.logs.len().saturating_sub(1);
    }
  }

  pub fn scroll_to_log_end(&mut self) {
    self.log_scroll = 0;
  }

  pub fn normalize_log_scroll(&mut self) {
    if self.logs.is_empty() {
      self.log_scroll = 0;
      return;
    }
    let max = self.logs.len().saturating_sub(1);
    if self.log_scroll > max {
      self.log_scroll = max;
    }
  }

  pub fn status_height(&self, width: u16) -> u16 {
    if let Some(prompt) = &self.prompt {
      let inner_width = width.saturating_sub(2).max(1);
      let height = prompt.visual_height(inner_width);
      height.max(3).min(10)
    } else {
      3
    }
  }

  fn handle_mouse_event(&mut self, event: MouseEvent) {
    match event.kind {
      MouseEventKind::ScrollUp => {
        self.focus = Focus::Output;
        self.scroll_logs(2);
      }
      MouseEventKind::ScrollDown => {
        self.focus = Focus::Output;
        self.scroll_logs(-2);
      }
      _ => {}
    }
  }

  fn handle_prompt_input(
    &mut self,
    code: KeyCode,
  ) -> Result<bool, RunnerError> {
    if self.prompt.is_none() {
      self.hide_cursor()?;
      return Ok(false);
    }

    let mut prompt = self.prompt.take().unwrap();
    if prompt.needs_cursor() {
      self.show_cursor()?;
    } else {
      self.hide_cursor()?;
    }

    use KeyCode::*;
    match prompt.kind.clone() {
      PromptKind::ConfirmRun(name) => match code {
        Char('y') | Enter => {
          if let Err(err) = self.run_hook(&name) {
            self.push_log(LogLevel::Error, format!("{err}"));
          }
        }
        Char('n') | Char('\x04') | Char('\x03') | Esc => {}
        _ => {
          self.set_prompt(prompt)?;
          return Ok(true);
        }
      },
      PromptKind::ConfirmRemove(name) => match code {
        Char('y') | Enter => {
          if let Err(err) = self.remove_hook(&name) {
            self.push_log(LogLevel::Error, format!("{err}"));
          }
        }
        Char('n') | Char('\x04') | Char('\x03') | Esc => {}
        _ => {
          self.set_prompt(prompt)?;
          return Ok(true);
        }
      },
      PromptKind::AddName => match code {
        Enter => {
          let name = prompt.buffer.trim().to_string();
          if name.is_empty() {
            self.push_log(LogLevel::Error, "Hook name cannot be empty.");
            self.set_prompt(prompt)?;
            return Ok(true);
          }
          if ensure_valid_hook_name(&name).is_err() {
            self.push_log(
              LogLevel::Error,
              format!("'{name}' is not a valid Git hook name."),
            );
            self.push_log(
              LogLevel::Info,
              format!(
                "Supported hook names: '{}'",
                crate::constants::GIT_HOOKS.join("', '")
              ),
            );
            self.set_prompt(prompt)?;
            return Ok(true);
          }
          if self.hooks.iter().any(|(n, _)| n == &name) {
            self.push_log(
              LogLevel::Error,
              format!("Hook '{name}' already exists. Use edit to change it."),
            );
            self.set_prompt(prompt)?;
            return Ok(true);
          }
          self.set_prompt(Prompt::add_hook_spec(name))?;
          return Ok(true);
        }
        Esc => {
          self.clear_prompt()?;
        }
        key => {
          return match_common_input!(self, prompt, key);
        }
      },
      PromptKind::AddSpec { hook } => match code {
        Enter => {
          if prompt.buffer.trim().is_empty() {
            self
              .push_log(LogLevel::Error, "Task specification cannot be empty.");
            self.set_prompt(prompt)?;
            return Ok(true);
          }
          if let Err(err) = self.add_hook(&hook, &*prompt.buffer) {
            self.push_log(LogLevel::Error, format!("{err}"));
            self.set_prompt(prompt)?;
          } else {
            self.clear_prompt()?;
          }
        }
        Char('\x04') | Char('\x03') | Esc => {
          self.clear_prompt()?;
          return Ok(true);
        }
        key => {
          return match_common_input!(self, prompt, key);
        }
      },
      PromptKind::Update { hook } => match code {
        Enter => {
          if let Err(err) = self.update_hook(&hook, &*prompt.buffer) {
            self.push_log(LogLevel::Error, format!("{err}"));
            self.set_prompt(prompt)?;
          } else {
            self.clear_prompt()?;
          }
        }
        Char('\x04') | Char('\x03') | Esc => {
          self.clear_prompt()?;
          return Ok(true);
        }
        key => {
          return match_common_input!(self, prompt, key);
        }
      },
    }

    Ok(true)
  }
}

type HukResult<T> = core::result::Result<T, std::io::Error>;

trait CursorVisibility {
  fn show_cursor(&self) -> HukResult<()>;
  fn hide_cursor(&self) -> HukResult<()>;
}

impl CursorVisibility for DashboardState<'_> {
  fn show_cursor(&self) -> HukResult<()> {
    crossterm::execute!(io::stdout(), crossterm::cursor::Show)
  }

  fn hide_cursor(&self) -> HukResult<()> {
    crossterm::execute!(io::stdout(), crossterm::cursor::Hide)
  }
}

#[derive(Clone)]
pub struct Prompt {
  pub kind:     PromptKind,
  pub label:    String,
  pub buffer:   String,
  cursor_index: usize,
}

impl Default for Prompt {
  fn default() -> Self {
    Self {
      kind:         PromptKind::AddName,
      label:        String::new(),
      buffer:       String::new(),
      cursor_index: 0,
    }
  }
}
impl Prompt {
  pub fn confirm_run(name: String) -> Self {
    Self {
      kind: PromptKind::ConfirmRun(name.clone()),
      label: format!("Run hook '{name}'? (y/n)"),
      ..Default::default()
    }
  }

  pub fn confirm_remove(name: String) -> Self {
    Self {
      kind: PromptKind::ConfirmRemove(name.clone()),
      label: format!("Delete hook '{name}'? (y/n)"),
      ..Default::default()
    }
  }

  pub fn add_hook_name() -> Self {
    Self {
      kind: PromptKind::AddName,
      label: "New hook name".into(),
      ..Default::default()
    }
  }

  pub fn add_hook_spec(hook: String) -> Self {
    Self {
      kind: PromptKind::AddSpec { hook: hook.clone() },
      label: format!("Spec for '{hook}'"),
      ..Default::default()
    }
  }

  pub fn update_hook(hook: String, preset: String) -> Self {
    Self {
      kind: PromptKind::Update { hook: hook.clone() },
      label: format!("New spec for '{hook}'"),
      buffer: preset.clone(),
      cursor_index: preset.len(),
      ..Default::default()
    }
  }

  fn needs_cursor(&self) -> bool {
    matches!(
      self.kind,
      PromptKind::AddName
        | PromptKind::AddSpec { .. }
        | PromptKind::Update { .. }
    )
  }
}

#[derive(Clone)]
pub enum PromptKind {
  ConfirmRun(String),
  ConfirmRemove(String),
  AddName,
  AddSpec { hook: String },
  Update { hook: String },
}

trait PromptCursor {
  fn insert_char(&mut self, c: char);
  fn backspace(&mut self);
  fn delete_char(&mut self);
  fn move_left(&mut self);
  fn move_right(&mut self);
  fn move_home(&mut self);
  fn move_end(&mut self);
  fn move_up(&mut self);
  fn move_down(&mut self);
  fn visual_height(&self, width: u16) -> u16;
  fn visual_cursor(&self, width: u16) -> (u16, u16);
}

impl Prompt {
  fn cursor_index(&self) -> usize {
    self.cursor_index.min(self.buffer.len())
  }

  fn set_cursor_index(&mut self, idx: usize) {
    self.cursor_index = idx.min(self.buffer.len());
  }

  fn line_bounds(&self, idx: usize) -> (usize, usize) {
    let idx = idx.min(self.buffer.len());
    let start = self.buffer[..idx].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let end = self.buffer[idx..]
      .find('\n')
      .map(|p| idx + p)
      .unwrap_or_else(|| self.buffer.len());
    (start, end)
  }

  fn column_at(&self, idx: usize) -> usize {
    let idx = idx.min(self.buffer.len());
    let (start, _) = self.line_bounds(idx);
    self.buffer[start..idx].chars().count()
  }

  fn index_for_column(&self, line_start: usize, target_col: usize) -> usize {
    let line_start = line_start.min(self.buffer.len());
    let mut col = 0;
    let mut idx = line_start;
    for (offset, ch) in self.buffer[line_start..].char_indices() {
      if ch == '\n' {
        break;
      }
      if col == target_col {
        idx = line_start + offset;
        return idx;
      }
      col += 1;
      idx = line_start + offset + ch.len_utf8();
    }
    idx
  }
}

impl PromptCursor for Prompt {
  fn insert_char(&mut self, c: char) {
    let idx = self.cursor_index();
    self.buffer.insert(idx, c);
    self.set_cursor_index(idx + c.len_utf8());
  }

  fn backspace(&mut self) {
    let idx = self.cursor_index();
    if idx == 0 {
      return;
    }
    if let Some((prev, ch)) = self
      .buffer
      .char_indices()
      .take_while(|(pos, _)| *pos < idx)
      .last()
    {
      self.buffer.drain(prev..prev + ch.len_utf8());
      self.set_cursor_index(prev);
    } else {
      self.set_cursor_index(0);
    }
  }

  fn delete_char(&mut self) {
    let idx = self.cursor_index();
    if idx >= self.buffer.len() {
      return;
    }
    let slice = &self.buffer[idx..];
    let delete_len = slice
      .char_indices()
      .nth(1)
      .map(|(offset, _)| offset)
      .unwrap_or_else(|| slice.len());
    self.buffer.drain(idx..idx + delete_len);
    self.set_cursor_index(idx);
  }

  fn move_left(&mut self) {
    let idx = self.cursor_index();
    if idx == 0 {
      return;
    }
    if let Some((prev, _)) = self
      .buffer
      .char_indices()
      .take_while(|(pos, _)| *pos < idx)
      .last()
    {
      self.set_cursor_index(prev);
    } else {
      self.set_cursor_index(0);
    }
  }

  fn move_right(&mut self) {
    let idx = self.cursor_index();
    if idx >= self.buffer.len() {
      self.set_cursor_index(self.buffer.len());
      return;
    }
    let slice = &self.buffer[idx..];
    let next = slice
      .char_indices()
      .nth(1)
      .map(|(offset, _)| idx + offset)
      .unwrap_or_else(|| self.buffer.len());
    self.set_cursor_index(next);
  }

  fn move_home(&mut self) {
    let (start, _) = self.line_bounds(self.cursor_index());
    self.set_cursor_index(start);
  }

  fn move_end(&mut self) {
    let (_, end) = self.line_bounds(self.cursor_index());
    self.set_cursor_index(end);
  }

  fn move_up(&mut self) {
    let idx = self.cursor_index();
    if idx == 0 {
      return;
    }
    let (current_start, _) = self.line_bounds(idx);
    if current_start == 0 {
      self.set_cursor_index(0);
      return;
    }
    let target_col = self.column_at(idx);
    let prev_end = current_start - 1;
    let prev_start = self.buffer[..prev_end]
      .rfind('\n')
      .map(|p| p + 1)
      .unwrap_or(0);
    let prev_target = self.index_for_column(prev_start, target_col);
    self.set_cursor_index(prev_target.min(prev_end));
  }

  fn move_down(&mut self) {
    let idx = self.cursor_index();
    let (_current_start, current_end) = self.line_bounds(idx);
    if current_end >= self.buffer.len() {
      self.set_cursor_index(self.buffer.len());
      return;
    }
    let target_col = self.column_at(idx);
    let next_start = current_end + 1;
    let next_end = self.line_bounds(next_start).1;
    let next_target = self.index_for_column(next_start, target_col);
    self.set_cursor_index(next_target.min(next_end));
  }

  fn visual_height(&self, width: u16) -> u16 {
    let buffer_lines = wrap_text_lines(&self.buffer, width);
    buffer_lines.len() as u16
  }

  fn visual_cursor(&self, width: u16) -> (u16, u16) {
    let usable_width = width.max(1) as usize;
    let mut line = 0usize;
    let mut col = 0usize;
    let target = self.cursor_index();
    for (idx, ch) in self.buffer.char_indices() {
      if idx >= target {
        break;
      }
      if ch == '\n' {
        line += 1;
        col = 0;
        continue;
      }
      col += 1;
      if col >= usable_width {
        line += 1;
        col = 0;
      }
    }
    (col as u16, line as u16)
  }
}

#[derive(Clone)]
pub struct LogEntry {
  level:     LogLevel,
  message:   String,
  timestamp: chrono::DateTime<chrono::Local>,
}

impl LogEntry {
  fn to_line(&self) -> Line<'_> {
    let (label, color) = match self.level {
      LogLevel::Info => ("info", Color::Cyan),
      LogLevel::Success => ("ok", Color::Green),
      LogLevel::Stdout => ("out", Color::Gray),
      LogLevel::Stderr => ("err", Color::Red),
      LogLevel::Error => ("fail", Color::LightRed),
    };
    let time = self.timestamp.format("%H:%M:%S").to_string();
    Line::from(vec![
      Span::styled(
        format!("{label} "),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
      ),
      Span::styled(format!("[{time}] "), Style::default().fg(Color::DarkGray)),
      Span::raw(&self.message),
    ])
  }
}

#[derive(Clone, Copy)]
pub enum LogLevel {
  Info,
  Success,
  Stdout,
  Stderr,
  Error,
}
