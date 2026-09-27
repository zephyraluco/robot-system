// overlays.rs — Shared modal chrome: centered_rect / ModalLayout / dark
// overlay / modal title / search line.
//
// Every overlay and dialog — including the help dialog (? / F1 / /help, see
// `dialogs::help_dialog`) — lives in `dialogs/` on the shared `DialogCore` +
// `DialogBehavior` base and uses the helpers below.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

pub const CLAURST_ACCENT: Color = Color::Rgb(233, 30, 99);
pub const CLAURST_PANEL_BG: Color = Color::Rgb(20, 20, 28);
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

// ---------------------------------------------------------------------------
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_search_line_separates_leading_space_from_cursor() {
        let line = modal_search_line("", "Search", CLAURST_MUTED, CLAURST_TEXT);
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].content.as_ref(), " ");
        assert_eq!(line.spans[1].content.as_ref(), "S");
        assert_eq!(line.spans[2].content.as_ref(), "earch");
    }
}
