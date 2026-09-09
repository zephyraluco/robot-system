// invalid_config_dialog.rs — Startup dialog for malformed settings.json or AGENTS.md.
//
// Mirrors TS `InvalidConfigDialog` / `InvalidSettingsDialog`:
// - Displayed on startup when config parsing fails.
// - Shows a red-bordered box with the error message.
// - Dismissed by pressing Enter or Escape; user can then fix the file and restart.
// Built on the shared `DialogCore` + `DialogBehavior` base (crate::dialogs::dialog).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use ratatui::Frame;

use crate::dialogs::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::overlays::ModalLayout;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// State for the invalid-config startup dialog.
#[derive(Debug)]
pub struct InvalidConfigDialogState {
    /// Embedded generic dialog base (visibility, geometry, title).
    pub core: DialogCore,
    /// Kind of config error.
    pub kind: InvalidConfigKind,
    /// Human-readable error message (may be multi-line).
    pub error_message: String,
    /// Scroll offset for long error messages.
    pub scroll: u16,
}

/// What kind of configuration is broken.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum InvalidConfigKind {
    #[default]
    Settings,
    ClaudeMd,
    Generic,
}

impl InvalidConfigDialogState {
    pub fn new() -> Self {
        Self {
            core: DialogCore::new("Configuration Error", 80, 24),
            kind: InvalidConfigKind::Settings,
            error_message: String::new(),
            scroll: 0,
        }
    }

    /// Show the dialog with a settings.json error.
    pub fn show_settings_error(error: &str) -> Self {
        let mut state = Self::new();
        state.kind = InvalidConfigKind::Settings;
        state.error_message = error.to_string();
        state.core.set_title("Invalid Settings");
        state.core.open();
        state
    }

    /// Show the dialog with a AGENTS.md parse error.
    pub fn show_claude_md_error(error: &str) -> Self {
        let mut state = Self::new();
        state.kind = InvalidConfigKind::ClaudeMd;
        state.error_message = error.to_string();
        state.core.set_title("Invalid AGENTS.md");
        state.core.open();
        state
    }

    pub fn dismiss(&mut self) {
        self.core.close();
        self.scroll = 0;
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self, max: u16) {
        if self.scroll + 1 < max {
            self.scroll += 1;
        }
    }
}

impl DialogBehavior for InvalidConfigDialogState {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        match key.code {
            KeyCode::Enter => {
                self.dismiss();
                DialogOutcome::Confirmed
            }
            KeyCode::Up => {
                self.scroll_up();
                DialogOutcome::Handled
            }
            KeyCode::Down => {
                self.scroll_down(20);
                DialogOutcome::Handled
            }
            _ => DialogOutcome::Ignored,
        }
    }

    fn render_content(&self, frame: &mut Frame, layout: &ModalLayout) {
        // Build content lines
        let mut lines: Vec<Line<'static>> = Vec::new();

        // Subtitle
        let subtitle = match self.kind {
            InvalidConfigKind::Settings => "~/.claurst/settings.json could not be parsed.",
            InvalidConfigKind::ClaudeMd => "AGENTS.md could not be parsed.",
            InvalidConfigKind::Generic => "A configuration file could not be parsed.",
        };
        lines.push(Line::from(vec![Span::styled(
            subtitle.to_string(),
            Style::default().fg(Color::Yellow),
        )]));
        lines.push(Line::from(""));

        // Error detail
        lines.push(Line::from(vec![Span::styled(
            "Error:".to_string(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]));
        for error_line in self.error_message.lines() {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(error_line.to_string(), Style::default().fg(Color::White)),
            ]));
        }
        lines.push(Line::from(""));

        // Instructions
        lines.push(Line::from(vec![Span::styled(
            "To resolve:".to_string(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )]));
        let instructions = match self.kind {
            InvalidConfigKind::Settings => vec![
                "  1. Open ~/.claurst/settings.json in a text editor.",
                "  2. Fix the JSON syntax error shown above.",
                "  3. Restart Claurst.",
            ],
            InvalidConfigKind::ClaudeMd => vec![
                "  1. Open the AGENTS.md file shown above in a text editor.",
                "  2. Fix the syntax error.",
                "  3. Restart Claurst.",
            ],
            InvalidConfigKind::Generic => vec![
                "  1. Fix the configuration file shown above.",
                "  2. Restart Claurst.",
            ],
        };
        for instr in instructions {
            lines.push(Line::from(vec![Span::styled(
                instr.to_string(),
                Style::default().fg(Color::Gray),
            )]));
        }
        lines.push(Line::from(""));

        // Dismiss hint
        lines.push(Line::from(vec![Span::styled(
            "  Press Enter or Escape to dismiss and continue with defaults.",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )]));

        let total_lines = lines.len() as u16;
        let visible_height = layout.body_area.height;
        let max_scroll = total_lines.saturating_sub(visible_height);
        let scroll = self.scroll.min(max_scroll);

        Paragraph::new(lines)
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false })
            .render(layout.body_area, frame.buffer_mut());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn invalid_config_dialog_state_defaults() {
        let state = InvalidConfigDialogState::new();
        assert!(!state.is_visible());
        assert_eq!(state.kind, InvalidConfigKind::Settings);
        assert!(state.error_message.is_empty());
    }

    #[test]
    fn invalid_config_dialog_show_settings_error() {
        let state = InvalidConfigDialogState::show_settings_error("unexpected token at line 3");
        assert!(state.is_visible());
        assert_eq!(state.kind, InvalidConfigKind::Settings);
        assert!(state.error_message.contains("unexpected token"));
    }

    #[test]
    fn invalid_config_dialog_dismiss() {
        let mut state = InvalidConfigDialogState::show_settings_error("err");
        state.dismiss();
        assert!(!state.is_visible());
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn invalid_config_dialog_renders_without_panic() {
        use crate::dialogs::dialog::DialogBehavior as _;
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        let state = InvalidConfigDialogState::show_settings_error("JSON parse error: unexpected ,");

        terminal.draw(|frame| {
            state.render(frame, frame.area());
        }).unwrap();

        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(content.contains("Invalid Settings") || content.contains("Configuration"));
    }

    #[test]
    fn invalid_config_dialog_shows_error_text() {
        use crate::dialogs::dialog::DialogBehavior as _;
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        let state = InvalidConfigDialogState::show_settings_error("missing field `model`");

        terminal.draw(|frame| {
            state.render(frame, frame.area());
        }).unwrap();

        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(content.contains("missing field") || content.contains("Error"));
    }

    #[test]
    fn invalid_config_dialog_hidden_by_default_renders_nothing() {
        use crate::dialogs::dialog::DialogBehavior as _;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let state = InvalidConfigDialogState::new(); // visible = false
        let snapshot_before = terminal.backend().buffer().clone();

        terminal.draw(|frame| {
            state.render(frame, frame.area());
        }).unwrap();

        // Buffer should be unchanged since dialog is hidden
        let buf = terminal.backend().buffer().clone();
        assert_eq!(buf.content(), snapshot_before.content());
    }
}
