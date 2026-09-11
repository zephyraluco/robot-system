// theme_screen.rs — Theme picker overlay opened by /theme.
//
// Shows a list of available themes with colour swatches. Arrow keys navigate,
// Enter selects, Esc cancels.
//
// Built on the shared `DialogCore` + `DialogBehavior` base
// (`crate::dialogs::dialog`): the embedded `DialogCore` owns visibility and
// geometry, and the dispatch pipeline captures every keyboard and mouse event
// while the picker is open (so input never leaks to the transcript).

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::dialogs::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::overlays::{
    modal_header_line_area, ModalLayout, CLAURST_ACCENT, CLAURST_MUTED, CLAURST_PANEL_BG,
    CLAURST_TEXT,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single theme option shown in the picker.
#[derive(Debug, Clone)]
pub struct ThemeOption {
    pub name: String,
    pub label: String,
    pub description: String,
    /// A few representative colours used for the swatch preview.
    pub swatch: [Color; 4],
}

pub struct ThemeScreen {
    /// Shared dialog base: visibility, geometry and the key/mouse capture
    /// pipeline (modal — swallows every event while the picker is open).
    pub core: DialogCore,
    pub themes: Vec<ThemeOption>,
    pub selected_idx: usize,
}

impl ThemeScreen {
    pub fn new() -> Self {
        let themes = builtin_themes();
        let mut core = DialogCore::new("Choose a theme", 70, 13)
            .header_height(2)
            .footer_height(1)
            .dismiss_on_outside_click();
        core.set_size(70, Self::height_for(themes.len()));
        Self {
            core,
            themes,
            selected_idx: 0,
        }
    }

    /// Height that fits the whole list: every theme row is followed by a spacer
    /// row, plus two header rows, one footer row and the two border rows.
    fn height_for(theme_count: usize) -> u16 {
        (theme_count as u16).saturating_mul(2) + 5
    }

    /// Whether the picker is currently shown.
    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    pub fn open(&mut self, current_theme: &str) {
        self.core.open();
        // Select the current theme, if found
        if let Some(idx) = self.themes.iter().position(|t| t.name == current_theme) {
            self.selected_idx = idx;
        } else {
            self.selected_idx = 0;
        }
    }

    pub fn close(&mut self) {
        self.core.close();
    }

    pub fn select_prev(&mut self) {
        let count = self.themes.len();
        if count == 0 {
            return;
        }
        if self.selected_idx == 0 {
            self.selected_idx = count - 1;
        } else {
            self.selected_idx -= 1;
        }
    }

    pub fn select_next(&mut self) {
        let count = self.themes.len();
        if count == 0 {
            return;
        }
        self.selected_idx = (self.selected_idx + 1) % count;
    }

    /// Return the name of the currently selected theme.
    pub fn selected_name(&self) -> Option<&str> {
        self.themes.get(self.selected_idx).map(|t| t.name.as_str())
    }
}

impl Default for ThemeScreen {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Built-in themes
// ---------------------------------------------------------------------------

fn builtin_themes() -> Vec<ThemeOption> {
    vec![
        ThemeOption {
            name: "default".to_string(),
            label: "Default".to_string(),
            description: "Claurst default — dark background, cyan accents".to_string(),
            swatch: [Color::Black, Color::Cyan, Color::Green, Color::White],
        },
        ThemeOption {
            name: "dark".to_string(),
            label: "Dark".to_string(),
            description: "High-contrast dark theme".to_string(),
            swatch: [
                Color::Rgb(18, 18, 18),
                Color::Rgb(97, 175, 239),
                Color::Rgb(152, 195, 121),
                Color::Rgb(229, 229, 229),
            ],
        },
        ThemeOption {
            name: "light".to_string(),
            label: "Light".to_string(),
            description: "Light background with dark text".to_string(),
            swatch: [
                Color::White,
                Color::Blue,
                Color::DarkGray,
                Color::Black,
            ],
        },
        ThemeOption {
            name: "solarized".to_string(),
            label: "Solarized".to_string(),
            description: "Solarized Dark — warm tones with blue accents".to_string(),
            swatch: [
                Color::Rgb(0, 43, 54),
                Color::Rgb(38, 139, 210),
                Color::Rgb(133, 153, 0),
                Color::Rgb(131, 148, 150),
            ],
        },
        ThemeOption {
            name: "nord".to_string(),
            label: "Nord".to_string(),
            description: "Nord — cool blue-grey palette".to_string(),
            swatch: [
                Color::Rgb(46, 52, 64),
                Color::Rgb(136, 192, 208),
                Color::Rgb(163, 190, 140),
                Color::Rgb(216, 222, 233),
            ],
        },
        ThemeOption {
            name: "dracula".to_string(),
            label: "Dracula".to_string(),
            description: "Dracula — purple/pink dark theme".to_string(),
            swatch: [
                Color::Rgb(40, 42, 54),
                Color::Rgb(139, 233, 253),
                Color::Rgb(80, 250, 123),
                Color::Rgb(248, 248, 242),
            ],
        },
        ThemeOption {
            name: "monokai".to_string(),
            label: "Monokai".to_string(),
            description: "Monokai — vibrant colours on dark background".to_string(),
            swatch: [
                Color::Rgb(39, 40, 34),
                Color::Rgb(102, 217, 239),
                Color::Rgb(166, 226, 46),
                Color::Rgb(248, 248, 242),
            ],
        },
        ThemeOption {
            name: "deuteranopia".to_string(),
            label: "Deuteranopia".to_string(),
            description: "Red-green color blind friendly — blue/yellow/gray palette".to_string(),
            swatch: [
                Color::Rgb(18, 18, 18),
                Color::Rgb(0, 122, 204),  // Blue
                Color::Rgb(255, 180, 0),  // Gold/Yellow
                Color::Rgb(200, 200, 200), // Light gray
            ],
        },
    ]
}

// ---------------------------------------------------------------------------
// DialogBehavior — unified key/mouse capture pipeline
// ---------------------------------------------------------------------------

impl DialogBehavior for ThemeScreen {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        // Esc is consumed by the dispatch pipeline (→ Cancelled).
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.select_prev();
                DialogOutcome::Handled
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.select_next();
                DialogOutcome::Handled
            }
            KeyCode::Enter => DialogOutcome::Confirmed,
            _ => DialogOutcome::Ignored,
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) -> DialogOutcome {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.select_prev();
                DialogOutcome::Handled
            }
            MouseEventKind::ScrollDown => {
                self.select_next();
                DialogOutcome::Handled
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // Click-to-select: rows alternate (entry, spacer, entry, …).
                let body = self.core.layout().body_area;
                if body.height > 0
                    && mouse.row >= body.y
                    && mouse.row < body.y.saturating_add(body.height)
                {
                    let idx = (mouse.row - body.y) as usize / 2;
                    if idx < self.themes.len() {
                        self.selected_idx = idx;
                    }
                }
                DialogOutcome::Handled
            }
            _ => DialogOutcome::Ignored,
        }
    }

    fn render_content(&self, frame: &mut Frame, layout: &ModalLayout) {
        // `esc` hint, right-aligned on the title row drawn by the shared `render`.
        if layout.header_area.height > 0 && layout.header_area.width > 4 {
            let esc_area = Rect {
                height: 1,
                ..layout.header_area
            };
            frame.render_widget(
                Paragraph::new("esc ")
                    .alignment(Alignment::Right)
                    .style(Style::default().fg(CLAURST_MUTED)),
                esc_area,
            );
        }
        if let Some(subtitle_area) = modal_header_line_area(layout.header_area, 1) {
            frame.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    " Preview palettes before wiring up richer theme behavior.",
                    Style::default().fg(CLAURST_MUTED),
                )])),
                subtitle_area,
            );
        }

        let mut lines: Vec<Line> = Vec::new();

        for (i, theme) in self.themes.iter().enumerate() {
            let is_selected = i == self.selected_idx;
            let bg = if is_selected { CLAURST_ACCENT } else { CLAURST_PANEL_BG };
            let fg = if is_selected { Color::White } else { CLAURST_TEXT };
            let desc_fg = if is_selected { Color::Rgb(248, 220, 236) } else { CLAURST_MUTED };

            // Build the swatch using block characters with background colour
            let swatch_spans: Vec<Span> = theme
                .swatch
                .iter()
                .map(|&c| Span::styled("  ", Style::default().bg(c)))
                .collect();

            let mut row_spans: Vec<Span> = Vec::new();
            row_spans.push(Span::styled(" ", Style::default().bg(bg)));
            row_spans.extend(swatch_spans);
            row_spans.push(Span::styled("  ", Style::default().bg(bg)));
            row_spans.push(Span::styled(
                format!("{:<12}", theme.label),
                Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
            ));
            row_spans.push(Span::styled(
                theme.description.clone(),
                Style::default().fg(desc_fg).bg(bg),
            ));
            let used: usize = row_spans.iter().map(|span| span.content.len()).sum();
            let pad = layout.body_area.width.saturating_sub(used as u16) as usize;
            if pad > 0 {
                row_spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
            }

            lines.push(Line::from(row_spans));
            lines.push(Line::from(""));
        }
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(CLAURST_PANEL_BG)),
            layout.body_area,
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " ↑↓ navigate  ·  enter apply  ·  esc cancel",
                Style::default().fg(CLAURST_MUTED).add_modifier(Modifier::ITALIC),
            )])),
            layout.footer_area,
        );
    }
}

