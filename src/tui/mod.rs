//! Terminal user interface for `huk`.
//!
//! Provides an interactive dashboard for browsing hooks, running them with
//! confirmation and captured output, and editing hook definitions without
//! leaving the terminal.

#![allow(dead_code)]

use std::io::Stdout;
use std::io::Write;
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
use ratatui::layout::Rect;
use ratatui::prelude::Stylize;
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
use ratatui::widgets::ListState;
use ratatui::widgets::Padding;
use ratatui::widgets::Paragraph;
use serde_json::Map;
use serde_json::Value;

use crate::cli::DashboardOpts;
use crate::config::*;
use crate::constants::VERSION;
use crate::runner::OutputChunk;
use crate::runner::RunnerError;
use crate::runner::TaskRunner;
use crate::runner::mutate_hooks;
use crate::task::TaskSpec;

pub(crate) mod constants;

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

pub trait Drawable {
  fn draw(&mut self, f: &mut ratatui::Frame<'_>);
}

pub trait InputHandler {
  fn handle_input(&mut self, code: KeyCode) -> Result<bool, RunnerError>;
}

pub trait MouseHandler {
  fn handle_mouse(&mut self, event: MouseEvent);
}

pub trait Runnable<'a, W: Write = Stdout> {
  fn run(
    &mut self,
    terminal: &mut Terminal<CrosstermBackend<W>>,
    cwd: &'a Path,
  ) -> Result<(), RunnerError>;
}

