// key_input_dialog.rs — Masked text input overlay for entering API keys.
//
// Provides a modal dialog that collects an API key from the user with
// masked display (showing only the last 4 characters).
// Built on the shared `DialogCore` + `DialogBehavior` base (crate::dialogs::dialog).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::prelude::Stylize;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::dialogs::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::overlays::{ModalLayout, CLAURST_PANEL_BG};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// State for the API key input dialog.
pub struct KeyInputDialogState {
    /// Embedded generic dialog base (visibility, geometry, title).
    pub core: DialogCore,
    pub provider_id: String,
    pub provider_name: String,
    pub input: String,
    pub cursor_pos: usize,
}

impl Default for KeyInputDialogState {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyInputDialogState {
    pub fn new() -> Self {
        Self {
            core: DialogCore::new("Connect", 60, 9)
                .header_height(0)
                .footer_height(0),
            provider_id: String::new(),
            provider_name: String::new(),
            input: String::new(),
            cursor_pos: 0,
        }
    }

    /// Open the dialog for a specific provider.
    pub fn open(&mut self, provider_id: String, provider_name: String) {
        self.core.open();
        self.provider_id = provider_id;
        self.provider_name = provider_name;
        self.input.clear();
        self.cursor_pos = 0;
    }

    /// Close and clear the dialog.
    pub fn close(&mut self) {
        self.core.close();
        self.input.clear();
        self.cursor_pos = 0;
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    /// Insert a character at the cursor position.
    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            // Find the previous char boundary
            let prev = self.input[..self.cursor_pos]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.remove(prev);
            self.cursor_pos = prev;
        }
    }

    /// Take the entered key and close the dialog.
    pub fn take_key(&mut self) -> String {
        let key = self.input.clone();
        self.close();
        key
    }
}

impl DialogBehavior for KeyInputDialogState {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        match key.code {
            // Enter signals confirmation but does NOT close here — `close()`
            // clears `input`, and the caller reads the value via `take_key()`
            // (which closes the dialog) after seeing `Confirmed`.
            KeyCode::Enter => DialogOutcome::Confirmed,
            KeyCode::Backspace => {
                self.backspace();
                DialogOutcome::Handled
            }
            KeyCode::Char(c) => {
                self.insert_char(c);
                DialogOutcome::Handled
            }
            _ => DialogOutcome::Ignored,
        }
    }

    fn render_content(&self, frame: &mut Frame, layout: &ModalLayout) {
        let pink = Color::Rgb(233, 30, 99);
        let dim = Color::Rgb(90, 90, 90);
        let dialog_bg = CLAURST_PANEL_BG;

        let inner = Rect {
            x: layout.body_area.x,
            y: layout.body_area.y,
            width: layout.body_area.width,
            height: layout.body_area.height,
        };

        // ── Build lines ──
        let mut lines: Vec<Line<'static>> = Vec::new();

        // Title row: "Connect {provider}" on left, "esc" on right
        let title_text = format!("Connect {}", self.provider_name);
        let title_pad = inner.width.saturating_sub(title_text.len() as u16 + 5) as usize;
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {}", title_text),
                Style::default().fg(pink).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>width$}", "esc ", width = title_pad),
                Style::default().fg(dim),
            ),
        ]));

        // Blank line
        lines.push(Line::from(""));

        // "API Key:" label
        lines.push(Line::from(vec![Span::styled(
            " API Key:",
            Style::default().fg(Color::Rgb(180, 180, 180)),
        )]));

        // Masked key display (show last 4 chars, mask the rest)
        let masked = if self.input.is_empty() {
            "paste your API key here...".to_string()
        } else {
            let len = self.input.len();
            if len <= 4 {
                self.input.clone()
            } else {
                format!(
                    "{}{}",
                    "\u{2022}".repeat(len - 4),
                    &self.input[len - 4..]
                )
            }
        };

        let input_style = if self.input.is_empty() {
            Style::default().fg(dim)
        } else {
            Style::default().fg(Color::White)
        };

        lines.push(Line::from(vec![
            Span::styled(format!(" {}", masked), input_style),
            Span::styled("_", Style::default().fg(pink)), // cursor
        ]));

        // Blank line
        lines.push(Line::from(""));

        // Hint row
        lines.push(Line::from(vec![
            Span::styled(" enter", Style::default().fg(dim)),
            Span::styled(" confirm", Style::default().fg(dim)),
        ]));

        let para = Paragraph::new(lines).bg(dialog_bg);
        frame.render_widget(para, inner);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Regression: Enter must NOT clear the input before the caller reads it
    /// via `take_key()`. `close()` clears `input`, so `on_key` must only
    /// signal `Confirmed` and leave the closing to `take_key()` — otherwise
    /// the provider activation sees an empty key and the model picker never
    /// opens (issue: "输入完 key 后不会进入模型选择界面").
    #[test]
    fn enter_then_take_key_returns_typed_value() {
        let mut dlg = KeyInputDialogState::new();
        dlg.open("groq".into(), "Groq".into());
        for c in "sk-test-1234".chars() {
            assert_eq!(dlg.handle_key(key(KeyCode::Char(c))), DialogOutcome::Handled);
        }
        // Enter signals Confirmed; the dialog stays open (value intact).
        assert_eq!(dlg.handle_key(key(KeyCode::Enter)), DialogOutcome::Confirmed);
        let api_key = dlg.take_key();
        assert_eq!(api_key, "sk-test-1234");
        // take_key() closes the dialog.
        assert!(!dlg.is_visible());
    }

    #[test]
    fn esc_closes_and_cancelled() {
        let mut dlg = KeyInputDialogState::new();
        dlg.open("groq".into(), "Groq".into());
        assert_eq!(dlg.handle_key(key(KeyCode::Esc)), DialogOutcome::Cancelled);
        assert!(!dlg.is_visible());
    }
}

// ---------------------------------------------------------------------------
// (Rendering is provided by `DialogBehavior::render` + `render_content` above.)
// ---------------------------------------------------------------------------
