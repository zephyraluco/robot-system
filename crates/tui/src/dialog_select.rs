// dialog_select.rs — Reusable fuzzy-search selection dialog widget.
//
// Used for the /connect provider picker and potentially for future
// selection dialogs (models, commands, sessions).
//
// `DialogSelectState` is built on the generic dialog base (`DialogCore` +
// `DialogBehavior` in `crate::dialog`): it embeds a `DialogCore` for
// visibility/geometry and routes keyboard + mouse events through the
// `DialogBehavior` dispatch pipeline (`handle_key` / `handle_mouse`),
// while keeping its legacy direct-manipulation methods (move_up, filter_push,
// …) for callers that have not migrated yet.

use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Alignment, Rect};
use ratatui::prelude::Stylize;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::cell::{Cell, RefCell};

use crate::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::overlays::{modal_search_line, ModalLayout, CLAURST_PANEL_BG};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single selectable item in the dialog.
#[derive(Debug, Clone)]
pub struct SelectItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub badge: Option<String>, // e.g., "FREE", "LOCAL", "NEW"
}

/// State for the DialogSelect overlay.
#[derive(Debug, Clone)]
pub struct DialogSelectState {
    /// Embedded generic dialog base (visibility, geometry, title).
    pub core: DialogCore,
    pub items: Vec<SelectItem>,
    pub selected_index: usize,
    pub filter: String,
    filtered_indices: Vec<usize>,
    /// The area where this dialog was last rendered (for mouse hit testing).
    pub last_render_area: Cell<Rect>,
    /// Maps absolute screen row → filtered item index. Built during render.
    row_to_item: RefCell<Vec<(u16, usize)>>,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl DialogSelectState {
    pub fn new(title: impl Into<String>, items: Vec<SelectItem>) -> Self {
        let count = items.len();
        let filtered: Vec<usize> = (0..count).collect();
        // 3 header rows (title / blank / search), no footer, body holds the
        // items; the height adapts to the filtered content via `sync_size`.
        let mut state = Self {
            core: DialogCore::new(title, 65, 20).header_height(3).footer_height(0),
            items,
            selected_index: 0,
            filter: String::new(),
            filtered_indices: filtered,
            last_render_area: Cell::new(Rect::default()),
            row_to_item: RefCell::new(Vec::new()),
        };
        state.sync_size();
        state
    }

    pub fn open(&mut self) {
        self.core.open();
        self.selected_index = 0;
        self.filter.clear();
        self.refilter();
        self.last_render_area.set(Rect::default());
        self.row_to_item.borrow_mut().clear();
    }

    pub fn close(&mut self) {
        self.core.close();
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    /// Take the currently selected item and close the dialog. Used by callers
    /// that dispatch through the `DialogBehavior` pipeline: when
    /// `handle_key` returns `DialogOutcome::Confirmed` (Enter), the confirmed
    /// item is consumed here.
    pub fn take_selected(&mut self) -> Option<SelectItem> {
        let item = self.selected().cloned();
        self.close();
        item
    }

    pub fn move_up(&mut self) {
        let count = self.filtered_indices.len();
        if count == 0 {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = count - 1;
        } else {
            self.selected_index -= 1;
        }
    }

    pub fn move_down(&mut self) {
        let count = self.filtered_indices.len();
        if count == 0 {
            return;
        }
        self.selected_index = (self.selected_index + 1) % count;
    }

    pub fn page_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(10);
    }

    pub fn page_down(&mut self) {
        self.selected_index =
            (self.selected_index + 10).min(self.filtered_indices.len().saturating_sub(1));
    }

    pub fn move_home(&mut self) {
        self.selected_index = 0;
    }

    pub fn move_end(&mut self) {
        self.selected_index = self.filtered_indices.len().saturating_sub(1);
    }

    /// Get the currently selected item (if any).
    pub fn selected(&self) -> Option<&SelectItem> {
        self.filtered_indices
            .get(self.selected_index)
            .and_then(|&idx| self.items.get(idx))
    }

    /// Type a character into the filter.
    pub fn filter_push(&mut self, c: char) {
        self.filter.push(c);
        self.refilter();
    }

    /// Backspace in the filter.
    pub fn filter_pop(&mut self) {
        self.filter.pop();
        self.refilter();
    }

    /// Check if a mouse position is inside the last rendered dialog area.
    pub fn contains(&self, column: u16, row: u16) -> bool {
        let area = self.last_render_area.get();
        area.width > 0
            && area.height > 0
            && column >= area.x
            && column < area.x.saturating_add(area.width)
            && row >= area.y
            && row < area.y.saturating_add(area.height)
    }

    /// Handle a mouse click at the given absolute screen row.
    /// Uses the row→item map built during the last render for pixel-accurate selection.
    /// Returns `true` if an item was selected, `false` otherwise.
    pub fn handle_mouse_click(&mut self, row: u16) -> bool {
        let map = self.row_to_item.borrow();
        for &(screen_row, item_idx) in map.iter() {
            if row == screen_row {
                self.selected_index = item_idx;
                return true;
            }
        }
        false
    }

    fn refilter(&mut self) {
        if self.filter.is_empty() {
            self.filtered_indices = (0..self.items.len()).collect();
        } else {
            let query = self.filter.to_lowercase();
            self.filtered_indices = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    item.title.to_lowercase().contains(&query)
                        || item.description.to_lowercase().contains(&query)
                        || item.category.to_lowercase().contains(&query)
                })
                .map(|(i, _)| i)
                .collect();
        }
        // Clamp selection
        if self.selected_index >= self.filtered_indices.len() {
            self.selected_index = self.filtered_indices.len().saturating_sub(1);
        }
        // The dialog height adapts to the filtered content.
        self.sync_size();
    }

    /// Push the content-derived height into the embedded `DialogCore` so the
    /// shared `render` pipeline sizes the modal accordingly (the height is
    /// still clamped to the screen by `begin_modal_frame`).
    fn sync_size(&mut self) {
        let height = self.content_height();
        self.core.set_size(65, height);
    }

    /// Total painted line count: header (title + blank + search) + items +
    /// category headers + gaps; floored at 8 so the dialog never collapses.
    fn content_height(&self) -> u16 {
        let item_lines = self.filtered_indices.len() as u16;
        let category_count = if self.filter.is_empty() {
            let mut sections = 0u16;
            let mut last_category: Option<&str> = None;
            for &idx in &self.filtered_indices {
                let category = self.items[idx].category.as_str();
                if last_category != Some(category) {
                    sections += 1;
                    last_category = Some(category);
                }
            }
            sections
        } else {
            0
        };
        (3 + item_lines + category_count * 2).max(8)
    }
}

