// bypass_permissions_dialog.rs — Startup confirmation dialog for --dangerously-skip-permissions.
//
// Mirrors TS `BypassPermissionsModeDialog.tsx`:
// - Displayed at startup when the session was launched with bypass-permissions mode.
// - Shows a red-bordered warning explaining the risks.
// - User must explicitly accept ("Yes, I accept") or decline ("No, exit").
// - If declined the app exits immediately.
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

/// State for the bypass-permissions startup confirmation dialog.
#[derive(Debug, Clone)]
pub struct BypassPermissionsDialogState {
    /// Embedded generic dialog base (visibility, geometry, title).
    pub core: DialogCore,
    /// 0 = "No, exit" selected; 1 = "Yes, I accept" selected
    pub selected: usize,
}

impl BypassPermissionsDialogState {
    pub fn new() -> Self {
        Self {
            core: DialogCore::new("WARNING: Bypass Permissions Mode", 72, 22),
            selected: 0,
        }
    }

    /// Show the dialog (called at startup when bypass mode is active).
    pub fn show(&mut self) {
        self.core.open();
        self.selected = 0;
    }

    /// Move selection up (wraps).
    pub fn select_prev(&mut self) {
        self.selected = if self.selected == 0 { 1 } else { 0 };
    }

    /// Move selection down (wraps).
    pub fn select_next(&mut self) {
        self.selected = if self.selected == 1 { 0 } else { 1 };
    }

    /// Returns `true` if the currently-selected option is "Yes, I accept".
    pub fn is_accept_selected(&self) -> bool {
        self.selected == 1
    }

    pub fn dismiss(&mut self) {
        self.core.close();
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }
}

impl DialogBehavior for BypassPermissionsDialogState {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        match key.code {
            // "No, exit" — quit immediately (caller sets should_exit on Cancelled).
            KeyCode::Char('1') => {
                self.core.close();
                DialogOutcome::Cancelled
            }
            // "Yes, I accept" — dismiss and continue (caller persists the choice).
            KeyCode::Char('2') => {
                self.core.close();
                DialogOutcome::Confirmed
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.select_prev();
                DialogOutcome::Handled
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.select_next();
                DialogOutcome::Handled
            }
            KeyCode::Enter => {
                self.core.close();
                if self.is_accept_selected() {
                    DialogOutcome::Confirmed
                } else {
                    DialogOutcome::Cancelled
                }
            }
            _ => DialogOutcome::Ignored,
        }
    }

    fn render_content(&self, frame: &mut Frame, layout: &ModalLayout) {
        let mut lines: Vec<Line<'static>> = Vec::new();

        // Body text (matches TS dialog copy)
        lines.push(Line::from(vec![Span::styled(
            "Claurst running in Bypass Permissions mode",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "In Bypass Permissions mode, Claurst will NOT ask for your",
            Style::default().fg(Color::White),
        )]));
        lines.push(Line::from(vec![Span::styled(
            "approval before running potentially dangerous commands.",
            Style::default().fg(Color::White),
        )]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "This mode should only be used in a sandboxed container or VM",
            Style::default().fg(Color::DarkGray),
        )]));
        lines.push(Line::from(vec![Span::styled(
            "that has restricted internet access and can easily be restored",
            Style::default().fg(Color::DarkGray),
        )]));
        lines.push(Line::from(vec![Span::styled(
            "if damaged.",
            Style::default().fg(Color::DarkGray),
        )]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "By proceeding, you accept all responsibility for actions taken",
            Style::default().fg(Color::DarkGray),
        )]));
        lines.push(Line::from(vec![Span::styled(
            "while running in Bypass Permissions mode.",
            Style::default().fg(Color::DarkGray),
        )]));
        lines.push(Line::from(""));
        lines.push(Line::from(""));

        // Options
        let opt_no_style = if self.selected == 0 {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(Color::White)
        };
        let opt_yes_style = if self.selected == 1 {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(Color::Red)
        };

        lines.push(Line::from(vec![
            Span::styled("  [1] ", Style::default().fg(Color::DarkGray)),
            Span::styled("No, exit", opt_no_style),
            Span::raw("        "),
            Span::styled("  [2] ", Style::default().fg(Color::DarkGray)),
            Span::styled("Yes, I accept", opt_yes_style),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "  ↑↓ or 1/2 to select  ·  Enter to confirm",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )]));

        Paragraph::new(lines)
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
    fn bypass_dialog_defaults_hidden() {
        let state = BypassPermissionsDialogState::new();
        assert!(!state.is_visible());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn bypass_dialog_show_sets_visible() {
        let mut state = BypassPermissionsDialogState::new();
        state.show();
        assert!(state.is_visible());
        assert_eq!(state.selected, 0); // "No, exit" selected by default
    }

    #[test]
    fn bypass_dialog_navigate() {
        let mut state = BypassPermissionsDialogState::new();
        state.show();
        assert!(!state.is_accept_selected());
        state.select_next();
        assert!(state.is_accept_selected());
        state.select_prev();
        assert!(!state.is_accept_selected());
    }

    #[test]
    fn bypass_dialog_navigate_wraps() {
        let mut state = BypassPermissionsDialogState::new();
        state.show();
        state.select_prev(); // wrap from 0 → 1
        assert_eq!(state.selected, 1);
        state.select_next(); // wrap from 1 → 0
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn bypass_dialog_dismiss() {
        let mut state = BypassPermissionsDialogState::new();
        state.show();
        state.dismiss();
        assert!(!state.is_visible());
    }

    #[test]
    fn bypass_dialog_renders_without_panic() {
        use crate::dialogs::dialog::DialogBehavior as _;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut state = BypassPermissionsDialogState::new();
        state.show();
        terminal.draw(|frame| {
            state.render(frame, frame.area());
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("WARNING") || content.contains("Bypass"));
    }

    #[test]
    fn bypass_dialog_shows_both_options() {
        use crate::dialogs::dialog::DialogBehavior as _;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut state = BypassPermissionsDialogState::new();
        state.show();
        terminal.draw(|frame| {
            state.render(frame, frame.area());
        }).unwrap();
        let content: String = terminal.backend().buffer().clone().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("No") || content.contains("exit"));
        assert!(content.contains("accept") || content.contains("Yes"));
    }

    #[test]
    fn bypass_dialog_hidden_renders_nothing() {
        use crate::dialogs::dialog::DialogBehavior as _;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let state = BypassPermissionsDialogState::new(); // visible = false
        let before = terminal.backend().buffer().clone();
        terminal.draw(|frame| {
            state.render(frame, frame.area());
        }).unwrap();
        assert_eq!(terminal.backend().buffer().content(), before.content());
    }
}