// ---------------------------------------------------------------------------
// Key handling helpers (called from app.rs)
// ---------------------------------------------------------------------------

/// Adapter: drive one key event through the picker's `DialogBehavior`
/// pipeline and return the chosen theme name when the user confirms.
pub fn handle_theme_key(screen: &mut ThemeScreen, key: KeyEvent) -> Option<String> {
    let out = screen.handle_key(key);
    if out.is_confirmed() {
        let name = screen.selected_name().map(String::from);
        screen.close();
        name
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn theme_screen_renders_current_theme() {
        let mut screen = ThemeScreen::new();
        screen.open("dark");

        let backend = TestBackend::new(90, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area()))
            .unwrap();

        let rendered = terminal.backend().buffer();
        let content = rendered
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(content.contains("Choose a theme"));
        assert!(content.contains("Dark"));
    }

    #[test]
    fn theme_navigation_wraps() {
        let mut screen = ThemeScreen::new();
        screen.open("default");

        screen.select_prev();
        assert_eq!(screen.selected_name(), Some("deuteranopia"));

        screen.select_next();
        assert_eq!(screen.selected_name(), Some("default"));
    }

    /// An opened picker with a synthetic layout recorded, so mouse hit-testing
    /// behaves as if it had been rendered once.
    fn opened() -> ThemeScreen {
        let mut screen = ThemeScreen::new();
        screen.open("default");
        screen.core.set_layout(ModalLayout {
            dialog_area: Rect::new(0, 0, 70, 21),
            inner_area: Rect::new(1, 1, 68, 19),
            header_area: Rect::new(1, 1, 68, 2),
            body_area: Rect::new(1, 3, 68, 16),
            footer_area: Rect::new(1, 19, 68, 1),
        });
        screen
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn invisible_picker_ignores_keys() {
        let mut screen = ThemeScreen::new();
        assert!(!screen.is_visible());
        assert!(!screen.handle_key(key(KeyCode::Enter)).is_handled());
    }

    #[test]
    fn esc_closes_the_picker() {
        let mut screen = opened();
        let out = screen.handle_key(key(KeyCode::Esc));
        assert!(out.is_cancelled());
        assert!(!screen.is_visible());
    }

    #[test]
    fn enter_confirms_and_adapter_returns_the_theme_name() {
        let mut screen = opened();
        screen.selected_idx = 1; // "dark"
        let out = screen.handle_key(key(KeyCode::Enter));
        assert!(out.is_confirmed());

        // `handle_theme_key` closes the picker and yields the name.
        let mut screen = opened();
        screen.selected_idx = 1;
        assert_eq!(handle_theme_key(&mut screen, key(KeyCode::Enter)).as_deref(), Some("dark"));
        assert!(!screen.is_visible());

        // Esc yields no name.
        let mut screen = opened();
        assert_eq!(handle_theme_key(&mut screen, key(KeyCode::Esc)), None);
        assert!(!screen.is_visible());
    }

    #[test]
    fn arrow_keys_move_the_selection() {
        let mut screen = opened();
        screen.handle_key(key(KeyCode::Down));
        assert_eq!(screen.selected_idx, 1);
        screen.handle_key(key(KeyCode::Up));
        assert_eq!(screen.selected_idx, 0);
    }

    #[test]
    fn mouse_wheel_and_click_select_a_theme() {
        let mut screen = opened();

        assert!(screen
            .handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 10,
                row: 5,
                modifiers: crossterm::event::KeyModifiers::NONE,
            })
            .is_handled());
        assert_eq!(screen.selected_idx, 1);

        // Click-to-select: body starts at row 3 and rows alternate
        // (entry, spacer), so row 5 maps to theme index 1.
        screen.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert_eq!(screen.selected_idx, 1);
    }

    #[test]
    fn mask_click_closes_the_picker() {
        let mut screen = opened();
        let out = screen.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 200,
            row: 200,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert!(out.is_cancelled());
        assert!(!screen.is_visible());
    }

    #[test]
    fn dialog_is_tall_enough_for_every_theme() {
        // Regression: the picker used to be sized for one row per theme even
        // though each entry is followed by a spacer, so the last themes were
        // clipped and unreachable.
        let screen = ThemeScreen::new();
        let height = ThemeScreen::height_for(screen.themes.len());
        assert_eq!(height, screen.themes.len() as u16 * 2 + 5);
        // 2 border + 2 header + 1 footer rows leave len*2 body rows.
        assert_eq!(height - 5, screen.themes.len() as u16 * 2);
    }
}