pub trait HookManager<'a>
where
  Self: Sized + 'a,
{
  fn selected_hook(&'a self) -> Option<(CowStr<'a>, &'a TaskSpec)>;

  fn cwd(&self) -> &Path;

  fn add_hook(&mut self, name: &str, spec: TaskSpec)
  -> Result<(), RunnerError>;

  fn refresh_config(&mut self) -> Result<(), RunnerError>;

  fn remove_hook(&mut self, name: &str) -> Result<(), RunnerError>;

  fn run_hook(&mut self, name: &str) -> Result<(), RunnerError> {
    self.run_hook_with(name, &[])
  }

  fn run_hook_with(
    &mut self,
    name: &str,
    args: &[String],
  ) -> Result<(), RunnerError>;

  fn update_hook(
    &mut self,
    name: &str,
    spec: TaskSpec,
  ) -> Result<(), RunnerError>;

  fn discover_config(&mut self) -> Result<HookConfig, RunnerError> {
    HookConfig::discover(self.cwd()).map_err(Into::into)
  }

  fn mutate_hooks<F>(&mut self, mutator: F) -> Result<(), RunnerError>
  where
    F: FnOnce(&mut Map<String, Value>) -> Result<(), RunnerError>,
  {
    let cfg = self.discover_config()?;
    mutate_hooks(&cfg, mutator)?;
    self.refresh_config()
  }
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
  spec.to_json_pretty()
}

fn editable_spec(spec: &TaskSpec) -> String {
  match spec {
    TaskSpec::Single(s) => s.to_string(),
    _ => serde_json::to_string(&spec.to_json_value())
      .unwrap_or_else(|_| spec.to_json()),
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
  #[default]
  Hooks,
  Tasks,
  Output,
  Input,
}

impl Focus {
  fn next(self) -> Self {
    use Focus::*;
    match self {
      Hooks => Tasks,
      Tasks => Output,
      Output => Input,
      Input => Hooks,
    }
  }

  fn prev(self) -> Self {
    use Focus::*;
    match self {
      Hooks => Input,
      Tasks => Hooks,
      Output => Tasks,
      Input => Output,
    }
  }
}

#[derive(Clone, Copy)]
pub enum SpecEditMode {
  Add,
  Update,
}

fn task_source_label(cfg: &HookConfig) -> String {
  match (cfg.deno_tasks.len(), cfg.node_scripts.len()) {
    (0, 0) | (_, 0) => "Tasks".to_string(),
    (0, _) => "Scripts".to_string(),
    (_, _) => "Tasks & Scripts".to_string(),
  }
}

fn split_list_detail(area: Rect) -> (Rect, Rect) {
  let min_total =
    constants::MIN_LIST_HEIGHT.saturating_add(constants::MIN_DETAIL_HEIGHT);
  let mut list_height = ((area.height as f32) * 0.35).round() as u16;

  if area.height <= min_total {
    list_height = area.height / 2;
  } else {
    list_height = list_height
      .max(constants::MIN_LIST_HEIGHT)
      .min(area.height.saturating_sub(constants::MIN_DETAIL_HEIGHT));
  }

  let detail_height = area.height.saturating_sub(list_height);
  let list_rect = Rect {
    x:      area.x,
    y:      area.y,
    width:  area.width,
    height: list_height,
  };
  let detail_rect = Rect {
    x:      area.x,
    y:      area.y.saturating_add(list_height),
    width:  area.width,
    height: detail_height,
  };
  (list_rect, detail_rect)
}

fn collect_available_hooks(current: &[(String, TaskSpec)]) -> Vec<String> {
  let mut hooks: Vec<String> = crate::constants::GIT_HOOKS
    .iter()
    .filter(|name| !current.iter().any(|(h, _)| h == *name))
    .map(|name| (*name).to_string())
    .collect();
  hooks.sort();
  hooks
}

fn selected_tasks_from_spec(
  spec: &TaskSpec,
  tasks: &[(String, TaskSpec)],
) -> Vec<usize> {
  let mut selected = Vec::new();
  match spec {
    TaskSpec::Single(name) => {
      if let Some((idx, _)) = tasks
        .iter()
        .enumerate()
        .find(|(_, (n, _))| n == name.as_ref())
      {
        selected.push(idx);
      }
    }
    TaskSpec::Sequence(list) => {
      for item in list {
        if let TaskSpec::Single(name) = item
          && let Some((idx, _)) = tasks
            .iter()
            .enumerate()
            .find(|(_, (n, _))| n == name.as_ref())
          && !selected.contains(&idx)
        {
          selected.push(idx);
        }
      }
    }
    TaskSpec::Detailed { .. } => {}
  }
  selected.sort_unstable();
  selected
}

fn task_spec_from_selection(
  selections: &[usize],
  tasks: &[(String, TaskSpec)],
) -> Option<TaskSpec> {
  let mut items: Vec<TaskSpec> = selections
    .iter()
    .filter_map(|idx| tasks.get(*idx))
    .map(|(name, _)| TaskSpec::Single(CowStr::from(name.clone())))
    .collect();

  if items.is_empty() {
    return None;
  }
  if items.len() == 1 {
    return Some(items.remove(0));
  }
  Some(TaskSpec::Sequence(items))
}

/// Internal state for the dashboard.
#[derive(Clone, Constructor)]
pub struct DashboardState<'a> {
  pub cwd:         &'a Path,
  pub running:     bool,
  pub hooks:       Vec<(String, TaskSpec)>,
  pub index:       usize,
  pub tasks:       Vec<(String, TaskSpec)>,
  pub task:        usize,
  pub hook_state:  ListState,
  pub task_state:  ListState,
  pub logs:        Vec<LogEntry>,
  pub prompt:      Option<Prompt>,
  pub focus:       Focus,
  pub scroll:      usize,
  pub source:      String,
  pub task_label:  String,
  pub task_source: String,
}

impl<'a> Default for DashboardState<'a> {
  fn default() -> Self {
    Self {
      cwd:         Path::new("."),
      running:     false,
      hooks:       vec![],
      index:       0,
      tasks:       vec![],
      task:        0,
      hook_state:  ListState::default(),
      task_state:  ListState::default(),
      logs:        vec![],
      prompt:      None,
      focus:       Focus::Hooks,
      scroll:      0,
      source:      String::new(),
      task_label:  String::new(),
      task_source: String::new(),
    }
  }
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

  fn add_hook(
    &mut self,
    hook: &str,
    spec: TaskSpec,
  ) -> Result<(), RunnerError> {
    ensure_valid_hook_name(hook)?;
    self.mutate_hooks(|hooks| {
      hooks.insert(hook.to_string(), spec.to_json_value());
      Ok(())
    })?;
    self.select_hook(hook);
    self.push_log(LogLevel::Success, format!("Added hook '{hook}'."));

    Ok(())
  }

  fn remove_hook(&mut self, hook: &str) -> Result<(), RunnerError> {
    self.mutate_hooks(|hooks| {
      hooks.remove(hook);
      Ok(())
    })?;

    self.push_log(LogLevel::Success, format!("Removed hook '{hook}'."));

    Ok(())
  }

  fn update_hook(
    &mut self,
    hook: &str,
    spec: TaskSpec,
  ) -> Result<(), RunnerError> {
    let cfg = HookConfig::discover(self.cwd)?;

    mutate_hooks(&cfg, |hooks| {
      hooks.insert(hook.to_string(), spec.to_json_value());
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

  fn run_hook_with(
    &mut self,
    hook: &str,
    extra_args: &[String],
  ) -> Result<(), RunnerError> {
    let cfg = HookConfig::discover(self.cwd)?;
    let Some(spec) = cfg.hooks.get(hook) else {
      self.push_log(LogLevel::Error, format!("Hook '{hook}' not found."));
      return Ok(());
    };
    self.apply_config(&cfg);
    self.select_hook(hook);

    let mut runner = TaskRunner::new_with_capture(&cfg);

    self.running = true;
    self.push_log(LogLevel::Info, format!("Running hook '{hook}'..."));
    let result = runner.run_spec(spec, hook, extra_args);
    self.running = false;

    let output = runner.take_output();
    self.append_output(output);

    if let Err(err) = result {
      self.push_log(LogLevel::Error, format!("{err}"));
    } else {
      self.push_log(LogLevel::Success, format!("Hook '{hook}' finished."));
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
            use Focus::*;
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
                  } else if self.handle_input(Enter)? {
                    continue;
                  }
                }
              }
              Tab => self.focus = self.focus.next(),
              BackTab => self.focus = self.focus.prev(),
              Up => match self.focus {
                Hooks => self.move_selection_up(),
                Tasks => self.task = self.task.saturating_sub(1),
                Output => self.scroll_logs(1),
                Input => self.prompt.iter_mut().for_each(|p| p.move_up()),
              },
              Down => match self.focus {
                Hooks => self.move_selection_down(),
                Tasks => {
                  if self.task < self.tasks.len().saturating_sub(1) {
                    self.task += 1;
                  }
                }
                Output => self.scroll_logs(-1),
                Input => self.prompt.iter_mut().for_each(|p| p.move_down()),
              },
              Home => match self.focus {
                Hooks => self.index = 0,
                Tasks => self.task = 0,
                Output => self.scroll_to_log_start(),
                Input => self.prompt.iter_mut().for_each(|p| p.move_home()),
              },
              End => match self.focus {
                Hooks => self.index = self.hooks.len().saturating_sub(1),
                Tasks => self.task = self.tasks.len().saturating_sub(1),
                Output => self.scroll_to_log_end(),
                Input => self.prompt.iter_mut().for_each(|p| p.move_end()),
              },
              PageUp => match self.focus {
                Hooks => {
                  for _ in 0..3 {
                    self.move_selection_up();
                  }
                }
                Tasks => {
                  for _ in 0..3 {
                    self.task = self.task.saturating_sub(1);
                  }
                }
                Output => self.scroll_logs(self.status_height(0) as isize),
                Input => self.prompt.iter_mut().for_each(|p| p.move_up()),
              },
              PageDown => match self.focus {
                Hooks => {
                  for _ in 0..3 {
                    self.move_selection_down();
                  }
                }
                Tasks => {
                  for _ in 0..3 {
                    if self.task < self.tasks.len().saturating_sub(1) {
                      self.task += 1;
                    }
                  }
                }
                Output => self.scroll_logs(-5),
                Input => self.prompt.iter_mut().for_each(|p| p.move_down()),
              },
              Char('r') | Char('R') | F(5) => {
                self.refresh_config()?;
              }
              Enter => match self.focus {
                Hooks => {
                  if let Some((name, _)) = self.current_hook() {
                    let prompt = Prompt::confirm_run(name.to_string());
                    self.set_prompt(prompt)?;
                  }
                }
                Tasks => {
                  if let Some((name, _)) = self.current_task() {
                    let prompt = Prompt::confirm_run_task(name.to_string());
                    self.set_prompt(prompt)?;
                  }
                }
                _ => {}
              },
              Char('a') => {
                let options = collect_available_hooks(&self.hooks);
                if options.is_empty() {
                  self.push_log(
                    LogLevel::Error,
                    "All supported hooks are already configured.",
                  );
                } else {
                  self.set_prompt(Prompt::pick_hook(options))?;
                }
              }
              Char('e') => {
                let current = self
                  .current_hook()
                  .map(|(name, spec)| (name.clone(), spec.clone()));
                if let Some((name, spec)) = current {
                  if self.tasks.is_empty() {
                    self.set_prompt(Prompt::update_hook(
                      name,
                      editable_spec(&spec),
                    ))?;
                  } else {
                    let selected = selected_tasks_from_spec(&spec, &self.tasks);
                    let next_task =
                      selected.first().copied().unwrap_or(self.task);
                    self.task = next_task;
                    self.set_prompt(Prompt::pick_task(
                      name,
                      SpecEditMode::Update,
                      editable_spec(&spec),
                      selected,
                    ))?;
                  }
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
            self.normalize_scroll();
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
        Constraint::Fill(2),
        Constraint::Min(if f.area().height > 24 { 2 } else { 1 }),
        Constraint::Min(status_height),
      ])
      .split(f.area());

    let title = format!(" {} — {} hooks", self.source, self.hooks.len());
    let header = Paragraph::new(Text::from(title))
      .style(Style::default().add_modifier(Modifier::BOLD))
      .block(
        Block::default()
          .borders(Borders::BOTTOM)
          .border_type(BorderType::Rounded)
          .title(" huk dashboard ")
          .title(
            Line::from(format!(" v{VERSION} "))
              .right_aligned()
              .style(Style::default().dim()),
          ),
      );
    f.render_widget(header, layout[0]);

    // Main area: list + details (responsive columns).
    let main_area = layout[1];
    let use_columns = main_area.width >= constants::MIN_TWO_COLUMN_WIDTH;
    let constraints = if use_columns {
      [Constraint::Percentage(50), Constraint::Percentage(50)]
    } else {
      [
        Constraint::Min(constants::MIN_PANEL_HEIGHT),
        Constraint::Min(constants::MIN_PANEL_HEIGHT),
      ]
    };
    let main = Layout::default()
      .direction(if use_columns {
        Direction::Horizontal
      } else {
        Direction::Vertical
      })
      .constraints(constraints)
      .split(main_area);

    let hooks_area = main[0];
    let tasks_area = main[1];
    let (hooks_list_area, hooks_detail_area) = split_list_detail(hooks_area);

    let (hook_items, hook_selected, hook_title, hook_detail_title, hook_text) =
      if let Some(PromptKind::PickHook { options, index }) =
        self.prompt.as_ref().map(|p| &p.kind)
      {
        let items: Vec<ListItem> = options
          .iter()
          .enumerate()
          .map(|(i, name)| {
            let marker = if i == *index { "›" } else { " " };
            let style = if i == *index {
              Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
            } else {
              Style::default()
            };
            ListItem::new(Span::styled(format!("{marker} {name}"), style))
          })
          .collect();
        (
          items,
          Some(*index),
          " Add Hook ".to_string(),
          " Hook Selection ".to_string(),
          "Pick a hook name to add.".to_string(),
        )
      } else {
        let items: Vec<ListItem> = self
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
        let detail_text = if let Some((_, spec)) = self.current_hook() {
          format_spec(spec)
        } else {
          "No hooks configured.".into()
        };
        let detail_title = if let Some((s, _)) = self.current_hook() {
          format!(" {s} ")
        } else {
          " Task Specification ".to_string()
        };
        let selected = if self.hooks.is_empty() {
          None
        } else {
          Some(self.index)
        };
        (
          items,
          selected,
          " Hooks (↑↓ to move, Enter to run) ".to_string(),
          detail_title,
          detail_text,
        )
      };

    let list = List::new(hook_items).block(
      Block::default()
        .borders(Borders::ALL)
        .border_style(if self.focus == Focus::Hooks {
          Style::default().fg(Color::Yellow)
        } else {
          Style::default()
        })
        .border_type(if self.focus == Focus::Hooks {
          BorderType::Double
        } else {
          BorderType::Rounded
        })
        .padding(Padding::uniform(1))
        .title(hook_title),
    );
    if hook_selected.is_some() {
      self.hook_state.select(hook_selected);
    } else {
      self.hook_state.select(None);
    }
    f.render_stateful_widget(list, hooks_list_area, &mut self.hook_state);

    let detail = Paragraph::new(hook_text)
      .block(
        Block::default()
          .borders(Borders::ALL)
          .border_type(if self.focus == Focus::Hooks {
            BorderType::Thick
          } else {
            BorderType::Rounded
          })
          .border_style(if self.focus == Focus::Hooks {
            Style::default().fg(Color::Yellow)
          } else {
            Style::default()
          })
          .padding(Padding::uniform(1))
          .title(hook_detail_title),
      )
      .wrap(ratatui::widgets::Wrap { trim: true });
    f.render_widget(detail, hooks_detail_area);

    // Tasks + details.
    let mut task_picker_selection: Vec<usize> = Vec::new();
    let is_task_picker =
      if let Some(PromptKind::PickTask { selections, .. }) =
        self.prompt.as_ref().map(|p| &p.kind)
      {
        task_picker_selection = selections.clone();
        true
      } else {
        false
      };
    let task_items: Vec<ListItem> = self
      .tasks
      .iter()
      .enumerate()
      .map(|(i, (name, _))| {
        let marker = if i == self.task { "›" } else { " " };
        let selected = task_picker_selection.contains(&i);
        let badge = if is_task_picker {
          if selected { "[x]" } else { "[ ]" }
        } else {
          ""
        };
        let style = if i == self.task {
          Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
        } else {
          Style::default()
        };
        let content = if badge.is_empty() {
          format!("{marker} {name}")
        } else {
          format!("{marker} {badge} {name}")
        };
        ListItem::new(Span::styled(content, style))
      })
      .collect();
    let task_list = List::new(task_items).block(
      Block::default()
        .borders(Borders::ALL)
        .border_style(if self.focus == Focus::Tasks || is_task_picker {
          Style::default().fg(Color::Yellow)
        } else {
          Style::default().dim()
        })
        .border_type(if self.focus == Focus::Tasks || is_task_picker {
          BorderType::Double
        } else {
          BorderType::Rounded
        })
        .padding(Padding::uniform(1))
        .title(format!(" {} ({}) ", self.task_label, self.task_source)),
    );
    let (tasks_list_area, tasks_detail_area) = split_list_detail(tasks_area);
    if self.tasks.is_empty() {
      self.task_state.select(None);
    } else {
      self.task_state.select(Some(self.task));
    }
    f.render_stateful_widget(task_list, tasks_list_area, &mut self.task_state);

    let task_spec_text = if let Some((_, spec)) = self.current_task() {
      format_spec(spec)
    } else {
      "No tasks configured.".into()
    };
    let task_detail = Paragraph::new(task_spec_text)
      .block(
        Block::default()
          .borders(Borders::ALL)
          .border_type(if self.focus == Focus::Tasks || is_task_picker {
            BorderType::Thick
          } else {
            BorderType::Rounded
          })
          .border_style(if self.focus == Focus::Tasks || is_task_picker {
            Style::default().fg(Color::Yellow)
          } else {
            Style::default()
          })
          .padding(Padding::uniform(1))
          .title(format!(
            " {name} ",
            name = if let Some((name, _)) = self.current_task() {
              name.to_string()
            } else {
              "Task Specification".into()
            }
          )),
      )
      .wrap(ratatui::widgets::Wrap { trim: true });
    f.render_widget(task_detail, tasks_detail_area);

    // Log panel.
    let log_view_height = layout[2].height.saturating_sub(2).max(1) as usize;
    let max_scroll = self.logs.len().saturating_sub(log_view_height);
    let scroll = self.scroll.min(max_scroll);
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
          .title(" Output "),
      )
      .wrap(ratatui::widgets::Wrap { trim: false });

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
          " [enter] run · [a] add · [e] edit · [d] delete · [F2] manual spec  |  [r] reload · [q] quit  |  [tab] toggle focus",
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
      && prompt.needs_cursor()
    {
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
      PromptKind::ConfirmRunTask(name) => match code {
        Char('y') | Enter => {
          if let Err(err) = self.run_task(&name) {
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
      PromptKind::PickHook { options, mut index } => match code {
        Up => {
          index = index.saturating_sub(1);
          self.set_prompt(Prompt {
            kind: PromptKind::PickHook { options, index },
            ..prompt
          })?;
          return Ok(true);
        }
        Down => {
          if index + 1 < options.len() {
            index += 1;
          }
          self.set_prompt(Prompt {
            kind: PromptKind::PickHook { options, index },
            ..prompt
          })?;
          return Ok(true);
        }
        Enter => {
          if let Some(name) = options.get(index).cloned() {
            if self.tasks.is_empty() {
              self.set_prompt(Prompt::add_hook_spec(name))?;
            } else {
              self.task = 0;
              self.set_prompt(Prompt::pick_task(
                name,
                SpecEditMode::Add,
                String::new(),
                Vec::new(),
              ))?;
            }
          } else {
            self.push_log(LogLevel::Error, "No hook selected.");
            self.clear_prompt()?;
          }
          return Ok(true);
        }
        Char('\x04') | Char('\x03') | Esc => {
          self.clear_prompt()?;
          return Ok(true);
        }
        _ => {
          self.set_prompt(prompt)?;
          return Ok(true);
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
          match parse_spec_input(&prompt.buffer) {
            Ok(spec) => {
              if let Err(err) = self.add_hook(&hook, spec) {
                self.push_log(LogLevel::Error, format!("{err}"));
                self.set_prompt(prompt)?;
              } else {
                self.clear_prompt()?;
              }
            }
            Err(err) => {
              self.push_log(LogLevel::Error, format!("{err}"));
              self.set_prompt(prompt)?;
            }
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
          if prompt.buffer.trim().is_empty() {
            self
              .push_log(LogLevel::Error, "Task specification cannot be empty.");
            self.set_prompt(prompt)?;
            return Ok(true);
          }
          match parse_spec_input(&prompt.buffer) {
            Ok(spec) => {
              if let Err(err) = self.update_hook(&hook, spec) {
                self.push_log(LogLevel::Error, format!("{err}"));
                self.set_prompt(prompt)?;
              } else {
                self.clear_prompt()?;
              }
            }
            Err(err) => {
              self.push_log(LogLevel::Error, format!("{err}"));
              self.set_prompt(prompt)?;
            }
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
      PromptKind::PickTask {
        hook,
        mode,
        fallback,
        mut selections,
        mut index,
      } => match code {
        Up => {
          index = index.saturating_sub(1);
          self.task = index;
          self.set_prompt(Prompt {
            kind: PromptKind::PickTask {
              hook,
              mode,
              fallback,
              selections,
              index,
            },
            ..prompt
          })?;
          return Ok(true);
        }
        Down => {
          if index + 1 < self.tasks.len() {
            index += 1;
          }
          self.task = index;
          self.set_prompt(Prompt {
            kind: PromptKind::PickTask {
              hook,
              mode,
              fallback,
              selections,
              index,
            },
            ..prompt
          })?;
          return Ok(true);
        }
        Char(' ') => {
          if self.tasks.is_empty() {
            self.set_prompt(prompt)?;
            return Ok(true);
          }
          if selections.contains(&index) {
            selections.retain(|idx| *idx != index);
          } else {
            selections.push(index);
          }
          selections.sort_unstable();
          self.set_prompt(Prompt {
            kind: PromptKind::PickTask {
              hook,
              mode,
              fallback,
              selections,
              index,
            },
            ..prompt
          })?;
          return Ok(true);
        }
        F(2) => {
          match mode {
            SpecEditMode::Add => {
              self.set_prompt(Prompt::add_hook_spec_with_buffer(
                hook, fallback,
              ))?;
            }
            SpecEditMode::Update => {
              self.set_prompt(Prompt::update_hook(hook, fallback))?;
            }
          }
          return Ok(true);
        }
        Enter => {
          if selections.is_empty() {
            self.push_log(LogLevel::Error, "Select at least one task.");
            self.set_prompt(Prompt {
              kind: PromptKind::PickTask {
                hook,
                mode,
                fallback,
                selections,
                index,
              },
              ..prompt
            })?;
            return Ok(true);
          }
          if let Some(spec) = task_spec_from_selection(&selections, &self.tasks)
          {
            let result = match mode {
              SpecEditMode::Add => self.add_hook(&hook, spec),
              SpecEditMode::Update => self.update_hook(&hook, spec),
            };
            if let Err(err) = result {
              self.push_log(LogLevel::Error, format!("{err}"));
              self.set_prompt(Prompt::pick_task(
                hook, mode, fallback, selections,
              ))?;
              return Ok(true);
            }
          }
          self.clear_prompt()?;
          return Ok(true);
        }
        Char('\x04') | Char('\x03') | Esc => {
          self.clear_prompt()?;
          return Ok(true);
        }
        _ => {
          self.set_prompt(prompt)?;
          return Ok(true);
        }
      },
    }

    Ok(true)
  }
}

impl<'a> MouseHandler for DashboardState<'a> {
  fn handle_mouse(&mut self, event: MouseEvent) {
    let mut delta = match event.kind {
      MouseEventKind::ScrollUp => 1,
      MouseEventKind::ScrollDown => -1,
      _ => 0,
    };
    use KeyModifiers as KM;
    if event.modifiers.contains(KM::ALT) || event.modifiers.contains(KM::META) {
      delta *= constants::FAST_SCROLL_MULTIPLIER as isize;
    } else {
      delta *= constants::BASE_SCROLL_DELTA as isize;
    }
    self.scroll(delta);
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

    let cwd = cfg.source.as_path().parent().unwrap_or(Path::new("."));
    let tasks = cfg
      .tasks()
      .into_iter()
      .map(|(name, spec)| (name.to_string(), spec))
      .collect();
    Self {
      cwd,
      hooks,
      index: 0,
      tasks,
      task: 0,
      hook_state: ListState::default(),
      task_state: ListState::default(),
      running: false,
      logs: Vec::new(),
      prompt: None,
      focus: Focus::Hooks,
      scroll: 0,
      source: cfg.source.as_str().to_string(),
      task_label: task_source_label(cfg),
      task_source: cfg.source.file_name().to_string(),
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
    self.tasks = cfg
      .tasks()
      .into_iter()
      .map(|(name, spec)| (name.to_string(), spec))
      .collect();
    if self.task >= self.tasks.len() && !self.tasks.is_empty() {
      self.task = self.tasks.len() - 1;
    }
    self.source = cfg.source.as_str().to_string();
    self.task_label = task_source_label(cfg);
    self.task_source = cfg.source.file_name().to_string();
  }

  pub fn current_hook(&self) -> Option<(&String, &TaskSpec)> {
    self.hooks.get(self.index).map(|(name, spec)| (name, spec))
  }

  pub fn current_task(&self) -> Option<(&String, &TaskSpec)> {
    self.tasks.get(self.task).map(|(name, spec)| (name, spec))
  }

  pub fn run_task(&mut self, task: &str) -> Result<(), RunnerError> {
    let cfg = HookConfig::discover(self.cwd)?;
    self.apply_config(&cfg);
    let mut runner = TaskRunner::new_with_capture(&cfg);

    self.running = true;
    self.push_log(LogLevel::Info, format!("Running task '{task}'..."));
    let result = runner.run_named_task(task);
    self.running = false;

    let output = runner.take_output();
    self.append_output(output);

    if let Err(err) = result {
      self.push_log(LogLevel::Error, format!("{err}"));
    } else {
      self.push_log(LogLevel::Success, format!("Task '{task}' finished."));
    }
    Ok(())
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
    if self.scroll > 0 {
      self.scroll += 1;
    }
    if self.logs.len() > constants::LOG_LIMIT {
      let excess = self.logs.len() - constants::LOG_LIMIT;
      self.logs.drain(0..excess);
      self.normalize_scroll();
    }
  }

  pub fn append_output(&mut self, chunks: Vec<OutputChunk>) {
    for chunk in chunks {
      match chunk {
        OutputChunk::Stdout(s) => self.push_log(
          LogLevel::Stdout,
          s.replace("\r\n", "\n").replace('\r', "\n"),
        ),
        OutputChunk::Stderr(s) => self.push_log(
          LogLevel::Stderr,
          s.replace("\r\n", "\n").replace('\r', "\n"),
        ),
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

  /// Scroll the currently focused region by [`delta`] lines.
  pub fn scroll(&mut self, delta: isize) {
    use Focus::*;

    if matches!(self.focus, Hooks | Tasks) {
      let (list, offset) = if self.focus == Hooks {
        (&mut self.hook_state, &mut self.index)
      } else {
        (&mut self.task_state, &mut self.task)
      };

      if delta < 0 {
        list.scroll_up_by(delta.unsigned_abs() as u16);
      } else if delta > 0 {
        list.scroll_down_by(delta.unsigned_abs() as u16);
      }
      *offset = list.offset();
    } else if self.focus == Output {
      self.scroll_logs(delta);
    }
  }

  pub fn scroll_to(&mut self, offset: usize) {
    use Focus::*;

    match self.focus {
      Hooks => {
        *self.hook_state.selected_mut() = Some(offset);
      }
      Tasks => {
        *self.task_state.selected_mut() = Some(offset);
      }
      Output => {
        self.scroll = offset;
      }
      _ => {}
    }
  }

  pub fn scroll_logs(&mut self, delta: isize) {
    if self.logs.is_empty() {
      self.scroll_to_log_end();
      return;
    }
    let max = self.logs.len().saturating_sub(1);
    if delta.is_negative() {
      let amount = delta.wrapping_abs() as usize;
      self.scroll = self.scroll.saturating_sub(amount);
    } else {
      let amount = delta as usize;
      self.scroll = (self.scroll + amount).min(max);
    }
  }

  pub fn scroll_to_log_start(&mut self) {
    if self.logs.is_empty() {
      self.scroll_to_log_end();
    } else {
      self.scroll = self.logs.len().saturating_sub(1);
    }
  }

  pub fn scroll_to_log_end(&mut self) {
    self.scroll = 0;
  }

  pub fn normalize_scroll(&mut self) {
    if self.logs.is_empty() {
      self.scroll_to_log_end();
      return;
    }
    let max = self.logs.len().saturating_sub(1);
    if self.scroll > max {
      self.scroll = max;
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

  pub fn confirm_run_task(name: String) -> Self {
    Self {
      kind: PromptKind::ConfirmRunTask(name.clone()),
      label: format!("Run task '{name}'? (y/n)"),
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
      label: format!("Manual spec for '{hook}'"),
      ..Default::default()
    }
  }

  pub fn add_hook_spec_with_buffer(hook: String, buffer: String) -> Self {
    let cursor_index = buffer.len();
    Self {
      kind: PromptKind::AddSpec { hook: hook.clone() },
      label: format!("Manual spec for '{hook}'"),
      buffer,
      cursor_index,
    }
  }

  pub fn update_hook(hook: String, preset: String) -> Self {
    Self {
      kind: PromptKind::Update { hook: hook.clone() },
      label: format!("Manual spec for '{hook}'"),
      buffer: preset.clone(),
      cursor_index: preset.len(),
      ..Default::default()
    }
  }

  pub fn pick_task(
    hook: String,
    mode: SpecEditMode,
    fallback: String,
    selections: Vec<usize>,
  ) -> Self {
    let index = selections.first().copied().unwrap_or(0);
    Self {
      kind: PromptKind::PickTask {
        hook: hook.clone(),
        mode,
        fallback,
        selections,
        index,
      },
      label: format!(
        "Pick tasks for '{hook}' (↑↓ move, Space toggle, Enter confirm, F2 manual)"
      ),
      ..Default::default()
    }
  }

  pub fn pick_hook(options: Vec<String>) -> Self {
    Self {
      kind: PromptKind::PickHook { options, index: 0 },
      label: "Select hook to add (↑↓ move, Enter confirm)".into(),
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
  ConfirmRunTask(String),
  ConfirmRemove(String),
  AddName,
  AddSpec {
    hook: String,
  },
  Update {
    hook: String,
  },
  PickHook {
    options: Vec<String>,
    index:   usize,
  },
  PickTask {
    hook:       String,
    mode:       SpecEditMode,
    fallback:   String,
    selections: Vec<usize>,
    index:      usize,
  },
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
    let prefix = format!("{label} [{time}] ");
    let mut indent = String::from('\n');
    for _ in 0..prefix.len() {
      indent.push(' ');
    }
    Line::from(vec![
      Span::styled(
        format!("{label} "),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
      ),
      Span::styled(format!("[{time}] "), Style::default().fg(Color::DarkGray)),
      Span::raw(self.message.replace(['\r', '\n'], &indent)),
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
