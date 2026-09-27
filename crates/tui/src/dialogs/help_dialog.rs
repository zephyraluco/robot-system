//! Help dialog — the `? / F1 / /help` shortcut & command reference.
//!
//! A centred modal built on the shared dialog base (`DialogCore` +
//! `DialogBehavior` in `crate::dialogs::dialog`): the embedded `DialogCore` owns
//! visibility/geometry and the `DialogBehavior` dispatch pipeline
//! (`handle_key` / `handle_mouse` / `render`) captures every keyboard + mouse
//! event while open, so nothing leaks into the transcript underneath.
//!
//! Two columns: all keyboard shortcuts on the left, the `/` command catalog on
//! the right, with a live search filter across the command names, aliases and
//! descriptions.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::dialogs::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::overlays::{
    modal_header_line_area, modal_search_line, render_modal_title_frame, ModalLayout,
    CLAURST_ACCENT, CLAURST_MUTED, CLAURST_PANEL_BG, CLAURST_TEXT,
};

/// Upper bound for `↓` scrolling; the renderer clamps to the real content
/// height every frame, so an over-generous bound is harmless and keeps the key
/// handler independent of the (only-known-at-render-time) scrollback size.
const SCROLL_BOUND: u16 = 50;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single command entry shown in the help dialog.
#[derive(Debug, Clone)]
pub struct HelpEntry {
    pub name: String,
    /// Comma-separated aliases, e.g. "h, ?"
    pub aliases: String,
    pub description: String,
    pub category: String,
}

/// State for the help dialog.
pub struct HelpDialogState {
    /// Embedded generic dialog base (visibility, geometry, title).
    pub core: DialogCore,
    pub scroll_offset: u16,
    /// Live search filter — only commands matching this substring are shown.
    pub filter: String,
    /// Dynamically populated entries from the command registry.
    pub commands: Vec<HelpEntry>,
}

impl Default for HelpDialogState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl HelpDialogState {
    pub fn new() -> Self {
        Self {
            core: DialogCore::new("Shortcuts & commands", 100, 36)
                .header_height(3)
                .footer_height(1),
            scroll_offset: 0,
            filter: String::new(),
            commands: Vec::new(),
        }
    }

    /// Populate (or replace) the command entries from the command registry.
    /// Entries are sorted by category then name.
    pub fn populate_from_commands(&mut self, entries: Vec<HelpEntry>) {
        self.commands = entries;
        // Sort stable by category, then name for consistent display.
        self.commands.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    pub fn toggle(&mut self) {
        if self.core.is_visible() {
            self.close();
        } else {
            self.core.open();
        }
    }

    pub fn close(&mut self) {
        self.core.close();
        // Reset the transient view state on close, like every other dialog.
        self.scroll_offset = 0;
        self.filter.clear();
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        if self.scroll_offset + 1 < SCROLL_BOUND {
            self.scroll_offset += 1;
        }
    }

    pub fn push_filter_char(&mut self, c: char) {
        self.filter.push(c);
        self.scroll_offset = 0;
    }

    pub fn pop_filter_char(&mut self) {
        self.filter.pop();
        self.scroll_offset = 0;
    }
}

// ---------------------------------------------------------------------------
// DialogBehaviour (the shared modal protocol)
// ---------------------------------------------------------------------------

impl DialogBehavior for HelpDialogState {
    crate::dialogs::dialog::dialog_core_accessors!();

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        // NOTE: Esc is consumed by the dispatch pipeline (→ Cancelled + close)
        // before this hook runs; `?` and F1 are the dialog's own close keys.
        let plain = !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
            && !key.modifiers.contains(KeyModifiers::SUPER);
        match key.code {
            KeyCode::F(1) => {
                self.close();
                DialogOutcome::Cancelled
            }
            KeyCode::Char('?') if plain => {
                self.close();
                DialogOutcome::Cancelled
            }
            KeyCode::Up => {
                self.scroll_up();
                DialogOutcome::Handled
            }
            KeyCode::Down => {
                self.scroll_down();
                DialogOutcome::Handled
            }
            KeyCode::PageUp => {
                for _ in 0..10 {
                    self.scroll_up();
                }
                DialogOutcome::Handled
            }
            KeyCode::PageDown => {
                for _ in 0..10 {
                    self.scroll_down();
                }
                DialogOutcome::Handled
            }
            KeyCode::Backspace => {
                self.pop_filter_char();
                DialogOutcome::Handled
            }
            KeyCode::Char(c) if plain => {
                self.push_filter_char(c);
                DialogOutcome::Handled
            }
            _ => DialogOutcome::Ignored,
        }
    }

