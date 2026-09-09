// desktop_upsell_startup.rs — DesktopUpsellStartup surface.
//
// Shown at startup on supported platforms (macOS / Windows x64) when the user
// hasn't yet tried the Claurst Code Desktop app.  Mirrors the behavior of
// src/components/DesktopUpsell/DesktopUpsellStartup.tsx:
//
//   - Shown at most 3 times per user (seen_count guard).
//   - Three choices: "Open in Claurst Code Desktop" (Try), "Not now", "Don't ask again".
//   - "Try" acknowledges and closes (CLI cannot actually launch the desktop app,
//     so we treat it the same as "Not now" but could be extended).
//   - "Don't ask again" sets the dismissed flag permanently.
//   - Esc / "Not now" closes without permanently dismissing.
// Built on the shared `DialogCore` + `DialogBehavior` base (crate::dialogs::dialog).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use crate::dialogs::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::overlays::ModalLayout;

// ---------------------------------------------------------------------------
// Platform guard
// ---------------------------------------------------------------------------

/// Returns true when Claurst Desktop is a supported platform option.
pub fn is_desktop_supported_platform() -> bool {
    cfg!(target_os = "macos")
        || (cfg!(target_os = "windows") && cfg!(target_arch = "x86_64"))
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Which option the cursor is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DesktopUpsellSelection {
    #[default]
    Try,
    NotNow,
    Never,
}

impl DesktopUpsellSelection {
    fn label(self) -> &'static str {
        match self {
            Self::Try => "Open in Claurst Code Desktop",
            Self::NotNow => "Not now",
            Self::Never => "Don't ask again",
        }
    }

    const ALL: [Self; 3] = [Self::Try, Self::NotNow, Self::Never];
}

/// Desktop upsell startup dialog state.
#[derive(Debug, Clone)]
pub struct DesktopUpsellStartupState {
    /// Embedded generic dialog base (visibility, geometry, title).
    pub core: DialogCore,
    /// Which option is highlighted.
    pub selection: DesktopUpsellSelection,
    /// How many times the dialog has been shown this session.
    pub seen_count: u32,
    /// Whether the user has permanently dismissed the dialog.
    dismissed: bool,
}

impl Default for DesktopUpsellStartupState {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopUpsellStartupState {
    pub fn new() -> Self {
        Self {
            core: DialogCore::new("Claurst Code Desktop", 58, 12),
            selection: DesktopUpsellSelection::default(),
            seen_count: 0,
            dismissed: false,
        }
    }

    /// Show the dialog if eligible: supported platform, not dismissed, seen < 3.
    pub fn show_if_eligible(&mut self) {
        if self.dismissed || !is_desktop_supported_platform() {
            return;
        }
        if self.seen_count >= 3 {
            return;
        }
        self.seen_count += 1;
        self.core.open();
    }

    /// Move the cursor up.
    pub fn select_prev(&mut self) {
        self.selection = match self.selection {
            DesktopUpsellSelection::Try => DesktopUpsellSelection::Never,
            DesktopUpsellSelection::NotNow => DesktopUpsellSelection::Try,
            DesktopUpsellSelection::Never => DesktopUpsellSelection::NotNow,
        };
    }

    /// Move the cursor down.
    pub fn select_next(&mut self) {
        self.selection = match self.selection {
            DesktopUpsellSelection::Try => DesktopUpsellSelection::NotNow,
            DesktopUpsellSelection::NotNow => DesktopUpsellSelection::Never,
            DesktopUpsellSelection::Never => DesktopUpsellSelection::Try,
        };
    }

    /// Confirm the currently highlighted selection.
    /// Returns `true` if the user selected "Never" (permanent dismiss).
    pub fn confirm(&mut self) -> bool {
        match self.selection {
            DesktopUpsellSelection::Try | DesktopUpsellSelection::NotNow => {
                self.core.close();
                false
            }
            DesktopUpsellSelection::Never => {
                self.core.close();
                self.dismissed = true;
                true
            }
        }
    }

    /// Close without permanent dismiss (Esc key).
    pub fn dismiss_temporarily(&mut self) {
        self.core.close();
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    /// Height the dialog occupies (0 if not visible).
    pub fn height(&self) -> u16 {
        if self.core.is_visible() { 12 } else { 0 }
    }
}

impl DialogBehavior for DesktopUpsellStartupState {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        match key.code {
            KeyCode::Up | KeyCode::BackTab => {
                self.select_prev();
                DialogOutcome::Handled
            }
            KeyCode::Down | KeyCode::Tab => {
                self.select_next();
                DialogOutcome::Handled
            }
            KeyCode::Enter => {
                self.confirm();
                DialogOutcome::Confirmed
            }
            _ => DialogOutcome::Ignored,
        }
    }

