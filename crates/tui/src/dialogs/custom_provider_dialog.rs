// custom_provider_dialog.rs — Modal dialog for entering a custom provider URL and API key.
//
// Collects both a base URL and an API key for the custom OpenAI-compatible
// provider used by /connect.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomProviderField {
    Url,
    ApiKey,
}

pub struct CustomProviderDialogState {
    /// Embedded generic dialog base (visibility, geometry, title). The active
    /// field is stored in `core.focus_zone()` so the pipeline's built-in
    /// Tab / Shift+Tab cycling switches fields (see `active_field()`).
    pub core: DialogCore,
    pub provider_id: String,
    pub provider_name: String,
    pub url_input: String,
    pub api_key_input: String,
}

impl Default for CustomProviderDialogState {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomProviderDialogState {
    pub fn new() -> Self {
        Self {
            core: DialogCore::new("Connect", 76, 13)
                .header_height(0)
                .footer_height(0),
            provider_id: String::new(),
            provider_name: String::new(),
            url_input: String::new(),
            api_key_input: String::new(),
        }
    }

    /// The currently focused input field, derived from `core.focus_zone()`.
    pub fn active_field(&self) -> CustomProviderField {
        match self.core.focus_zone() {
            1 => CustomProviderField::ApiKey,
            _ => CustomProviderField::Url,
        }
    }

    pub fn open(&mut self, provider_id: String, provider_name: String, current_url: Option<String>) {
        self.core.open();
        self.core.set_focus_zone(0);
        self.provider_id = provider_id;
        self.provider_name = provider_name;
        self.url_input = current_url.unwrap_or_default();
        self.api_key_input.clear();
    }

    pub fn close(&mut self) {
        self.core.close();
        self.url_input.clear();
        self.api_key_input.clear();
        self.core.set_focus_zone(0);
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    pub fn move_next_field(&mut self) {
        let next = match self.active_field() {
            CustomProviderField::Url => 1,
            CustomProviderField::ApiKey => 0,
        };
        self.core.set_focus_zone(next);
    }

    pub fn move_prev_field(&mut self) {
        self.move_next_field();
    }

    pub fn insert_char(&mut self, c: char) {
        match self.active_field() {
            CustomProviderField::Url => self.url_input.push(c),
            CustomProviderField::ApiKey => self.api_key_input.push(c),
        }
    }

    pub fn backspace(&mut self) {
        match self.active_field() {
            CustomProviderField::Url => {
                self.url_input.pop();
            }
            CustomProviderField::ApiKey => {
                self.api_key_input.pop();
            }
        }
    }

    pub fn can_submit(&self) -> bool {
        !self.url_input.trim().is_empty()
    }

    pub fn take_values(&mut self) -> (String, String) {
        let url = self.url_input.trim().to_string();
        let api_key = self.api_key_input.clone();
        self.close();
        (url, api_key)
    }
}

impl DialogBehavior for CustomProviderDialogState {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    /// Two focus zones (URL / API key): the pipeline's built-in Tab /
    /// Shift+Tab cycling switches fields via `core.focus_zone()`.
    fn focus_zones(&self) -> usize {
        2
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        match key.code {
            // NOTE: Tab / BackTab are consumed by the dispatch pipeline
            // (focus-zone cycling) before this hook runs.
            KeyCode::Down => {
                self.move_next_field();
                DialogOutcome::Handled
            }
            KeyCode::Up => {
                self.move_prev_field();
                DialogOutcome::Handled
            }
            KeyCode::Enter => {
                if self.can_submit() {
                    // Signal confirmation but do NOT close here — `close()`
                    // clears both inputs, and the caller reads them via
                    // `take_values()` (which closes the dialog).
                    DialogOutcome::Confirmed
                } else {
                    self.move_next_field();
                    DialogOutcome::Handled
                }
            }
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
        let muted = Color::Rgb(180, 180, 180);
        let dialog_bg = CLAURST_PANEL_BG;

        let inner = Rect {
            x: layout.body_area.x,
            y: layout.body_area.y,
            width: layout.body_area.width,
            height: layout.body_area.height,
        };

        let title_text = format!("Connect {}", self.provider_name);
        let title_pad = inner.width.saturating_sub(title_text.len() as u16 + 5) as usize;

        let url_style = if self.active_field() == CustomProviderField::Url {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let key_style = if self.active_field() == CustomProviderField::ApiKey {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let url_text = if self.url_input.is_empty() {
            "https://your-openai-compatible-endpoint/v1".to_string()
        } else {
            self.url_input.clone()
        };

        let masked_key = if self.api_key_input.is_empty() {
            "paste your API key here...".to_string()
        } else {
            let chars: Vec<char> = self.api_key_input.chars().collect();
            if chars.len() <= 4 {
                self.api_key_input.clone()
            } else {
                let visible: String = chars[chars.len() - 4..].iter().collect();
                format!("{}{}", "•".repeat(chars.len() - 4), visible)
            }
        };

        let confirm_hint = if self.can_submit() {
            " enter confirm"
        } else {
            " fill URL field"
        };

        let mut lines: Vec<Line<'static>> = Vec::new();
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
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(" URL:", Style::default().fg(muted))]));
        lines.push(Line::from(vec![
            Span::styled(format!(" {}", url_text), url_style),
            Span::styled(
                if self.active_field() == CustomProviderField::Url { "_" } else { "" },
                Style::default().fg(pink),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(" API Key:", Style::default().fg(muted))]));
        lines.push(Line::from(vec![
            Span::styled(format!(" {}", masked_key), key_style),
            Span::styled(
                if self.active_field() == CustomProviderField::ApiKey { "_" } else { "" },
                Style::default().fg(pink),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" tab", Style::default().fg(dim)),
            Span::styled(" switch field  ", Style::default().fg(dim)),
            Span::styled(confirm_hint, Style::default().fg(dim)),
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

    /// Regression: Enter must NOT clear the inputs before the caller reads
    /// them via `take_values()`. `close()` clears both fields, so `on_key`
    /// must only signal `Confirmed` and leave the closing to `take_values()`.
    #[test]
    fn enter_then_take_values_returns_typed_values() {
        let mut dlg = CustomProviderDialogState::new();
        dlg.open("custom-openai".into(), "Custom".into(), None);
        for c in "https://api.example.com/v1".chars() {
            assert_eq!(dlg.handle_key(key(KeyCode::Char(c))), DialogOutcome::Handled);
        }
        // Tab to the API-key field, then type the key.
        assert_eq!(dlg.handle_key(key(KeyCode::Tab)), DialogOutcome::Handled);
        for c in "sk-secret".chars() {
            assert_eq!(dlg.handle_key(key(KeyCode::Char(c))), DialogOutcome::Handled);
        }
        // Enter signals Confirmed; the dialog stays open (values intact).
        assert_eq!(dlg.handle_key(key(KeyCode::Enter)), DialogOutcome::Confirmed);
        let (url, api_key) = dlg.take_values();
        assert_eq!(url, "https://api.example.com/v1");
        assert_eq!(api_key, "sk-secret");
        // take_values() closes the dialog.
        assert!(!dlg.is_visible());
    }
}
