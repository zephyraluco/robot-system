// free_mode_dialog.rs — Setup dialog for the composite "Free" provider.
//
// Walks the user through the multi-provider free-mode caveats and collects
// API keys from any subset of the supported upstreams. The chain stacks
// many free tiers (Groq, Cerebras, Google, Mistral, SambaNova, NVIDIA,
// Cohere, OpenRouter, OpenCode Zen, Z.AI, Zhipu) behind one synthetic
// `free/auto` model — the more keys the user pastes in, the more
// providers the router can fall back to. Minimum 1 key to enable; more
// is better.
//
// Built on the shared `DialogCore` + `DialogBehavior` base: the modal frame,
// overlay, title bar and the key/mouse capture pipeline come from the base;
// this module only implements field navigation/editing (`on_key`) and the
// body/footer rendering (`render_content`). The active field is stored in
// `core.focus_zone()` so the pipeline's built-in Tab / Shift+Tab cycling
// moves between fields.
//
// Layout:
//   ┌─ Connect Free (multi-provider — 2/11 keys) ──────────┐
//   │ Stack free tiers behind one endpoint.                │
//   │ TIP More keys = better availability and higher caps. │
//   │                                                      │
//   │ ▸ Groq                          console.groq.com/..  │
//   │   ••••••••AbCd_                                      │
//   │   Cerebras                      cloud.cerebras.ai    │
//   │   paste your API key here...                         │
//   │   …↑/↓ to scroll                                     │
//   │ ↑/↓ next   enter confirm (2 keys — more = better)    │
//   └──────────────────────────────────────────────────────┘

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::Stylize;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use claurst_api::{FreeUpstream, FREE_CATALOG};

use crate::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::input::normalize_char_with_shift;
use crate::keyboard_enhancement_active;
use crate::overlays::{CLAURST_PANEL_BG, ModalLayout};

/// One row in the dialog — one provider's name, URL, and the user's
/// (possibly empty) typed key.
#[derive(Debug, Clone)]
pub struct FreeModeField {
    pub upstream: &'static FreeUpstream,
    pub key: String,
}

pub struct FreeModeDialogState {
    pub core: DialogCore,
    pub fields: Vec<FreeModeField>,
    /// First visible field index (for scrolling when fields > viewport).
    pub scroll_offset: usize,
}

impl Default for FreeModeDialogState {
    fn default() -> Self {
        Self::new()
    }
}

impl FreeModeDialogState {
    pub fn new() -> Self {
        let fields = FREE_CATALOG
            .iter()
            .map(|upstream| FreeModeField {
                upstream,
                key: String::new(),
            })
            .collect();
        // header/footer heights are 0: the title row and footer hint are part
        // of the body paragraph, matching the original hand-rolled layout.
        let core = DialogCore::new(Self::title_for(0), 84, 24)
            .header_height(0)
            .footer_height(0);
        Self {
            core,
            fields,
            scroll_offset: 0,
        }
    }

    fn title_for(filled: usize) -> String {
        format!(
            "Connect Free (multi-provider \u{2014} {}/{} keys)",
            filled,
            FREE_CATALOG.len()
        )
    }

    /// Push the current filled-key count into the dialog title.
    fn sync_title(&mut self) {
        let filled = self.filled_count();
        self.core.set_title(Self::title_for(filled));
    }

    /// Open the dialog, pre-populating each row from `existing[upstream.id]`
    /// when present.
    pub fn open(&mut self, existing: &[(&str, String)]) {
        for field in &mut self.fields {
            field.key.clear();
        }
        for (id, key) in existing {
            if let Some(field) = self.fields.iter_mut().find(|f| f.upstream.id == *id) {
                field.key = key.clone();
            }
        }
        // Start on the first empty field, or the first field if none are empty.
        let start = self
            .fields
            .iter()
            .position(|f| f.key.is_empty())
            .unwrap_or(0);
        self.core.set_title(Self::title_for(self.filled_count()));
        self.core.open();
        self.core.set_focus_zone(start);
        self.scroll_offset = 0;
        self.ensure_active_visible();
    }

