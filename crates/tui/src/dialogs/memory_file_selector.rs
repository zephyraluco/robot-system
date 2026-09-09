// memory_file_selector.rs — Memory file selector overlay mirroring TS MemoryFileSelector.tsx
// Built on the shared `DialogCore` + `DialogBehavior` base (crate::dialogs::dialog).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::dialogs::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::overlays::{ModalLayout, CLAURST_ACCENT, CLAURST_MUTED, CLAURST_PANEL_BG, CLAURST_TEXT};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryFileType {
    User,
    Project,
    Local,
}

pub struct MemoryFile {
    pub path: String,
    pub display_path: String,
    pub file_type: MemoryFileType,
    pub exists: bool,
}

pub struct MemoryFileSelectorState {
    /// Embedded generic dialog base (visibility, geometry, title).
    pub core: DialogCore,
    pub files: Vec<MemoryFile>,
    pub selected: usize,
    pub project_root: std::path::PathBuf,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl MemoryFileSelectorState {
    pub fn new() -> Self {
        Self {
            core: DialogCore::new("Memory", 70, 8),
            files: Vec::new(),
            selected: 0,
            project_root: std::path::PathBuf::new(),
        }
    }

    /// Open the selector for the given project root.
    ///
    /// Populates the file list with:
    /// - User:    `~/.claurst/AGENTS.md`
    /// - Project: `{project_root}/AGENTS.md`
    /// - Local:   `{project_root}/.claurst/AGENTS.md`
    ///
    /// Each entry is marked `exists = true/false` based on the filesystem.
    pub fn open(&mut self, project_root: &std::path::Path) {
        self.project_root = project_root.to_path_buf();
        self.selected = 0;
        self.files.clear();

        // User-level: ~/.claurst/AGENTS.md
        let user_path = claurst_core::config::Settings::config_dir().join("AGENTS.md");
        let user_display = {
            let home = dirs::home_dir().unwrap_or_default();
            let rel = user_path
                .strip_prefix(&home)
                .unwrap_or(&user_path);
            format!("~/{}", rel.display())
        };
        self.files.push(MemoryFile {
            exists: user_path.exists(),
            path: user_path.to_string_lossy().into_owned(),
            display_path: user_display,
            file_type: MemoryFileType::User,
        });

        // Project-level: {project_root}/AGENTS.md
        let project_path = project_root.join("AGENTS.md");
        let project_display = project_path.display().to_string();
        self.files.push(MemoryFile {
            exists: project_path.exists(),
            path: project_path.to_string_lossy().into_owned(),
            display_path: project_display,
            file_type: MemoryFileType::Project,
        });

        // Local-level: {project_root}/.claurst/AGENTS.md
        let local_path = project_root.join(".claurst").join("AGENTS.md");
        let local_display = local_path.display().to_string();
        self.files.push(MemoryFile {
            exists: local_path.exists(),
            path: local_path.to_string_lossy().into_owned(),
            display_path: local_display,
            file_type: MemoryFileType::Local,
        });

        // Height: 2 border + 1 blank + N files + 1 blank + 1 footer = N + 5
        self.core.set_size(70, (self.files.len() as u16 + 6).max(8));
        self.core.open();
    }

    pub fn close(&mut self) {
        self.core.close();
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    pub fn select_prev(&mut self) {
        let count = self.files.len();
        if count == 0 {
            return;
        }
        if self.selected == 0 {
            self.selected = count - 1;
        } else {
            self.selected -= 1;
        }
    }

    pub fn select_next(&mut self) {
        let count = self.files.len();
        if count == 0 {
            return;
        }
        self.selected = (self.selected + 1) % count;
    }

    /// Return the path of the currently highlighted file, if any.
    pub fn selected_path(&self) -> Option<&str> {
        self.files.get(self.selected).map(|f| f.path.as_str())
    }
}

impl Default for MemoryFileSelectorState {
    fn default() -> Self {
        Self::new()
    }
}

impl DialogBehavior for MemoryFileSelectorState {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        match key.code {
            KeyCode::Up => {
                self.select_prev();
                DialogOutcome::Handled
            }
            KeyCode::Down => {
                self.select_next();
                DialogOutcome::Handled
            }
            KeyCode::Enter => {
                // Selection acknowledged — consumer can read selected_path()
                self.close();
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

        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(vec![
            Span::styled(" Memory", Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(" — choose a file", Style::default().fg(CLAURST_MUTED)),
            Span::styled(
                format!("{:>width$}", "Esc close", width = inner.width.saturating_sub(24) as usize),
                Style::default().fg(CLAURST_MUTED),
            ),
        ]));
        lines.push(Line::from(""));

        for (i, file) in self.files.iter().enumerate() {
            let type_label = match file.file_type {
                MemoryFileType::User => "User    ",
                MemoryFileType::Project => "Project ",
                MemoryFileType::Local => "Local   ",
            };

            let new_tag = if !file.exists {
                Span::styled(" (new)", Style::default().fg(CLAURST_MUTED))
            } else {
                Span::raw("")
            };

            if i == self.selected {
                lines.push(Line::from(vec![
                    Span::styled(
                        pad_line(&format!("  \u{203a} {type_label} {}", file.display_path), inner.width),
                        Style::default()
                            .fg(Color::Black)
                            .bg(CLAURST_ACCENT)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("    {type_label} {}", file.display_path),
                        Style::default().fg(CLAURST_TEXT),
                    ),
                    new_tag,
                ]));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "  \u{2191}\u{2193} navigate  Enter select  Esc close",
            Style::default().fg(CLAURST_MUTED),
        )]));

        let para = Paragraph::new(lines)
            .style(Style::default().bg(CLAURST_PANEL_BG).fg(CLAURST_TEXT))
            .alignment(Alignment::Left);

        use ratatui::widgets::Widget;
        para.render(inner, frame.buffer_mut());
    }
}

fn pad_line(text: &str, width: u16) -> String {
    let max_width = width as usize;
    let mut clipped: String = text.chars().take(max_width).collect();
    let visible = clipped.chars().count();
    if visible < max_width {
        clipped.push_str(&" ".repeat(max_width - visible));
    }
    clipped
}