    fn render_content(&self, frame: &mut Frame, layout: &ModalLayout) {
        use claurst_core::constants::APP_VERSION;

        render_modal_title_frame(frame, layout.header_area, "Shortcuts & commands", "esc");

        let search_line = modal_search_line(
            &self.filter,
            "Search shortcuts or commands",
            CLAURST_MUTED,
            CLAURST_TEXT,
        );
        if let Some(search_area) = modal_header_line_area(layout.header_area, 2) {
            frame.render_widget(Paragraph::new(search_line), search_area);
        }

        let content_area = layout.body_area;
        if content_area.height == 0 {
            return;
        }

        let col_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(42),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(content_area);

        self.render_shortcuts_column(frame, col_chunks[0]);
        self.render_divider(frame, col_chunks[1], content_area.height);
        self.render_commands_column(frame, col_chunks[2]);

        let version_line = Line::from(vec![Span::styled(
            format!(
                " v{}  ·  type to filter  ·  \u{2191}\u{2193} scroll commands  ·  esc close",
                APP_VERSION
            ),
            Style::default()
                .fg(CLAURST_MUTED)
                .add_modifier(Modifier::ITALIC),
        )]);
        frame.render_widget(Paragraph::new(version_line), layout.footer_area);
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

impl HelpDialogState {
    /// Left column: keyboard shortcuts, grouped by area.
    fn render_shortcuts_column(&self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let heading = |title: &str| {
            Line::from(Span::styled(
                title.to_string(),
                Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD),
            ))
        };

        lines.push(heading(" Keyboard Shortcuts"));
        lines.push(Line::from(""));

        lines.push(heading(" Navigation"));
        for (key, desc) in &[
            ("PageUp / PgDn", "Scroll messages"),
            ("j / k", "Scroll one line"),
            ("Home / End", "Top / bottom"),
        ] {
            lines.push(kb_line(key, desc));
        }
        lines.push(Line::from(""));

        lines.push(heading(" Input"));
        for (key, desc) in &[
            ("Enter", "Submit message"),
            ("Up / Down", "Input history"),
            ("Ctrl+R", "Search history"),
            ("Alt+E", "Expand pasted text"),
            ("Esc", "Cancel / close"),
        ] {
            lines.push(kb_line(key, desc));
        }
        lines.push(Line::from(""));

        lines.push(heading(" App"));
        for (key, desc) in &[
            ("F1 / ?", "Toggle help"),
            ("Ctrl+Shift+A", "Model picker"),
            ("Ctrl+K", "Command palette"),
            ("Ctrl+C", "Cancel / quit"),
            ("Ctrl+D", "Quit (empty input)"),
            ("Ctrl+L", "Clear screen"),
            ("t", "Expand/collapse thinking"),
        ] {
            lines.push(kb_line(key, desc));
        }

        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .style(Style::default().bg(CLAURST_PANEL_BG)),
            area,
        );
    }

    /// The vertical rule between the two columns.
    fn render_divider(&self, frame: &mut Frame, area: Rect, height: u16) {
        let lines: Vec<Line<'static>> = (0..height)
            .map(|_| Line::from(Span::styled("\u{2502}", Style::default().fg(CLAURST_MUTED))))
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// Right column: the `/` command catalog, filtered by `self.filter` and
    /// scrolled by `self.scroll_offset`.
    fn render_commands_column(&self, frame: &mut Frame, area: Rect) {
        let filter_lc = self.filter.to_lowercase();
        let filtered: Vec<&HelpEntry> = self
            .commands
            .iter()
            .filter(|e| {
                filter_lc.is_empty()
                    || e.name.to_lowercase().contains(filter_lc.as_str())
                    || e.aliases.to_lowercase().contains(filter_lc.as_str())
                    || e.description.to_lowercase().contains(filter_lc.as_str())
            })
            .collect();

        let heading = |title: &str| {
            Line::from(Span::styled(
                title.to_string(),
                Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD),
            ))
        };

        let mut lines: Vec<Line<'static>> = vec![heading(" Slash Commands"), Line::from("")];

        let mut current_cat = "";
        for entry in &filtered {
            if entry.category.as_str() != current_cat {
                current_cat = entry.category.as_str();
                if lines.len() > 2 {
                    lines.push(Line::from(""));
                }
                lines.push(heading(&format!(" {}", entry.category)));
            }
            let aliases_text = if entry.aliases.is_empty() {
                String::new()
            } else {
                format!(" ({})", entry.aliases)
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("/{:<14}", entry.name),
                    Style::default().fg(CLAURST_TEXT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(aliases_text, Style::default().fg(CLAURST_MUTED)),
                Span::raw("  "),
                Span::styled(entry.description.clone(), Style::default().fg(CLAURST_MUTED)),
            ]));
        }

