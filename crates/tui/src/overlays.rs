// overlays.rs — Shared modal chrome plus the help overlay:
//   - centered_rect / ModalLayout / dark overlay / modal title / search line
//   - HelpOverlay (? / F1 / /help)
//
// Every other overlay and dialog lives in `dialogs/` on the shared
// `DialogCore` + `DialogBehavior` base.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

pub const CLAURST_ACCENT: Color = Color::Rgb(233, 30, 99);
pub const CLAURST_PANEL_BG: Color = Color::Rgb(20, 20, 28);
pub const CLAURST_PANEL_BORDER: Color = Color::Rgb(72, 72, 80);
pub const CLAURST_TEXT: Color = Color::Rgb(235, 235, 240);
pub const CLAURST_MUTED: Color = Color::Rgb(110, 110, 118);

// ---------------------------------------------------------------------------
// Geometry helper (shared)
// ---------------------------------------------------------------------------

/// Compute a centred `Rect` of the given `width` × `height` inside `area`.
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

// ---------------------------------------------------------------------------
// Reusable overlay helpers (shared by all dialog renderers)
// ---------------------------------------------------------------------------

/// Darken the entire screen with a semi-transparent overlay.
/// Call this BEFORE rendering any dialog content.
pub fn render_dark_overlay(frame: &mut Frame, area: Rect) {
    render_dark_overlay_buf(frame.buffer_mut(), area);
}

pub fn render_dark_overlay_buf(buf: &mut Buffer, area: Rect) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                // cell.set_bg(CLAURST_OVERLAY_BG);
                cell.set_fg(CLAURST_MUTED);
            }
        }
    }
}

/// Fill a rectangle with the standard dialog background color (no border).
pub fn render_dialog_bg(frame: &mut Frame, area: Rect) {
    render_dialog_bg_buf(frame.buffer_mut(), area);
}

pub fn render_dialog_bg_buf(buf: &mut Buffer, area: Rect) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(' ');
                cell.set_bg(CLAURST_PANEL_BG);
                cell.set_fg(CLAURST_TEXT);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ModalLayout {
    pub dialog_area: Rect,
    pub inner_area: Rect,
    pub header_area: Rect,
    pub body_area: Rect,
    pub footer_area: Rect,
}

fn compute_modal_layout(
    area: Rect,
    width: u16,
    height: u16,
    header_height: u16,
    footer_height: u16,
) -> ModalLayout {
    let dialog_width = width.min(area.width.saturating_sub(4)).max(8);
    let dialog_height = height.min(area.height.saturating_sub(4)).max(6);
    let dialog_area = centered_rect(dialog_width, dialog_height, area);
    let inner_area = Rect {
        x: dialog_area.x + 1,
        y: dialog_area.y + 1,
        width: dialog_area.width.saturating_sub(2),
        height: dialog_area.height.saturating_sub(2),
    };
    let header_h = header_height.min(inner_area.height);
    let footer_h = footer_height.min(inner_area.height.saturating_sub(header_h));
    let body_area = Rect {
        x: inner_area.x,
        y: inner_area.y.saturating_add(header_h),
        width: inner_area.width,
        height: inner_area.height.saturating_sub(header_h + footer_h),
    };
    ModalLayout {
        dialog_area,
        inner_area,
        header_area: Rect {
            x: inner_area.x,
            y: inner_area.y,
            width: inner_area.width,
            height: header_h,
        },
        body_area,
        footer_area: Rect {
            x: inner_area.x,
            y: inner_area.y + inner_area.height.saturating_sub(footer_h),
            width: inner_area.width,
            height: footer_h,
        },
    }
}

pub fn begin_modal_frame(
    frame: &mut Frame,
    area: Rect,
    width: u16,
    height: u16,
    header_height: u16,
    footer_height: u16,
) -> ModalLayout {
    let layout = compute_modal_layout(area, width, height, header_height, footer_height);
    render_dark_overlay(frame, area);
    frame.render_widget(Clear, layout.dialog_area);
    render_dialog_bg(frame, layout.dialog_area);
    layout
}

pub fn begin_modal_buf(
    buf: &mut Buffer,
    area: Rect,
    width: u16,
    height: u16,
    header_height: u16,
    footer_height: u16,
) -> ModalLayout {
    let layout = compute_modal_layout(area, width, height, header_height, footer_height);
    render_dark_overlay_buf(buf, area);
    Clear.render(layout.dialog_area, buf);
    render_dialog_bg_buf(buf, layout.dialog_area);
    layout
}

pub fn modal_title_line(title: &str, right_hint: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {}", title),
            Style::default().fg(CLAURST_TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", right_hint),
            Style::default().fg(CLAURST_MUTED),
        ),
    ])
}