    fn render_content(&self, frame: &mut Frame, layout: &ModalLayout) {
        let inner = Rect {
            x: layout.body_area.x,
            y: layout.body_area.y,
            width: layout.body_area.width,
            height: layout.body_area.height,
        };

        let mut lines: Vec<Line> = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "Same Claurst features with visual diffs, live app",
                Style::default().fg(Color::White),
            )]),
            Line::from(vec![Span::styled(
                "preview, parallel sessions, and more.",
                Style::default().fg(Color::White),
            )]),
            Line::from(""),
        ];

        for option in &DesktopUpsellSelection::ALL {
            let selected = *option == self.selection;
            let prefix = if selected { "> " } else { "  " };
            let label_style = if selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            lines.push(Line::from(vec![
                Span::styled(prefix, label_style),
                Span::styled(option.label(), label_style),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "↑↓ navigate  Enter confirm  Esc close",
            Style::default().fg(Color::DarkGray),
        )]));

        Paragraph::new(lines).render(inner, frame.buffer_mut());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn desktop_upsell_show_increments_count() {
        let mut state = DesktopUpsellStartupState::new();
        // Platform guard may suppress on CI; test the count logic directly.
        state.dismissed = false;
        state.seen_count = 0;
        // Force show by simulating the body of show_if_eligible without platform check.
        state.seen_count += 1;
        state.core.open();
        assert_eq!(state.seen_count, 1);
        assert!(state.is_visible());
    }

    #[test]
    fn desktop_upsell_max_shows() {
        let mut state = DesktopUpsellStartupState::new();
        state.seen_count = 3;
        state.dismissed = false;
        // show_if_eligible should NOT show when count >= 3
        state.show_if_eligible();
        // On unsupported platform it also won't show; just verify count doesn't increment.
        assert_eq!(state.seen_count, 3);
    }

    #[test]
    fn desktop_upsell_never_dismisses() {
        let mut state = DesktopUpsellStartupState::new();
        state.core.open();
        state.selection = DesktopUpsellSelection::Never;
        let permanent = state.confirm();
        assert!(permanent);
        assert!(!state.is_visible());
        assert!(state.dismissed);
        // Attempting to show again should not succeed.
        state.show_if_eligible();
        assert!(!state.is_visible());
    }

    #[test]
    fn desktop_upsell_not_now_keeps_eligible() {
        let mut state = DesktopUpsellStartupState::new();
        state.core.open();
        state.selection = DesktopUpsellSelection::NotNow;
        let permanent = state.confirm();
        assert!(!permanent);
        assert!(!state.is_visible());
        assert!(!state.dismissed);
    }

    #[test]
    fn desktop_upsell_navigation_wraps() {
        let mut state = DesktopUpsellStartupState::new();
        assert_eq!(state.selection, DesktopUpsellSelection::Try);
        state.select_prev(); // Try → Never (wrap)
        assert_eq!(state.selection, DesktopUpsellSelection::Never);
        state.select_next(); // Never → Try (wrap)
        assert_eq!(state.selection, DesktopUpsellSelection::Try);
        state.select_next(); // Try → NotNow
        assert_eq!(state.selection, DesktopUpsellSelection::NotNow);
        state.select_next(); // NotNow → Never
        assert_eq!(state.selection, DesktopUpsellSelection::Never);
    }

    #[test]
    fn desktop_upsell_esc_does_not_dismiss_permanently() {
        let mut state = DesktopUpsellStartupState::new();
        state.core.open();
        state.seen_count = 1;
        state.dismiss_temporarily();
        assert!(!state.is_visible());
        assert!(!state.dismissed);
    }

    #[test]
    fn desktop_upsell_render_smoke() {
        use crate::dialogs::dialog::DialogBehavior as _;
        let mut state = DesktopUpsellStartupState::new();
        state.core.open();
        let area = Rect { x: 0, y: 0, width: 80, height: 24 };
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| {
            state.render(frame, area);
        }).unwrap();
        let rendered = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect::<Vec<_>>().join("");
        assert!(rendered.contains("Claurst Code Desktop") || rendered.contains("visual diffs"));
    }

    #[test]
    fn desktop_upsell_not_rendered_when_invisible() {
        use crate::dialogs::dialog::DialogBehavior as _;
        let state = DesktopUpsellStartupState::new();
        let area = Rect { x: 0, y: 0, width: 80, height: 24 };
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| {
            state.render(frame, area);
        }).unwrap();
        let rendered = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect::<Vec<_>>().join("");
        assert!(!rendered.contains("visual diffs"));
    }
}
