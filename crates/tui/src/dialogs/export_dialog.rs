// export_dialog.rs — Format picker dialog for /export command.
//
// Shows a two-option dialog (JSON | Markdown). On confirm, caller writes the file.
// Built on the shared `DialogCore` + `DialogBehavior` base (crate::dialogs::dialog).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::dialogs::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::overlays::{ModalLayout, CLAURST_ACCENT, CLAURST_MUTED, CLAURST_PANEL_BG, CLAURST_TEXT};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportFormat {
    #[default]
    Json,
    Markdown,
}

#[derive(Debug, Clone)]
pub struct ExportDialogState {
    /// Embedded generic dialog base (visibility, geometry, title).
    pub core: DialogCore,
    pub selected: ExportFormat,
}

impl Default for ExportDialogState {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportDialogState {
    pub fn new() -> Self {
        Self {
            core: DialogCore::new("Export conversation", 62, 14)
                .header_height(2)
                .footer_height(1),
            selected: ExportFormat::default(),
        }
    }

    pub fn open(&mut self) {
        self.core.open();
        self.selected = ExportFormat::default();
    }

    pub fn dismiss(&mut self) {
        self.core.close();
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    pub fn toggle(&mut self) {
        self.selected = match self.selected {
            ExportFormat::Json => ExportFormat::Markdown,
            ExportFormat::Markdown => ExportFormat::Json,
        };
    }
}

impl DialogBehavior for ExportDialogState {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        match key.code {
            KeyCode::Enter => {
                self.core.close();
                DialogOutcome::Confirmed
            }
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                self.toggle();
                DialogOutcome::Handled
            }
            KeyCode::Char('1') => {
                self.selected = ExportFormat::Json;
                DialogOutcome::Handled
            }
            KeyCode::Char('2') => {
                self.selected = ExportFormat::Markdown;
                DialogOutcome::Handled
            }
            _ => DialogOutcome::Ignored,
        }
    }

    fn render_content(&self, frame: &mut Frame, layout: &ModalLayout) {
        // Subtitle on the second header row (the title bar itself is drawn by
        // the `DialogBehavior::render` pipeline).
        if layout.header_area.height > 1 {
            let subtitle_area = Rect {
                x: layout.header_area.x,
                y: layout.header_area.y + 1,
                width: layout.header_area.width,
                height: layout.header_area.height - 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    " Choose a format to export this session.",
                    Style::default().fg(CLAURST_MUTED),
                )])),
                subtitle_area,
            );
        }

        let lines: Vec<Line<'static>> = vec![
            Line::from(""),
            export_option_row(
                "1",
                "JSON",
                "Structured export for tooling and replay",
                self.selected == ExportFormat::Json,
                layout.body_area.width,
            ),
            Line::from(""),
            export_option_row(
                "2",
                "Markdown",
                "Readable transcript for docs and sharing",
                self.selected == ExportFormat::Markdown,
                layout.body_area.width,
            ),
            Line::from(""),
            Line::from(vec![Span::styled(
                " Saved to ./claude-export-<timestamp>.<ext>",
                Style::default().fg(CLAURST_MUTED),
            )]),
        ];

        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .style(Style::default().bg(CLAURST_PANEL_BG)),
            layout.body_area,
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " tab/←/→ switch  ·  enter export  ·  1/2 choose",
                Style::default().fg(CLAURST_MUTED).add_modifier(Modifier::ITALIC),
            )])),
            layout.footer_area,
        );
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

fn export_option_row(
    key: &str,
    label: &str,
    description: &str,
    selected: bool,
    width: u16,
) -> Line<'static> {
    let bg = if selected { CLAURST_ACCENT } else { CLAURST_PANEL_BG };
    let fg = if selected { Color::White } else { CLAURST_TEXT };
    let desc_fg = if selected { Color::Rgb(245, 220, 232) } else { CLAURST_MUTED };
    let mut spans = vec![
        Span::styled(format!(" [{}] ", key), Style::default().fg(desc_fg).bg(bg)),
        Span::styled(label.to_string(), Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("  {}", description),
            Style::default().fg(desc_fg).bg(bg),
        ),
    ];
    let used: usize = spans.iter().map(|span| span.content.len()).sum();
    let pad = width.saturating_sub(used as u16) as usize;
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
    }
    Line::from(spans)
}

// ---------------------------------------------------------------------------
// Export helpers
// ---------------------------------------------------------------------------

pub fn export_as_markdown(
    messages: &[claurst_core::types::Message],
    session_title: Option<&str>,
) -> String {
    use claurst_core::types::Role;
    let mut out = String::new();
    if let Some(title) = session_title {
        out.push_str(&format!("# {}\n\n", title));
    } else {
        out.push_str("# Claurst Conversation Export\n\n");
    }
    for msg in messages {
        let label = match msg.role {
            Role::User => "**User**",
            Role::Assistant => "**Claurst**",
        };
        let text = msg.get_all_text();
        out.push_str(&format!("{}\n\n{}\n\n---\n\n", label, text));
    }
    out
}

pub fn export_as_json(
    messages: &[claurst_core::types::Message],
    session_title: Option<&str>,
) -> serde_json::Value {
    use claurst_core::types::Role;
    let items: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": match m.role { Role::User => "user", Role::Assistant => "assistant" },
                "content": m.get_all_text(),
            })
        })
        .collect();
    serde_json::json!({
        "title": session_title,
        "messages": items,
        "exported_at": chrono::Local::now().to_rfc3339(),
    })
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
    fn export_dialog_defaults_hidden() {
        let state = ExportDialogState::new();
        assert!(!state.is_visible());
        assert_eq!(state.selected, ExportFormat::Json);
    }

    #[test]
    fn export_dialog_open() {
        let mut state = ExportDialogState::new();
        state.open();
        assert!(state.is_visible());
    }

    #[test]
    fn export_dialog_toggle() {
        let mut state = ExportDialogState::new();
        state.open();
        assert_eq!(state.selected, ExportFormat::Json);
        state.toggle();
        assert_eq!(state.selected, ExportFormat::Markdown);
        state.toggle();
        assert_eq!(state.selected, ExportFormat::Json);
    }

    #[test]
    fn export_dialog_renders_without_panic() {
        use crate::dialogs::dialog::DialogBehavior as _;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut state = ExportDialogState::new();
        state.open();
        terminal.draw(|frame| {
            state.render(frame, frame.area());
        }).unwrap();
        let content: String = terminal.backend().buffer().clone().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Export") || content.contains("JSON"));
    }

    #[test]
    fn export_dialog_hidden_renders_nothing() {
        use crate::dialogs::dialog::DialogBehavior as _;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let state = ExportDialogState::new();
        let before = terminal.backend().buffer().clone();
        terminal.draw(|frame| {
            state.render(frame, frame.area());
        }).unwrap();
        assert_eq!(terminal.backend().buffer().content(), before.content());
    }
}