        if filtered.is_empty() {
            lines.push(Line::from(Span::styled(
                " No matching commands",
                Style::default().fg(CLAURST_MUTED),
            )));
        }

        let max_scroll = (lines.len() as u16).saturating_sub(area.height);
        let scroll = self.scroll_offset.min(max_scroll);

        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0))
                .style(Style::default().bg(CLAURST_PANEL_BG)),
            area,
        );
    }
}

/// One `key → description` row in the shortcuts column.
fn kb_line(key: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{:<20}", key),
            Style::default().fg(CLAURST_TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc.to_string(), Style::default().fg(CLAURST_MUTED)),
    ])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    fn entry(name: &str, category: &str) -> HelpEntry {
        HelpEntry {
            name: name.to_string(),
            aliases: String::new(),
            description: format!("{name} does things"),
            category: category.to_string(),
        }
    }

    fn opened() -> HelpDialogState {
        let mut h = HelpDialogState::new();
        h.populate_from_commands(vec![entry("help", "General"), entry("model", "Settings")]);
        h.core.open();
        h
    }

    fn render_text(state: &HelpDialogState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|f| DialogBehavior::render(state, f, f.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf.cell((x, y)).unwrap().symbol());
            }
        }
        out
    }

    #[test]
    fn toggle_opens_and_closes() {
        let mut h = HelpDialogState::new();
        assert!(!h.is_visible());
        h.toggle();
        assert!(h.is_visible());
        h.toggle();
        assert!(!h.is_visible());
    }

    #[test]
    fn close_resets_scroll_and_filter() {
        let mut h = opened();
        h.push_filter_char('m');
        h.scroll_down();
        assert_ne!(h.filter, "");
        assert_ne!(h.scroll_offset, 0);
        h.close();
        assert!(!h.is_visible());
        assert_eq!(h.filter, "");
        assert_eq!(h.scroll_offset, 0);
    }

    #[test]
    fn populate_sorts_by_category_then_name() {
        let mut h = HelpDialogState::new();
        h.populate_from_commands(vec![
            entry("zeta", "B"),
            entry("alpha", "B"),
            entry("mid", "A"),
        ]);
        let names: Vec<&str> = h.commands.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["mid", "alpha", "zeta"]);
    }

    #[test]
    fn typing_filters_and_backspace_restores() {
        let mut h = opened();
        h.handle_key(key(KeyCode::Char('m')));
        assert_eq!(h.filter, "m");
        h.handle_key(key(KeyCode::Backspace));
        assert_eq!(h.filter, "");
    }

    #[test]
    fn escape_and_question_mark_close_through_the_pipeline() {
        // Esc is handled by the dispatch pipeline; `?` and F1 by the dialog.
        let mut h = opened();
        assert!(h.handle_key(key(KeyCode::Esc)).is_cancelled());
        assert!(!h.is_visible());

        let mut h = opened();
        assert!(h.handle_key(key(KeyCode::Char('?'))).is_cancelled());
        assert!(!h.is_visible());

        let mut h = opened();
        assert!(h.handle_key(key(KeyCode::F(1))).is_cancelled());
        assert!(!h.is_visible());
    }

    #[test]
    fn arrows_scroll_but_unrelated_keys_keep_it_open() {
        let mut h = opened();
        h.handle_key(key(KeyCode::Down));
        assert_eq!(h.scroll_offset, 1);
        h.handle_key(key(KeyCode::Up));
        assert_eq!(h.scroll_offset, 0);
        // `↑` past the top clamps instead of underflowing.
        h.handle_key(key(KeyCode::Up));
        assert_eq!(h.scroll_offset, 0);

        let out = h.handle_key(key(KeyCode::Tab));
        assert!(!out.is_close(), "Tab must not close the help dialog");
        assert!(h.is_visible());
    }

    #[test]
    fn renders_both_columns_and_the_entries() {
        let text = render_text(&opened());
        assert!(text.contains("Shortcuts & commands"));
        assert!(text.contains("Keyboard Shortcuts"));
        assert!(text.contains("Slash Commands"));
        assert!(text.contains("Navigation"));
        assert!(text.contains("/help"));
        assert!(text.contains("/model"));
        assert!(text.contains("Settings"), "category header present");
    }

    #[test]
    fn filter_hides_non_matching_entries() {
        let mut h = opened();
        h.push_filter_char('m');
        let text = render_text(&h);
        assert!(text.contains("/model"), "matching entry stays");
        assert!(!text.contains("/help"), "non-matching entry is filtered out");
    }

    #[test]
    fn empty_registry_shows_the_empty_state() {
        let mut h = HelpDialogState::new();
        h.core.open();
        let text = render_text(&h);
        assert!(text.contains("No matching commands"));
    }
}
