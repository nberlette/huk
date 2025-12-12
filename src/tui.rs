//! Terminal user interface for `huk`.
//!
//! This module implements a minimal interactive dashboard using [`crossterm`]
//! and [`ratatui`], which lists configured hooks and allows the user to
//! execute a hook's tasks by pressing Enter. Press `q` to exit the dashboard.

#![allow(dead_code)]

use crate::config::HookConfig;
use crate::runner::RunnerError;
use crate::runner::TaskRunner;
use crate::task::TaskSpec;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use crossterm::event::{self};
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::Paragraph;
use std::io::Stdout;
use std::io::{self};

/// Launch the dashboard. Returns an error if the terminal cannot be initialized
/// or if configuration loading fails.
pub fn handle_dashboard() -> Result<(), RunnerError> {
  let cfg = HookConfig::discover(&std::env::current_dir()?)?;

  // Collect hooks in a stable order for display.
  let mut hooks: Vec<(String, TaskSpec)> = cfg
    .hooks
    .iter()
    .map(|(name, spec)| (name.clone(), spec.clone()))
    .collect();

  hooks.sort_by(|a, b| a.0.cmp(&b.0));

  if hooks.is_empty() {
    eprintln!(
      "No hooks found in '{path}' config file.",
      path = cfg.source.as_str()
    );
    return Ok(());
  }

  let mut state = DashboardState::new(hooks);
  enable_raw_mode().map_err(|e| RunnerError::Io(e))?;
  let mut stdout = io::stdout();
  crossterm::execute!(stdout, EnterAlternateScreen)
    .map_err(|e| RunnerError::Io(e))?;

  let backend = CrosstermBackend::new(stdout);
  let mut terminal = Terminal::new(backend).map_err(|e| RunnerError::Io(e))?;

  let result = run_dashboard(&mut terminal, &cfg, &mut state);

  // Restore terminal.
  disable_raw_mode().map_err(|e| RunnerError::Io(e))?;

  crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)
    .map_err(|e| RunnerError::Io(e))?;

  terminal.show_cursor().map_err(|e| RunnerError::Io(e))?;
  result
}

/// Manage the event loop for the dashboard.
pub fn run_dashboard(
  terminal: &mut Terminal<CrosstermBackend<Stdout>>,
  cfg: &HookConfig,
  state: &mut DashboardState,
) -> Result<(), RunnerError> {
  loop {
    terminal
      .draw(|f| {
        // Split into two panels: left for hooks, right for description.
        let chunks = Layout::default()
          .direction(Direction::Horizontal)
          .constraints(&[
            Constraint::Percentage(40),
            Constraint::Percentage(60),
          ])
          .split(f.area());

        // Hook list
        let items: Vec<ListItem> = state
          .hooks
          .iter()
          .enumerate()
          .map(|(i, (name, _))| {
            let content = if i == state.selected {
              Span::styled(
                format!("> {name}"),
                Style::default()
                  .fg(Color::Yellow)
                  .add_modifier(Modifier::BOLD)
                  .add_modifier(Modifier::UNDERLINED)
                  .underline_color(Color::LightYellow),
              )
            } else {
              Span::raw(format!("  {name}"))
            };
            ListItem::new(content)
          })
          .collect();

        let list = List::new(items)
          .block(Block::default().title("Hooks").borders(Borders::ALL));

        f.render_widget(list, chunks[0]);

        // Description panel
        let (_, spec) = &state.hooks[state.selected];
        let desc = format!("{spec:?}");
        let paragraph = Paragraph::new(desc)
          .block(
            Block::default()
              .title("Task Specification")
              .borders(Borders::ALL),
          )
          .wrap(ratatui::widgets::Wrap { trim: true });

        f.render_widget(paragraph, chunks[1]);
      })
      .map_err(|e| RunnerError::Io(e))?;

    // Handle events.
    if event::poll(std::time::Duration::from_millis(200))
      .map_err(|e| RunnerError::Io(e))?
    {
      if let Event::Key(KeyEvent {
        code, modifiers, ..
      }) = event::read().map_err(|e| RunnerError::Io(e))?
      {
        match code {
          KeyCode::Char('q') => break,
          KeyCode::Up => state.selected = state.selected.saturating_sub(1),
          KeyCode::Down => {
            if state.selected + 1 < state.hooks.len() {
              state.selected += 1;
            }
          }
          KeyCode::Enter => {
            // Execute selected hook.
            let (name, spec) = &state.hooks[state.selected];
            let mut runner = TaskRunner::new(cfg);
            runner.run_spec(spec, name, &[])?;
          }
          KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
            break;
          }
          _ => {}
        }
      }
    }
  }
  Ok(())
}

/// Internal state for the dashboard.
pub struct DashboardState {
  pub running:  bool,
  pub hooks:    Vec<(String, TaskSpec)>,
  pub selected: usize,
}

impl DashboardState {
  pub const fn new(hooks: Vec<(String, TaskSpec)>) -> Self {
    Self {
      hooks,
      selected: 0,
      running: false,
    }
  }

  pub fn start(&mut self) -> bool {
    if !self.running {
      self.running = true;
      return true;
    }
    false
  }

  pub fn stop(&mut self) -> bool {
    if self.running {
      self.running = false;
      return true;
    }
    false
  }
}

impl Default for DashboardState {
  fn default() -> Self {
    Self::new(Vec::new())
  }
}