    pub fn close(&mut self) {
        self.core.close();
        for field in &mut self.fields {
            field.key.clear();
        }
        self.core.set_focus_zone(0);
        self.scroll_offset = 0;
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    /// Number of rows shown at once in the scrolling viewport.
    pub const VISIBLE_ROWS: usize = 4;

    /// Index of the field the user is currently editing (stored in the
    /// embedded `DialogCore`'s focus zone so Tab / Shift+Tab cycle it).
    pub fn active_idx(&self) -> usize {
        self.core
            .focus_zone()
            .min(self.fields.len().saturating_sub(1))
    }

    pub fn move_next(&mut self) {
        if self.fields.is_empty() {
            return;
        }
        let next = (self.active_idx() + 1) % self.fields.len();
        self.core.set_focus_zone(next);
        self.ensure_active_visible();
    }

    pub fn move_prev(&mut self) {
        if self.fields.is_empty() {
            return;
        }
        let prev = if self.active_idx() == 0 {
            self.fields.len() - 1
        } else {
            self.active_idx() - 1
        };
        self.core.set_focus_zone(prev);
        self.ensure_active_visible();
    }

    fn ensure_active_visible(&mut self) {
        let active = self.active_idx();
        if active < self.scroll_offset {
            self.scroll_offset = active;
        } else if active >= self.scroll_offset + Self::VISIBLE_ROWS {
            self.scroll_offset = active + 1 - Self::VISIBLE_ROWS;
        }
    }

    pub fn insert_char(&mut self, c: char) {
        let idx = self.active_idx();
        if let Some(field) = self.fields.get_mut(idx) {
            field.key.push(c);
        }
        self.sync_title();
    }

    pub fn backspace(&mut self) {
        let idx = self.active_idx();
        if let Some(field) = self.fields.get_mut(idx) {
            field.key.pop();
        }
        self.sync_title();
    }

    /// Enabling Free mode requires at least one non-empty key. More is better.
    pub fn can_submit(&self) -> bool {
        self.fields.iter().any(|f| !f.key.trim().is_empty())
    }

    pub fn filled_count(&self) -> usize {
        self.fields.iter().filter(|f| !f.key.trim().is_empty()).count()
    }

    /// Consume the dialog state, returning every non-empty `(provider_id, key)`
    /// pair the user entered.
    pub fn take_values(&mut self) -> Vec<(&'static str, String)> {
        let out: Vec<(&'static str, String)> = self
            .fields
            .iter()
            .filter_map(|f| {
                let trimmed = f.key.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some((f.upstream.id, trimmed.to_string()))
                }
            })
            .collect();
        self.close();
        out
    }
}

// ---------------------------------------------------------------------------
// DialogBehavior — shared modal frame + key/mouse capture pipeline
// ---------------------------------------------------------------------------

impl DialogBehavior for FreeModeDialogState {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    /// One focus zone per provider field: the pipeline's built-in Tab /
    /// Shift+Tab cycling then moves between fields, matching the old
    /// `Tab → move_next` behaviour.
    fn focus_zones(&self) -> usize {
        self.fields.len()
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        match key.code {
            KeyCode::Down => {
                self.move_next();
                DialogOutcome::Handled
            }
            KeyCode::Up => {
                self.move_prev();
                DialogOutcome::Handled
            }
            KeyCode::Enter => {
                if self.can_submit() {
                    // The caller picks the values up via `take_values()`
                    // (which also closes the dialog).
                    DialogOutcome::Confirmed
                } else {
                    self.move_next();
                    DialogOutcome::Handled
                }
            }
            KeyCode::Backspace => {
                self.backspace();
                DialogOutcome::Handled
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER | KeyModifiers::ALT) =>
            {
                let c = if keyboard_enhancement_active() {
                    normalize_char_with_shift(c, key.modifiers)
                } else {
                    c
                };
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
        let tip = Color::Rgb(120, 210, 150);

        let mut lines: Vec<Line<'static>> = Vec::new();

        // Title row: dynamic title on the left, right-aligned "esc" hint on
        // the same line (original hand-rolled layout).
        let title_text = self.core.title().to_string();
        let title_pad = layout
            .body_area
            .width
            .saturating_sub(title_text.chars().count() as u16 + 5) as usize;
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

        // Description (one tight line) + tip.
        lines.push(Line::from(vec![Span::styled(
            " Stack free tiers behind one endpoint.",
            Style::default().fg(muted),
        )]));
        lines.push(Line::from(vec![
            Span::styled(" TIP ", Style::default().fg(tip).add_modifier(Modifier::BOLD)),
            Span::styled(
                "More keys = better availability and higher caps.",
                Style::default().fg(tip),
            ),
        ]));
        lines.push(Line::from(""));

        // Field viewport
        let start = self.scroll_offset;
        let end = (start + Self::VISIBLE_ROWS).min(self.fields.len());
        if start > 0 {
            lines.push(Line::from(vec![Span::styled(
                format!("   \u{2191} {} above", start),
                Style::default().fg(dim),
            )]));
        }

        let row_label_width: usize = self
            .fields
            .iter()
            .map(|f| f.upstream.title.chars().count())
            .max()
            .unwrap_or(0)
            .max(8);

        let active = self.active_idx();
        for idx in start..end {
            let field = &self.fields[idx];
            let is_active = idx == active;
            let marker = if is_active { "\u{25b8}" } else { " " };
            let label_style = if is_active {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(muted)
            };
            let url_style = Style::default().fg(dim);

            let label_padded =
                format!("{:<width$}", field.upstream.title, width = row_label_width);
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", marker), Style::default().fg(pink)),
                Span::styled(label_padded, label_style),
                Span::styled("   ", Style::default()),
                Span::styled(field.upstream.key_url.to_string(), url_style),
            ]));

            let masked = mask_key(&field.key);
            let input_style = if field.key.is_empty() {
                Style::default().fg(dim)
            } else if is_active {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let cursor = if is_active { "_" } else { "" };
            lines.push(Line::from(vec![
                Span::styled("     ", Style::default()),
                Span::styled(masked, input_style),
                Span::styled(cursor.to_string(), Style::default().fg(pink)),
            ]));
        }

        if end < self.fields.len() {
            lines.push(Line::from(vec![Span::styled(
                format!("   \u{2193} {} more", self.fields.len() - end),
                Style::default().fg(dim),
            )]));
        }

        lines.push(Line::from(""));

        // Footer hint (part of the same paragraph, as in the original layout).
        let confirm_hint = if self.can_submit() {
            let filled = self.filled_count();
            format!(
                "enter confirm ({} key{} \u{2014} more = better)",
                filled,
                if filled == 1 { "" } else { "s" }
            )
        } else {
            "paste at least 1 key \u{2014} as many as you can add is better".to_string()
        };
        lines.push(Line::from(vec![
            Span::styled(" \u{2191}/\u{2193}", Style::default().fg(dim)),
            Span::styled(" next field   ", Style::default().fg(dim)),
            Span::styled(confirm_hint, Style::default().fg(dim)),
        ]));

        let para = Paragraph::new(lines).bg(CLAURST_PANEL_BG);
        frame.render_widget(para, layout.body_area);
    }
}

// ---------------------------------------------------------------------------
// Key masking helper
// ---------------------------------------------------------------------------

fn mask_key(input: &str) -> String {
    if input.is_empty() {
        "paste your API key here...".to_string()
    } else {
        let chars: Vec<char> = input.chars().collect();
        if chars.len() <= 4 {
            input.to_string()
        } else {
            let tail: String = chars[chars.len() - 4..].iter().collect();
            format!("{}{}", "\u{2022}".repeat(chars.len() - 4), tail)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn defaults_hidden() {
        let s = FreeModeDialogState::new();
        assert!(!s.is_visible());
        assert_eq!(s.fields.len(), FREE_CATALOG.len());
    }

    #[test]
    fn open_starts_on_first_empty_field() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        assert!(s.is_visible());
        assert_eq!(s.active_idx(), 0);
    }

    #[test]
    fn open_seeds_existing_keys_and_skips_to_first_empty() {
        let mut s = FreeModeDialogState::new();
        s.open(&[(FREE_CATALOG[0].id, "existing-key".to_string())]);
        assert_eq!(s.fields[0].key, "existing-key");
        // First empty is the second field.
        assert_eq!(s.active_idx(), 1);
    }

    #[test]
    fn move_next_wraps() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        let n = s.fields.len();
        s.core.set_focus_zone(n - 1);
        s.move_next();
        assert_eq!(s.active_idx(), 0);
    }

    #[test]
    fn move_prev_wraps() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        s.move_prev();
        assert_eq!(s.active_idx(), s.fields.len() - 1);
    }