// ---------------------------------------------------------------------------
// DialogBehavior — generic key/mouse capture pipeline
// ---------------------------------------------------------------------------

impl DialogBehavior for DialogSelectState {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    fn focus_zones(&self) -> usize {
        1
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        // NOTE: Esc is consumed by the dispatch pipeline (→ Cancelled) before
        // this hook runs.
        match key.code {
            KeyCode::Home => {
                self.move_home();
                DialogOutcome::Handled
            }
            KeyCode::End => {
                self.move_end();
                DialogOutcome::Handled
            }
            KeyCode::Up => {
                self.move_up();
                DialogOutcome::Handled
            }
            KeyCode::Down => {
                self.move_down();
                DialogOutcome::Handled
            }
            KeyCode::PageUp => {
                self.page_up();
                DialogOutcome::Handled
            }
            KeyCode::PageDown => {
                self.page_down();
                DialogOutcome::Handled
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_up();
                DialogOutcome::Handled
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_down();
                DialogOutcome::Handled
            }
            KeyCode::Enter => {
                // Confirm only when there is a selection; otherwise stay open.
                if self.selected().is_some() {
                    self.close();
                    DialogOutcome::Confirmed
                } else {
                    DialogOutcome::Ignored
                }
            }
            KeyCode::Backspace => {
                self.filter_pop();
                DialogOutcome::Handled
            }
            KeyCode::Char(c) if key.modifiers.is_empty() => {
                // Plain typing only: modified chars (Ctrl+V, Ctrl+C, …) are
                // shortcuts, not filter input. Modal capture still swallows
                // them so they never reach the UI underneath.
                self.filter_push(c);
                DialogOutcome::Handled
            }
            _ => DialogOutcome::Ignored,
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) -> DialogOutcome {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Pixel-accurate item selection via the row→item map built
                // during the last render. Clicks elsewhere inside the dialog
                // are absorbed.
                self.handle_mouse_click(mouse.row);
                DialogOutcome::Handled
            }
            MouseEventKind::ScrollUp => {
                self.move_up();
                DialogOutcome::Handled
            }
            MouseEventKind::ScrollDown => {
                self.move_down();
                DialogOutcome::Handled
            }
            _ => DialogOutcome::Ignored,
        }
    }

    fn render_content(&self, frame: &mut Frame, layout: &ModalLayout) {
        let dim = Color::Rgb(90, 90, 90);
        let dialog_bg = CLAURST_PANEL_BG;
        let highlight_bg = Color::Rgb(233, 30, 99); // pink highlight bar
        let highlight_fg = Color::White;
        let category_fg = Color::Rgb(233, 30, 99); // pink category names

        let dialog_area = layout.dialog_area;
        self.last_render_area.set(dialog_area);

        let header_area = layout.header_area;
        let body_area = layout.body_area;

        // ── Header extras (the shared `render` already drew the title on row 0) ──
        // "esc" hint, right-aligned on the title row.
        if header_area.height > 0 && header_area.width > 4 {
            let esc_area = Rect {
                height: 1,
                ..header_area
            };
            frame.render_widget(
                Paragraph::new("esc ")
                    .alignment(Alignment::Right)
                    .style(Style::default().fg(dim)),
                esc_area,
            );
        }
        // Search field on the third header row (row 1 stays blank).
        if header_area.height >= 3 {
            let search_area = Rect {
                y: header_area.y + 2,
                height: 1,
                ..header_area
            };
            frame.render_widget(
                Paragraph::new(modal_search_line(
                    &self.filter,
                    "Search",
                    dim,
                    Color::White,
                ))
                .bg(dialog_bg),
                search_area,
            );
        }

        if body_area.height == 0 {
            return;
        }

        // ── Scrollable items ──
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut row_map: Vec<(u16, usize)> = Vec::new();
        let mut current_line: u16 = 0;
        let mut last_category = String::new();

        for (display_idx, &item_idx) in self.filtered_indices.iter().enumerate() {
            let item = &self.items[item_idx];
            let is_selected = display_idx == self.selected_index;

            // Category header (only when not filtering)
            if item.category != last_category && self.filter.is_empty() {
                lines.push(Line::from(""));
                current_line += 1;
                lines.push(Line::from(vec![Span::styled(
                    format!(" {}", item.category),
                    Style::default()
                        .fg(category_fg)
                        .add_modifier(Modifier::BOLD),
                )]));
                current_line += 1;
                last_category = item.category.clone();
            }

            // Item — full-width highlight bar when selected
            let (item_fg, item_bg) = if is_selected {
                (highlight_fg, highlight_bg)
            } else {
                (Color::White, dialog_bg)
            };

            let mut spans = vec![Span::styled(
                format!(" {}", item.title),
                Style::default().fg(item_fg).bg(item_bg),
            )];

            // Auth hint in parens, dimmed
            if !item.description.is_empty() {
                spans.push(Span::styled(
                    format!(" {}", item.description),
                    Style::default()
                        .fg(if is_selected {
                            Color::Rgb(200, 200, 200)
                        } else {
                            dim
                        })
                        .bg(item_bg),
                ));
            }

            let badge_text = item.badge.clone().unwrap_or_default();
            let text_len: usize = spans.iter().map(|s| s.content.len()).sum();
            let badge_len = if badge_text.is_empty() {
                0
            } else {
                badge_text.len() + 1
            };
            let pad = body_area
                .width
                .saturating_sub(text_len as u16 + badge_len as u16) as usize;
            if pad > 0 {
                spans.push(Span::styled(
                    " ".repeat(pad),
                    Style::default().bg(item_bg),
                ));
            }
            if !badge_text.is_empty() {
                spans.push(Span::styled(
                    format!(" {}", badge_text),
                    Style::default()
                        .fg(if is_selected { highlight_fg } else { dim })
                        .bg(item_bg)
                        .add_modifier(Modifier::BOLD),
                ));
            }

            row_map.push((body_area.y + current_line, display_idx));
            lines.push(Line::from(spans));
            current_line += 1;
        }

        if self.filtered_indices.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                " No results found",
                Style::default().fg(dim),
            )]));
        }

        // ── Scroll ──
        let selected_item_line: u16 = {
            let mut line_num: u16 = 0;
            let mut last_cat = String::new();
            for (display_idx, &item_idx) in self.filtered_indices.iter().enumerate() {
                let item = &self.items[item_idx];
                if item.category != last_cat && self.filter.is_empty() {
                    line_num += 2; // blank line + category header
                    last_cat = item.category.clone();
                }
                if display_idx == self.selected_index {
                    break;
                }
                line_num += 1;
            }
            line_num
        };
        let total_lines = lines.len() as u16;
        let visible = body_area.height;
        let max_scroll = total_lines.saturating_sub(visible);
        let scroll_y = if selected_item_line + 3 >= visible {
            (selected_item_line + 3).saturating_sub(visible).min(max_scroll)
        } else {
            0
        };

        *self.row_to_item.borrow_mut() = row_map
            .into_iter()
            .filter_map(|(row, idx)| {
                let screen_row = row.saturating_sub(scroll_y);
                if screen_row >= body_area.y
                    && screen_row < body_area.y.saturating_add(body_area.height)
                {
                    Some((screen_row, idx))
                } else {
                    None
                }
            })
            .collect();

        let para = Paragraph::new(lines).bg(dialog_bg).scroll((scroll_y, 0));
        frame.render_widget(para, body_area);
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

    fn sample_items() -> Vec<SelectItem> {
        vec![
            SelectItem {
                id: "anthropic".into(),
                title: "Anthropic".into(),
                description: "Claude models".into(),
                category: "Recommended".into(),
                badge: None,
            },
            SelectItem {
                id: "openai".into(),
                title: "OpenAI".into(),
                description: "GPT models".into(),
                category: "Recommended".into(),
                badge: None,
            },
            SelectItem {
                id: "ollama".into(),
                title: "Ollama".into(),
                description: "Local inference + cloud models".into(),
                category: "Local".into(),
                badge: None,
            },
        ]
    }

    #[test]
    fn new_state_is_hidden() {
        let state = DialogSelectState::new("Test", sample_items());
        assert!(!state.is_visible());
        assert_eq!(state.selected_index, 0);
        assert!(state.filter.is_empty());
    }

    #[test]
    fn open_sets_visible() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        assert!(state.is_visible());
    }

    #[test]
    fn close_hides() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        state.close();
        assert!(!state.is_visible());
    }

    #[test]
    fn move_down_and_up_wrap() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        assert_eq!(state.selected_index, 0);
        state.move_down();
        assert_eq!(state.selected_index, 1);
        state.move_down();
        assert_eq!(state.selected_index, 2);
        // Wrap back to first after the last item.
        state.move_down();
        assert_eq!(state.selected_index, 0);
        state.move_up();
        assert_eq!(state.selected_index, 2);
        state.move_up();
        assert_eq!(state.selected_index, 1);
        state.move_up();
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn selected_returns_correct_item() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        assert_eq!(state.selected().unwrap().id, "anthropic");
        state.move_down();
        assert_eq!(state.selected().unwrap().id, "openai");
        state.move_down();
        assert_eq!(state.selected().unwrap().id, "ollama");
    }

    #[test]
    fn filter_reduces_results() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        state.filter_push('l');
        state.filter_push('o');
        state.filter_push('c');
        state.filter_push('a');
        state.filter_push('l');
        // Only "Ollama" matches "local"
        assert_eq!(state.selected().unwrap().id, "ollama");
    }

    #[test]
    fn filter_pop_restores() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        state.filter_push('z');
        state.filter_push('z');
        assert!(state.selected().is_none());
        state.filter_pop();
        state.filter_pop();
        assert_eq!(state.selected().unwrap().id, "anthropic");
    }

    #[test]
    fn page_up_and_down() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        state.page_down();
        assert_eq!(state.selected_index, 2); // clamped to last
        state.page_up();
        assert_eq!(state.selected_index, 0);
    }

    // -----------------------------------------------------------------------
    // DialogBehavior pipeline tests
    // -----------------------------------------------------------------------

    #[test]
    fn pipeline_esc_cancels_and_closes() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        let out = state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(out, DialogOutcome::Cancelled);
        assert!(!state.is_visible());
    }

    #[test]
    fn pipeline_enter_confirms_and_take_selected() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let out = state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(out, DialogOutcome::Confirmed);
        assert!(!state.is_visible());
        let selected = state.take_selected().unwrap();
        assert_eq!(selected.id, "openai");
    }

    #[test]
    fn pipeline_filter_typing_and_backspace() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        for c in "local".chars() {
            state.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(state.selected().unwrap().id, "ollama");
        state.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert!(state.filter.ends_with('a'));
    }

    #[test]
    fn pipeline_navigation_keys_move_selection() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        state.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(state.selected().unwrap().id, "ollama");
        state.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(state.selected_index, 1);
        state.handle_key(KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn pipeline_modal_swallows_unknown_keys() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        let out = state.handle_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
        assert_eq!(out, DialogOutcome::Handled);
        assert!(state.is_visible());
    }

    #[test]
    fn pipeline_invisible_ignores_input() {
        let mut state = DialogSelectState::new("Test", sample_items());
        let out = state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(out, DialogOutcome::Ignored);
    }

    #[test]
    fn home_and_end_jump_to_edges() {
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        state.move_end();
        assert_eq!(state.selected().unwrap().id, "ollama");
        state.move_home();
        assert_eq!(state.selected().unwrap().id, "anthropic");
    }

    #[test]
    fn render_does_not_panic() {
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        let mut state = DialogSelectState::new("Test", sample_items());
        state.open();
        terminal
            .draw(|frame| {
                state.render(frame, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn render_keeps_short_list_items_visible_when_selection_moves_down() {
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        let items = vec![
            SelectItem {
                id: "claude-md".into(),
                title: "CLAUDE.md".into(),
                description: "Import ~/.claude/CLAUDE.md".into(),
                category: "Import".into(),
                badge: None,
            },
            SelectItem {
                id: "settings".into(),
                title: "settings.json".into(),
                description: "Import ~/.claude/settings.json".into(),
                category: "Import".into(),
                badge: None,
            },
            SelectItem {
                id: "both".into(),
                title: "Both".into(),
                description: "Import both CLAUDE.md and settings.json".into(),
                category: "Import".into(),
                badge: Some("SAFE".into()),
            },
        ];
        let mut state = DialogSelectState::new("Import config", items);
        state.open();
        state.move_down();
        state.move_down();

        terminal
            .draw(|frame| {
                state.render(frame, frame.area());
            })
            .unwrap();

        let visible_items = state
            .row_to_item
            .borrow()
            .iter()
            .map(|(_, idx)| *idx)
            .collect::<Vec<_>>();
        assert_eq!(visible_items, vec![0, 1, 2]);
    }

    #[test]
    fn render_noop_when_hidden() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let state = DialogSelectState::new("Test", sample_items());
        let before = terminal.backend().buffer().clone();
        terminal
            .draw(|frame| {
                state.render(frame, frame.area());
            })
            .unwrap();
        assert_eq!(terminal.backend().buffer().content(), before.content());
    }
}