pub fn render_modal_title_frame(frame: &mut Frame, area: Rect, title: &str, right_hint: &str) {
    if area.height == 0 {
        return;
    }
    let title_width = UnicodeWidthStr::width(title);
    let hint_width = UnicodeWidthStr::width(right_hint);
    let padding = area
        .width
        .saturating_sub((title_width + hint_width + 3) as u16) as usize;
    let line = Line::from(vec![
        Span::styled(
            format!(" {}", title),
            Style::default().fg(CLAURST_TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ".repeat(padding), Style::default().fg(CLAURST_TEXT)),
        Span::styled(
            right_hint.to_string(),
            Style::default().fg(CLAURST_MUTED),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), Rect { x: area.x, y: area.y, width: area.width, height: 1 });
}

pub fn render_modal_title_buf(buf: &mut Buffer, area: Rect, title: &str, right_hint: &str) {
    if area.height == 0 {
        return;
    }
    let title_width = UnicodeWidthStr::width(title);
    let hint_width = UnicodeWidthStr::width(right_hint);
    let padding = area
        .width
        .saturating_sub((title_width + hint_width + 3) as u16) as usize;
    let line = Line::from(vec![
        Span::styled(
            format!(" {}", title),
            Style::default().fg(CLAURST_TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ".repeat(padding), Style::default().fg(CLAURST_TEXT)),
        Span::styled(
            right_hint.to_string(),
            Style::default().fg(CLAURST_MUTED),
        ),
    ]);
    Paragraph::new(line).render(
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
        buf,
    );
}

pub fn modal_header_line_area(header_area: Rect, row: u16) -> Option<Rect> {
    if header_area.height <= row {
        return None;
    }
    Some(Rect {
        x: header_area.x,
        y: header_area.y + row,
        width: header_area.width,
        height: 1,
    })
}

pub fn modal_search_line(
    query: &str,
    placeholder: &str,
    placeholder_color: Color,
    query_color: Color,
) -> Line<'static> {
    if query.is_empty() {
        let mut chars = placeholder.chars();
        let first = chars.next().unwrap_or(' ');
        let rest: String = chars.collect();
        Line::from(vec![
            Span::styled(" ", Style::default().fg(placeholder_color)),
            Span::styled(
                first.to_string(),
                Style::default()
                    .fg(placeholder_color)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Span::styled(rest, Style::default().fg(placeholder_color)),
        ])
    } else {
        Line::from(vec![Span::styled(
            format!(" {}", query),
            Style::default().fg(query_color),
        )])
    }
}

// ============================================================================
// HelpOverlay
// ============================================================================

/// State for the full-screen help overlay (? / F1 / /help).
#[derive(Debug, Default)]
pub struct HelpOverlay {
    pub visible: bool,
    pub scroll_offset: u16,
    /// Live search filter — only commands matching this substring are shown.
    pub filter: String,
    /// Dynamically populated entries from the command registry.
    pub commands: Vec<HelpEntry>,
}

/// A single command entry shown in the help overlay.
#[derive(Debug, Clone)]
pub struct HelpEntry {
    pub name: String,
    /// Comma-separated aliases, e.g. "h, ?"
    pub aliases: String,
    pub description: String,
    pub category: String,
}

impl HelpOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Populate (or replace) the command entries from the command registry.
    /// Entries are sorted by category then name.
    pub fn populate_from_commands(&mut self, entries: Vec<HelpEntry>) {
        self.commands = entries;
        // Sort stable by category, then name for consistent display.
        self.commands.sort_by(|a, b| {
            a.category.cmp(&b.category).then(a.name.cmp(&b.name))
        });
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if !self.visible {
            // Reset state when closing
            self.scroll_offset = 0;
            self.filter.clear();
        }
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.scroll_offset = 0;
        self.filter.clear();
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    pub fn scroll_down(&mut self, max: u16) {
        if self.scroll_offset + 1 < max {
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

/// Render the help overlay into the frame.
pub fn render_help_overlay(frame: &mut Frame, overlay: &HelpOverlay, area: Rect) {
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::widgets::Wrap;
    use claurst_core::constants::APP_VERSION;

    if !overlay.visible {
        return;
    }

    let layout = begin_modal_frame(frame, area, 100, 36, 3, 1);
    render_modal_title_frame(frame, layout.header_area, "Shortcuts & commands", "esc");
    let search_line = modal_search_line(
        &overlay.filter,
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
        .constraints([Constraint::Percentage(42), Constraint::Length(1), Constraint::Min(1)])
        .split(content_area);

    // ─── Left column: keyboard shortcuts by category ───────────────────────
    let mut left_lines: Vec<Line<'static>> = Vec::new();

    left_lines.push(Line::from(Span::styled(
        " Keyboard Shortcuts",
        Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD),
    )));
    left_lines.push(Line::from(""));

    // Navigation category
    left_lines.push(Line::from(Span::styled(
        " Navigation",
        Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD),
    )));
    for (key, desc) in &[
        ("PageUp / PgDn",   "Scroll messages"),
        ("j / k",           "Scroll one line"),
        ("Home / End",      "Top / bottom"),
    ] {
        left_lines.push(kb_line(key, desc));
    }
    left_lines.push(Line::from(""));

    // Input category
    left_lines.push(Line::from(Span::styled(
        " Input",
        Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD),
    )));
    for (key, desc) in &[
        ("Enter",           "Submit message"),
        ("Up / Down",       "Input history"),
        ("Ctrl+R",          "Search history"),
        ("Alt+E",           "Expand pasted text"),
        ("Esc",             "Cancel / close"),
    ] {
        left_lines.push(kb_line(key, desc));
    }
    left_lines.push(Line::from(""));

    // App category
    left_lines.push(Line::from(Span::styled(
        " App",
        Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD),
    )));
    for (key, desc) in &[
        ("F1 / ?",          "Toggle help"),
        ("Ctrl+Shift+A",    "Model picker"),
        ("Ctrl+K",          "Command palette"),
        ("Ctrl+C",          "Cancel / quit"),
        ("Ctrl+D",          "Quit (empty input)"),
        ("Ctrl+L",          "Clear screen"),
        ("t",               "Expand/collapse thinking"),
    ] {
        left_lines.push(kb_line(key, desc));
    }

    frame.render_widget(
        Paragraph::new(left_lines)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(CLAURST_PANEL_BG)),
        col_chunks[0],
    );

    // ─── Center divider ────────────────────────────────────────────────────
    let divider_lines: Vec<Line<'static>> = (0..content_area.height)
        .map(|_| Line::from(Span::styled("\u{2502}", Style::default().fg(CLAURST_MUTED))))
        .collect();
    frame.render_widget(Paragraph::new(divider_lines), col_chunks[1]);

    // ─── Right column: slash commands by category ──────────────────────────
    let filter_lc = overlay.filter.to_lowercase();
    let filtered: Vec<&HelpEntry> = overlay
        .commands
        .iter()
        .filter(|e| {
            filter_lc.is_empty()
                || e.name.to_lowercase().contains(filter_lc.as_str())
                || e.aliases.to_lowercase().contains(filter_lc.as_str())
                || e.description.to_lowercase().contains(filter_lc.as_str())
        })
        .collect();

    let mut right_lines: Vec<Line<'static>> = Vec::new();

    right_lines.push(Line::from(Span::styled(
        " Slash Commands",
        Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD),
    )));
    right_lines.push(Line::from(""));

    let mut current_cat = "";
    for entry in &filtered {
        if entry.category.as_str() != current_cat {
            current_cat = entry.category.as_str();
            if right_lines.len() > 2 {
                right_lines.push(Line::from(""));
            }
            right_lines.push(Line::from(Span::styled(
                format!(" {}", entry.category),
                Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD),
            )));
        }
        let aliases_text = if entry.aliases.is_empty() {
            String::new()
        } else {
            format!(" ({})", entry.aliases)
        };
        right_lines.push(Line::from(vec![
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
        right_lines.push(Line::from(Span::styled(
            " No matching commands",
            Style::default().fg(CLAURST_MUTED),
        )));
    }

    let right_total = right_lines.len() as u16;
    let right_visible = col_chunks[2].height;
    let max_scroll = right_total.saturating_sub(right_visible);
    let scroll = overlay.scroll_offset.min(max_scroll);

    frame.render_widget(
        Paragraph::new(right_lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .style(Style::default().bg(CLAURST_PANEL_BG)),
        col_chunks[2],
    );

    let version_line = Line::from(vec![
        Span::styled(
            format!(
                " v{}  ·  type to filter  ·  ↑↓ scroll commands  ·  esc close",
                APP_VERSION
            ),
            Style::default()
                .fg(CLAURST_MUTED)
                .add_modifier(Modifier::ITALIC),
        ),
    ]);
    frame.render_widget(Paragraph::new(version_line), layout.footer_area);
}

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

fn kb_line<'a>(key: &str, desc: &str) -> Line<'a> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{:<20}", key),
            Style::default()
                .fg(CLAURST_TEXT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc.to_string(), Style::default().fg(CLAURST_MUTED)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- HelpOverlay ---------------------------------------------------

    #[test]
    fn help_overlay_toggle() {
        let mut h = HelpOverlay::new();
        assert!(!h.visible);
        h.toggle();
        assert!(h.visible);
        h.toggle();
        assert!(!h.visible);
    }

    #[test]
    fn help_overlay_close_resets_state() {
        let mut h = HelpOverlay::new();
        h.visible = true;
        h.scroll_offset = 5;
        h.filter = "foo".to_string();
        h.close();
        assert!(!h.visible);
        assert_eq!(h.scroll_offset, 0);
        assert!(h.filter.is_empty());
    }

    #[test]
    fn help_overlay_filter() {
        let mut h = HelpOverlay::new();
        h.push_filter_char('h');
        h.push_filter_char('e');
        assert_eq!(h.filter, "he");
        h.pop_filter_char();
        assert_eq!(h.filter, "h");
    }

    #[test]
    fn modal_search_line_separates_leading_space_from_cursor() {
        let line = modal_search_line("", "Search", CLAURST_MUTED, CLAURST_TEXT);
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].content.as_ref(), " ");
        assert_eq!(line.spans[1].content.as_ref(), "S");
        assert_eq!(line.spans[2].content.as_ref(), "earch");
    }
}