    #[test]
    fn scroll_offset_follows_active() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        for _ in 0..FreeModeDialogState::VISIBLE_ROWS {
            s.move_next();
        }
        assert!(s.scroll_offset > 0);
        assert!(s.active_idx() >= s.scroll_offset);
        assert!(s.active_idx() < s.scroll_offset + FreeModeDialogState::VISIBLE_ROWS);
    }

    #[test]
    fn insert_and_backspace_target_active_field() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        s.insert_char('a');
        s.insert_char('b');
        assert_eq!(s.fields[0].key, "ab");
        s.backspace();
        assert_eq!(s.fields[0].key, "a");
    }

    #[test]
    fn can_submit_requires_at_least_one_key() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        assert!(!s.can_submit());
        s.insert_char('k');
        assert!(s.can_submit());
    }

    #[test]
    fn take_values_returns_only_non_empty_trimmed_pairs_and_closes() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        s.insert_char(' ');
        s.insert_char('a');
        s.insert_char(' ');
        s.move_next();
        s.insert_char('b');
        let values = s.take_values();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0], (FREE_CATALOG[0].id, "a".to_string()));
        assert_eq!(values[1], (FREE_CATALOG[1].id, "b".to_string()));
        assert!(!s.is_visible());
    }

    #[test]
    fn mask_key_hides_all_but_last_four() {
        assert_eq!(mask_key(""), "paste your API key here...");
        assert_eq!(mask_key("abc"), "abc");
        assert_eq!(mask_key("abcdefgh"), "\u{2022}\u{2022}\u{2022}\u{2022}efgh");
    }

    // ---- DialogBehavior pipeline ----------------------------------------

    #[test]
    fn esc_closes_via_pipeline() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        let out = s.handle_key(key(KeyCode::Esc));
        assert_eq!(out, DialogOutcome::Cancelled);
        assert!(!s.is_visible());
    }

    #[test]
    fn tab_cycles_fields_via_pipeline() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        let out = s.handle_key(key(KeyCode::Tab));
        assert_eq!(out, DialogOutcome::Handled);
        assert_eq!(s.active_idx(), 1);
        s.handle_key(key(KeyCode::BackTab));
        assert_eq!(s.active_idx(), 0);
    }

    #[test]
    fn enter_without_keys_moves_next_not_confirmed() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        let out = s.handle_key(key(KeyCode::Enter));
        assert_eq!(out, DialogOutcome::Handled);
        assert_eq!(s.active_idx(), 1);
        assert!(s.is_visible());
    }

    #[test]
    fn enter_with_key_confirms() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        s.insert_char('k');
        let out = s.handle_key(key(KeyCode::Enter));
        assert_eq!(out, DialogOutcome::Confirmed);
        // Caller closes via take_values().
        assert!(s.is_visible());
    }

    #[test]
    fn char_keys_edit_active_field_via_pipeline() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        s.handle_key(key(KeyCode::Char('x')));
        assert_eq!(s.fields[0].key, "x");
        s.handle_key(key(KeyCode::Backspace));
        assert_eq!(s.fields[0].key, "");
    }

    #[test]
    fn modal_swallows_unhandled_keys() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        let out = s.handle_key(key(KeyCode::F(5)));
        assert_eq!(out, DialogOutcome::Handled);
        assert!(s.is_visible());
    }

    #[test]
    fn renders_without_panic() {
        let mut s = FreeModeDialogState::new();
        s.open(&[]);
        s.insert_char('k');
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| DialogBehavior::render(&s, frame, frame.area()))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Connect Free"));
        assert!(content.contains("Groq"));
    }
}
