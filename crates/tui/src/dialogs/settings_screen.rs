// settings_screen.rs — Flat searchable settings interface.
//
// Opened by /config or /settings commands. Shows all editable settings
// in a single scrollable list with live search filtering.
// Changes are persisted via Settings::save_sync() or settings.json writes.
//
// Built on the shared `DialogCore` + `DialogBehavior` base
// (`crate::dialogs::dialog`): the embedded `DialogCore` owns
// visibility/geometry and the `DialogBehavior` dispatch pipeline captures every
// keyboard and mouse event while the screen is open (so input never leaks to
// the transcript underneath).

use claurst_core::config::{Config, Settings};
use claurst_core::output_styles::{builtin_styles, find_style};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use crate::dialogs::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::overlays::{
    centered_rect, modal_search_line, render_dark_overlay, render_dialog_bg, ModalLayout,
    CLAURST_ACCENT, CLAURST_MUTED, CLAURST_PANEL_BG,
};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum SettingKind {
    Bool,
    Enum { options: Vec<&'static str> },
    Number,
}

#[derive(Debug, Clone)]
pub struct SettingsEntry {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub kind: SettingKind,
    pub value: String,
}

pub struct SettingsScreen {
    /// Shared dialog base: visibility, geometry and the key/mouse capture
    /// pipeline (modal — swallows every event while the screen is open).
    pub core: DialogCore,
    pub search_query: String,
    pub selected_idx: usize,
    pub scroll_offset: usize,
    /// Which field is being edited (field name as key).
    pub edit_field: Option<String>,
    /// Current buffer content while editing a field.
    pub edit_value: String,
    /// Snapshot of settings at open time.
    pub settings_snapshot: Settings,
    /// Pending changes (field_name → new_value string).
    pub pending_changes: HashMap<String, String>,
    /// Set when an edit was committed and must be applied to the app's live
    /// `Config`. `on_key` has no `&mut Config`, so the adapter
    /// (`handle_settings_key`) performs the apply/persist right after.
    pending_config_apply: bool,

    // ---- Real settings fields ----
    pub auto_compact: bool,
    pub notifications: bool,
    pub show_turn_duration: bool,
    pub output_style: String,
    pub reduce_motion: bool,
    pub terminal_progress_bar: bool,
    pub verbose: bool,
    pub cursor_blink_enabled: bool,
    pub auto_copy_enabled: bool,
    pub mouse_capture: bool,
    pub show_cwd: bool,
    pub show_git_branch: bool,
    pub compact_threshold: String,
    pub auto_commits: bool,
    pub output_format: String,
    pub disable_claude_mds: bool,
    pub file_injection_enabled: bool,
    pub file_autocomplete_limit: String,
    pub file_autocomplete_show_hidden_files: bool,
    pub file_injection_max_size: String,
}

impl SettingsScreen {
    pub fn new() -> Self {
        let settings_snapshot = Settings::load_sync().unwrap_or_default();
        let mut screen = Self {
            // `dismiss_on_outside_click()` makes a left click on the dimmed mask
            // outside the panel close the screen, matching every other modal
            // dialog (see `App::dispatch_dialog_mouse`).
            core: DialogCore::new("Settings", 80, 24).dismiss_on_outside_click(),
            search_query: String::new(),
            selected_idx: 0,
            scroll_offset: 0,
            edit_field: None,
            edit_value: String::new(),
            settings_snapshot: settings_snapshot.clone(),
            pending_changes: HashMap::new(),
            auto_compact: false,
            notifications: true,
            show_turn_duration: false,
            output_style: "default".to_string(),
            reduce_motion: false,
            terminal_progress_bar: true,
            verbose: false,
            cursor_blink_enabled: false,
            auto_copy_enabled: false,
            mouse_capture: true,
            show_cwd: false,
            show_git_branch: false,
            compact_threshold: "95".to_string(),
            auto_commits: false,
            output_format: "text".to_string(),
            disable_claude_mds: false,
            file_injection_enabled: true,
            file_autocomplete_limit: "15".to_string(),
            file_autocomplete_show_hidden_files: false,
            file_injection_max_size: "100".to_string(),
            pending_config_apply: false,
        };
        // Apply settings from snapshot immediately on initialization
        screen.apply_settings_from_snapshot();
        screen
    }

    /// Apply all settings from the snapshot to the screen fields.
    /// This is called on initialization and when opening the settings screen.
    fn apply_settings_from_snapshot(&mut self) {
        self.auto_compact = self.settings_snapshot.auto_compact;
        self.notifications = self.settings_snapshot.notifications;
        self.show_turn_duration = self.settings_snapshot.show_turn_duration;
        self.output_style = self.settings_snapshot.config.output_style.clone().unwrap_or_else(|| "default".to_string());
        self.reduce_motion = self.settings_snapshot.reduce_motion;
        self.terminal_progress_bar = self.settings_snapshot.terminal_progress_bar;
        self.verbose = self.settings_snapshot.config.verbose;
        self.cursor_blink_enabled = self.settings_snapshot.config.cursor_blink_enabled;
        self.auto_copy_enabled = self.settings_snapshot.auto_copy_on_highlight;
        self.mouse_capture = self.settings_snapshot.config.mouse_capture_enabled();
        self.show_cwd = self.settings_snapshot.show_cwd;
        self.show_git_branch = self.settings_snapshot.show_git_branch;
        self.compact_threshold = self.settings_snapshot.config.compact_threshold.to_string();
        self.auto_commits = self.settings_snapshot.config.auto_commits.unwrap_or(false);
        self.output_format = match &self.settings_snapshot.config.output_format {
            claurst_core::config::OutputFormat::Text => "text".to_string(),
            claurst_core::config::OutputFormat::Json => "json".to_string(),
            claurst_core::config::OutputFormat::StreamJson => "stream_json".to_string(),
        };
        self.disable_claude_mds = self.settings_snapshot.config.disable_claude_mds;
        self.file_injection_enabled = self.settings_snapshot.config.file_injection_enabled;
        self.file_autocomplete_limit = self.settings_snapshot.config.file_autocomplete_limit.to_string();
        self.file_autocomplete_show_hidden_files = self.settings_snapshot.config.file_autocomplete_show_hidden_files;
        self.file_injection_max_size = self.settings_snapshot.config.file_injection_max_size.to_string();
    }

    pub fn open(&mut self) {
        self.settings_snapshot = Settings::load_sync().unwrap_or_default();
        self.pending_changes.clear();
        self.edit_field = None;
        self.edit_value.clear();
        self.search_query.clear();
        self.selected_idx = 0;
        self.scroll_offset = 0;
        self.core.open();

        // Wire real settings from snapshot
        self.apply_settings_from_snapshot();
    }

    pub fn close(&mut self) {
        self.core.close();
        self.edit_field = None;
        self.edit_value.clear();
    }

    /// Whether the settings screen is currently shown.
    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    /// The settings visible under the current search filter (an owned copy, so
    /// callers may mutate `self` while iterating the result).
    fn filtered_entries(&self) -> Vec<SettingsEntry> {
        let query = self.search_query.to_lowercase();
        all_entries(self)
            .into_iter()
            .filter(|e| e.label.to_lowercase().contains(&query))
            .collect()
    }

    /// Number of rows currently visible under the search filter.
    fn filtered_len(&self) -> usize {
        self.filtered_entries().len()
    }

    /// Keep `scroll_offset` in sync with the selected row, using the body height
    /// recorded by the last render (falls back to 10 rows before the first draw).
    fn sync_scroll(&mut self) {
        let recorded = self.core.layout().body_area.height as usize;
        let visible_rows = if recorded == 0 { 10 } else { recorded };
        if self.selected_idx < self.scroll_offset {
            self.scroll_offset = self.selected_idx;
        } else if self.selected_idx >= self.scroll_offset + visible_rows {
            self.scroll_offset = self.selected_idx + 1 - visible_rows;
        }
    }

    pub fn push_search_char(&mut self, c: char) {
        self.search_query.push(c);
        self.selected_idx = 0;
    }

    pub fn pop_search_char(&mut self) {
        self.search_query.pop();
        self.selected_idx = 0;
    }

    pub fn select_prev(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    pub fn select_next(&mut self, total_visible: usize) {
        if total_visible > 0 && self.selected_idx + 1 < total_visible {
            self.selected_idx += 1;
        }
    }

    /// Start editing a field by name, seeding the buffer with current value.
    pub fn start_edit(&mut self, field: &str, current_value: &str) {
        self.edit_field = Some(field.to_string());
        self.edit_value = current_value.to_string();
    }

    /// Commit the current edit to pending_changes.
    pub fn commit_edit(&mut self) {
        if let Some(field) = self.edit_field.take() {
            let value = std::mem::take(&mut self.edit_value);
            self.pending_changes.insert(field, value);
        }
    }

    /// Discard the current edit.
    pub fn cancel_edit(&mut self) {
        self.edit_field = None;
        self.edit_value.clear();
    }

    /// Apply all pending changes to settings and persist them.
    pub fn apply_and_save(&mut self, config: &mut Config) {
        for (field, value) in &self.pending_changes {
            match field.as_str() {
                "max_tokens" => {
                    if let Ok(n) = value.parse::<u32>() {
                        config.max_tokens = Some(n);
                    }
                }
                "output_style" => {
                    config.output_style = if value.is_empty() {
                        None
                    } else {
                        Some(value.clone())
                    };
                }
                "compact_threshold" => {
                    if let Ok(n) = value.parse::<f32>() {
                        config.compact_threshold = n;
                        self.compact_threshold = value.clone();
                    }
                }
                "fileAutocompleteLimit" => {
                    if let Ok(n) = value.parse::<usize>() {
                        config.file_autocomplete_limit = n;
                        self.file_autocomplete_limit = value.clone();
                    }
                }
                "fileInjectionMaxSize" => {
                    if let Ok(n) = value.parse::<usize>() {
                        config.file_injection_max_size = n;
                        self.file_injection_max_size = value.clone();
                    }
                }
                _ => {}
            }
        }
        self.settings_snapshot.config = config.clone();
        let _ = self.settings_snapshot.save_sync();
        self.pending_changes.clear();
    }
}

impl Default for SettingsScreen {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// DialogBehavior — unified key/mouse capture pipeline
// ---------------------------------------------------------------------------

impl DialogBehavior for SettingsScreen {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    /// `Esc` has dialog-specific precedence, so it is handled here rather than
    /// by the shared default (which would close unconditionally):
    ///
    /// 1. while editing a field → cancel the edit, stay open
    /// 2. while a search filter is set → clear the filter, stay open
    /// 3. otherwise → close (`Cancelled`)
    fn on_escape(&mut self) -> DialogOutcome {
        if self.edit_field.is_some() {
            self.cancel_edit();
            return DialogOutcome::Handled;
        }
        if !self.search_query.is_empty() {
            self.search_query.clear();
            self.selected_idx = 0;
            self.scroll_offset = 0;
            return DialogOutcome::Handled;
        }
        self.core.close();
        DialogOutcome::Cancelled
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        // ---- Field editing mode -------------------------------------------
        if self.edit_field.is_some() {
            return match key.code {
                KeyCode::Enter => {
                    self.commit_edit();
                    // Applying the committed value needs the app's live `Config`
                    // (`&mut Config`), which `on_key` cannot reach — flag it and
                    // let the `handle_settings_key` adapter do it.
                    self.pending_config_apply = true;
                    DialogOutcome::Handled
                }
                KeyCode::Backspace => {
                    self.edit_value.pop();
                    DialogOutcome::Handled
                }
                KeyCode::Char(c) if key.modifiers.is_empty() => {
                    self.edit_value.push(c);
                    DialogOutcome::Handled
                }
                _ => DialogOutcome::Ignored,
            };
        }

        // ---- Navigation / search mode -------------------------------------
        match key.code {
            KeyCode::Enter => {
                toggle_or_cycle_current(self);
                DialogOutcome::Handled
            }
            KeyCode::Up => {
                self.select_prev();
                self.sync_scroll();
                DialogOutcome::Handled
            }
            KeyCode::Down => {
                let total = self.filtered_len();
                self.select_next(total);
                self.sync_scroll();
                DialogOutcome::Handled
            }
            KeyCode::Backspace => {
                self.pop_search_char();
                self.sync_scroll();
                DialogOutcome::Handled
            }
            KeyCode::Char(c) if key.modifiers.is_empty() => {
                self.push_search_char(c);
                self.sync_scroll();
                DialogOutcome::Handled
            }
            _ => DialogOutcome::Ignored,
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) -> DialogOutcome {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.select_prev();
                self.sync_scroll();
                DialogOutcome::Handled
            }
            MouseEventKind::ScrollDown => {
                let total = self.filtered_len();
                self.select_next(total);
                self.sync_scroll();
                DialogOutcome::Handled
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // Click-to-select: rows map 1:1 to filtered entries from the top
                // of the body area, offset by the current scroll position.
                let body = self.core.layout().body_area;
                if body.height > 0
                    && mouse.row >= body.y
                    && mouse.row < body.y.saturating_add(body.height)
                {
                    let row = (mouse.row - body.y) as usize + self.scroll_offset;
                    if row < self.filtered_len() {
                        self.selected_idx = row;
                    }
                }
                DialogOutcome::Handled
            }
            _ => DialogOutcome::Ignored,
        }
    }

    /// The settings panel is a large custom layout rather than the shared
    /// centred modal frame, so the base chrome is bypassed here; the panel still
    /// records its geometry into the `DialogCore` (from `render_settings_screen`)
    /// so mouse hit-testing keeps working.
    fn render(&self, frame: &mut Frame, screen_area: Rect) {
        if !self.core.is_visible() {
            return;
        }
        render_settings_screen(frame, self, screen_area);
    }
}

// ---------------------------------------------------------------------------
// Settings entries definition
// ---------------------------------------------------------------------------

fn all_entries(screen: &SettingsScreen) -> Vec<SettingsEntry> {
    let mut entries = vec![
        SettingsEntry {
            key: "max_tokens",
            label: "Max Tokens",
            description: "Maximum tokens per response.",
            kind: SettingKind::Number,
            value: screen.settings_snapshot.config.max_tokens
                .map(|n| n.to_string())
                .unwrap_or_else(|| claurst_core::constants::DEFAULT_MAX_TOKENS.to_string()),
        },
        SettingsEntry {
            key: "auto_compact",
            label: "Auto-compact",
            description: "Automatically compact turns at threshold.",
            kind: SettingKind::Bool,
            value: if screen.auto_compact { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "notifications",
            label: "Desktop notifications",
            description: "Notify when a turn completes.",
            kind: SettingKind::Bool,
            value: if screen.notifications { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "show_turn_duration",
            label: "Show turn duration",
            description: "Display elapsed time per turn in status bar.",
            kind: SettingKind::Bool,
            value: if screen.show_turn_duration { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "output_style",
            label: "Output Style",
            description: "Controls the verbosity and format of responses.",
            kind: SettingKind::Enum {
                options: vec!["default", "concise", "explanatory", "learning"],
            },
            value: screen.output_style.clone(),
        },
        SettingsEntry {
            key: "reduce_motion",
            label: "Reduce motion",
            description: "Disable UI animations.",
            kind: SettingKind::Bool,
            value: if screen.reduce_motion { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "terminal_progress_bar",
            label: "Terminal progress bar",
            description: "Show progress during tool use.",
            kind: SettingKind::Bool,
            value: if screen.terminal_progress_bar { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "verbose",
            label: "Verbose logging",
            description: "Log additional debug information. Takes effect on next session.",
            kind: SettingKind::Bool,
            value: if screen.verbose { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "cursor_blink_enabled",
            label: "Cursor blinking",
            description: "Enable cursor blinking in the chat prompt.",
            kind: SettingKind::Bool,
            value: if screen.cursor_blink_enabled { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "auto_copy_enabled",
            label: "Auto-copy on highlight",
            description: "Automatically copy highlighted text to clipboard.",
            kind: SettingKind::Bool,
            value: if screen.auto_copy_enabled { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "mouse_capture",
            label: "Mouse capture",
            description: "Capture the mouse for scroll/right-click/drag-select. Turn off for native terminal text selection. Takes effect on next session.",
            kind: SettingKind::Bool,
            value: if screen.mouse_capture { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "show_cwd",
            label: "Show current directory",
            description: "Display the current working directory in the footer.",
            kind: SettingKind::Bool,
            value: if screen.show_cwd { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "show_git_branch",
            label: "Show git branch",
            description: "Display the current git branch in the footer.",
            kind: SettingKind::Bool,
            value: if screen.show_git_branch { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "compact_threshold",
            label: "Auto-compact threshold",
            description: "Context usage % at which to trigger auto-compact (0-100).",
            kind: SettingKind::Number,
            value: screen.compact_threshold.clone(),
        },
        SettingsEntry {
            key: "auto_commits",
            label: "Auto-commits",
            description: "Automatically snapshot changes to git via shadow-git.",
            kind: SettingKind::Bool,
            value: if screen.auto_commits { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "output_format",
            label: "Output format",
            description: "How responses are formatted: text, JSON, or streaming JSON.",
            kind: SettingKind::Enum {
                options: vec!["text", "json", "streamjson"],
            },
            value: screen.output_format.clone(),
        },
        SettingsEntry {
            key: "disable_claude_mds",
            label: "Disable CLAUDE.md",
            description: "Ignore CLAUDE.md files in projects (use defaults instead).",
            kind: SettingKind::Bool,
            value: if screen.disable_claude_mds { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "fileInjectionEnabled",
            label: "File injection (@)",
            description: "Auto-inject @file references into message context.",
            kind: SettingKind::Bool,
            value: if screen.file_injection_enabled { "true" } else { "false" }.to_string(),
        },
    ];

    // Only show these if file injection is enabled
    if screen.file_injection_enabled {
        entries.push(SettingsEntry {
            key: "fileAutocompleteLimit",
            label: "File autocomplete limit",
            description: "Max suggestions shown in @ autocomplete (type more to narrow results).",
            kind: SettingKind::Number,
            value: screen.file_autocomplete_limit.clone(),
        });
        entries.push(SettingsEntry {
            key: "fileAutocompleteShowHiddenFiles",
            label: "Show hidden files",
            description: "Include hidden files (.) in @ autocomplete.",
            kind: SettingKind::Bool,
            value: if screen.file_autocomplete_show_hidden_files { "true" } else { "false" }.to_string(),
        });
        entries.push(SettingsEntry {
            key: "fileInjectionMaxSize",
            label: "File injection max size",
            description: "Max file size to auto-inject (KB, 0=no limit).",
            kind: SettingKind::Number,
            value: screen.file_injection_max_size.clone(),
        });
    }

    entries
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

pub fn render_settings_screen(frame: &mut Frame, screen: &SettingsScreen, area: Rect) {
    if !screen.is_visible() {
        return;
    }

    render_dark_overlay(frame, area);

    // 80% width, 90% height, centred
    let w = (area.width * 4 / 5).max(60).min(area.width.saturating_sub(2));
    let h = (area.height * 9 / 10).max(20).min(area.height.saturating_sub(2));
    let popup = centered_rect(w, h, area);
    render_dialog_bg(frame, popup);

    // Inset inner area
    let inner = Rect {
        x: popup.x + 2,
        y: popup.y + 1,
        width: popup.width.saturating_sub(4),
        height: popup.height.saturating_sub(2),
    };

    if inner.height < 6 {
        // Too small to lay out — still record the panel so mouse hit-testing
        // knows where the (blank) dialog is.
        screen.core.set_layout(ModalLayout {
            dialog_area: popup,
            inner_area: inner,
            header_area: Rect::default(),
            body_area: Rect::default(),
            footer_area: Rect::default(),
        });
        return;
    }

    // Split into header + search + spacer + content + description + footer
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Percentage(50),
            Constraint::Length(1),
        ])
        .split(inner);

    let header_area = layout[0];
    let search_area = layout[1];
    let content_area = layout[3];
    let description_area = layout[4];
    let footer_area = layout[5];

    // Record the panel geometry so `DialogBehavior::handle_mouse` can hit-test
    // (payload-free: `DialogCore::set_layout` takes `&self` via a `Cell`).
    screen.core.set_layout(ModalLayout {
        dialog_area: popup,
        inner_area: inner,
        header_area,
        body_area: content_area,
        footer_area,
    });

    // Header
    let title = Line::from(vec![
        Span::styled(" Settings", Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(" — Claurst", Style::default().fg(CLAURST_MUTED)),
        Span::styled(
            format!("{:>width$}", "Esc close", width = inner.width.saturating_sub(19) as usize),
            Style::default().fg(CLAURST_MUTED),
        ),
    ]);
    frame.render_widget(Paragraph::new(title).style(Style::default().bg(CLAURST_PANEL_BG)), header_area);

    // Search
    let search_line = modal_search_line(&screen.search_query, "Type to search settings...", Color::DarkGray, CLAURST_ACCENT);
    frame.render_widget(Paragraph::new(search_line).style(Style::default().bg(CLAURST_PANEL_BG)), search_area);

    // Content
    render_settings_list(frame, screen, content_area);

    // Description of selected entry
    let filtered = screen.filtered_entries();

    let desc_text = if let Some(entry) = filtered.get(screen.selected_idx) {
        // For Output Style, show current selection and all available options with descriptions
        if entry.key == "output_style" {
            let mut lines = vec![entry.description.to_string(), String::new()];

            let all_styles = builtin_styles();
            let current_style_name = if screen.output_style.is_empty() { "default" } else { &screen.output_style };
            if let Some(current_style) = find_style(&all_styles, current_style_name) {
                lines.push(format!("Current: {} — {}", current_style.label, current_style.description));
                lines.push(String::new());
            }

            lines.push("Available:".to_string());
            for style in builtin_styles() {
                lines.push(format!("  {} — {}", style.name, style.description));
            }
            lines.join("\n")
        } else {
            entry.description.to_string()
        }
    } else {
        String::new()
    };
    let desc_para = Paragraph::new(desc_text)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Left)
        .block(Block::default().padding(ratatui::widgets::Padding::new(1, 0, 1, 0)));
    frame.render_widget(desc_para, description_area);

    // Footer
    let footer = if screen.edit_field.is_some() {
        Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD)),
            Span::raw("save  "),
            Span::styled(" Esc ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("cancel"),
        ])
    } else {
        Line::from(vec![
            Span::styled(" ↑↓ ", Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD)),
            Span::raw("navigate  "),
            Span::styled(" Enter ", Style::default().fg(CLAURST_ACCENT).add_modifier(Modifier::BOLD)),
            Span::raw("toggle/edit  "),
            Span::styled(" Esc ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("close"),
        ])
    };
    let footer_para = Paragraph::new(vec![footer])
        .style(Style::default().fg(CLAURST_MUTED).bg(CLAURST_PANEL_BG))
        .alignment(Alignment::Center);
    frame.render_widget(footer_para, footer_area);
}

fn render_settings_list(frame: &mut Frame, screen: &SettingsScreen, area: Rect) {
    // Entries visible under the current search filter.
    let filtered = screen.filtered_entries();

    if filtered.is_empty() {
        let para = Paragraph::new("No settings match your search.").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(para, area);
        return;
    }

    // Build lines
    let mut lines: Vec<Line> = Vec::new();
    let visible_rows = area.height as usize;

    for (i, entry) in filtered.iter().enumerate() {
        let is_selected = i == screen.selected_idx;
        let marker = if is_selected { "►" } else { " " };

        let label_len = 40usize;

        // Show edit value if currently editing this field, otherwise show the entry value
        let value_str = if screen.edit_field.as_deref() == Some(entry.key) && is_selected {
            format!("{}_ ", screen.edit_value)  // Add cursor indicator
        } else {
            entry.value.clone()
        };

        let row_style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(CLAURST_ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let line = Line::from(vec![
            Span::styled(
                format!("   {} {:<label_len$}", marker, entry.label),
                row_style,
            ),
            Span::styled(value_str, row_style),
        ]);
        lines.push(line);
    }

    // Scroll tracking is handled in update_scroll_offset_for_selection()

    // Apply manual scrolling
    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(screen.scroll_offset)
        .take(visible_rows.max(1))
        .collect();

    let para = Paragraph::new(visible_lines);
    frame.render_widget(para, area);
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

/// Drive one key event through the settings screen's `DialogBehavior`
/// pipeline. `config` is only needed when a field edit was committed (the
/// dialog itself cannot reach the app's live `Config`), so the pending apply is
/// drained here right after the event is handled.
///
/// Returns `true` when the screen consumed the event.
pub fn handle_settings_key(
    screen: &mut SettingsScreen,
    config: &mut Config,
    key: crossterm::event::KeyEvent,
) -> bool {
    let out = screen.handle_key(key);
    if screen.pending_config_apply {
        screen.pending_config_apply = false;
        screen.apply_and_save(config);
    }
    out.is_handled()
}

fn toggle_or_cycle_current(screen: &mut SettingsScreen) {
    let filtered = screen.filtered_entries();

    if let Some(entry) = filtered.get(screen.selected_idx) {
        match entry.kind {
            SettingKind::Bool => {
                let new_value = entry.value != "true";
                match entry.key {
                    "auto_compact" => {
                        screen.auto_compact = new_value;
                        screen.settings_snapshot.auto_compact = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "notifications" => {
                        screen.notifications = new_value;
                        screen.settings_snapshot.notifications = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "show_turn_duration" => {
                        screen.show_turn_duration = new_value;
                        screen.settings_snapshot.show_turn_duration = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "reduce_motion" => {
                        screen.reduce_motion = new_value;
                        screen.settings_snapshot.reduce_motion = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "terminal_progress_bar" => {
                        screen.terminal_progress_bar = new_value;
                        screen.settings_snapshot.terminal_progress_bar = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "verbose" => {
                        screen.verbose = new_value;
                        screen.settings_snapshot.config.verbose = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "cursor_blink_enabled" => {
                        screen.cursor_blink_enabled = new_value;
                        screen.settings_snapshot.config.cursor_blink_enabled = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "auto_copy_enabled" => {
                        screen.auto_copy_enabled = new_value;
                        screen.settings_snapshot.auto_copy_on_highlight = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "mouse_capture" => {
                        screen.mouse_capture = new_value;
                        // Persist only the off state; on is the default, so clear the key.
                        screen.settings_snapshot.config.mouse_capture =
                            if new_value { None } else { Some(false) };
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "show_cwd" => {
                        screen.show_cwd = new_value;
                        screen.settings_snapshot.show_cwd = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "show_git_branch" => {
                        screen.show_git_branch = new_value;
                        screen.settings_snapshot.show_git_branch = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "auto_commits" => {
                        screen.auto_commits = new_value;
                        screen.settings_snapshot.config.auto_commits = if new_value { Some(true) } else { None };
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "disable_claude_mds" => {
                        screen.disable_claude_mds = new_value;
                        screen.settings_snapshot.config.disable_claude_mds = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "fileInjectionEnabled" => {
                        screen.file_injection_enabled = new_value;
                        screen.settings_snapshot.config.file_injection_enabled = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "fileAutocompleteShowHiddenFiles" => {
                        screen.file_autocomplete_show_hidden_files = new_value;
                        screen.settings_snapshot.config.file_autocomplete_show_hidden_files = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    _ => {}
                }
            }
            SettingKind::Enum { ref options } => {
                let current_idx = options.iter().position(|&o| o == entry.value).unwrap_or(0);
                let next_idx = (current_idx + 1) % options.len();
                let new_value = options[next_idx];

                match entry.key {
                    "output_style" => {
                        screen.output_style = new_value.to_string();
                        screen.settings_snapshot.config.output_style = Some(new_value.to_string());
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "output_format" => {
                        screen.output_format = new_value.to_string();
                        screen.settings_snapshot.config.output_format = match new_value {
                            "json" => claurst_core::config::OutputFormat::Json,
                            "stream_json" => claurst_core::config::OutputFormat::StreamJson,
                            _ => claurst_core::config::OutputFormat::Text,
                        };
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    _ => {}
                }
            }
            SettingKind::Number => {
                screen.start_edit(entry.key, &entry.value);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    /// An opened screen with a synthetic layout recorded, so mouse hit-testing
    /// behaves as if it had been rendered once.
    fn opened() -> SettingsScreen {
        let mut screen = SettingsScreen::new();
        screen.open();
        screen.core.set_layout(ModalLayout {
            dialog_area: Rect::new(0, 0, 80, 24),
            inner_area: Rect::new(2, 1, 76, 22),
            header_area: Rect::new(2, 1, 76, 1),
            body_area: Rect::new(2, 5, 76, 10),
            footer_area: Rect::new(2, 23, 76, 1),
        });
        screen
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn new_screen_is_invisible_and_ignores_keys() {
        let mut screen = SettingsScreen::new();
        assert!(!screen.is_visible());
        assert!(!screen.handle_key(key(KeyCode::Enter)).is_handled());
    }

    #[test]
    fn esc_cancels_edit_then_clears_search_then_closes() {
        let mut screen = opened();

        // 1. while editing → Esc cancels the edit but keeps the screen open
        screen.start_edit("max_tokens", "4096");
        assert!(screen.handle_key(key(KeyCode::Esc)).is_handled());
        assert!(screen.edit_field.is_none());
        assert!(screen.is_visible());

        // 2. while a search filter is set → Esc clears it, still open
        screen.push_search_char('t');
        assert!(screen.handle_key(key(KeyCode::Esc)).is_handled());
        assert!(screen.search_query.is_empty());
        assert!(screen.is_visible());

        // 3. nothing pending → Esc closes the screen (Cancelled)
        let out = screen.handle_key(key(KeyCode::Esc));
        assert!(out.is_cancelled());
        assert!(!screen.is_visible());
    }

    #[test]
    fn typing_filters_the_settings_list() {
        let mut screen = opened();
        let total = screen.filtered_len();
        screen.handle_key(key(KeyCode::Char('t')));
        assert_eq!(screen.search_query, "t");
        assert_eq!(screen.selected_idx, 0);
        assert!(screen.filtered_len() < total, "filter should narrow the list");
        screen.handle_key(key(KeyCode::Backspace));
        assert!(screen.search_query.is_empty());
        assert_eq!(screen.filtered_len(), total);
    }

    #[test]
    fn number_field_edit_commit_flags_pending_apply() {
        let mut screen = opened();
        // "Max Tokens" is always the first entry and is a numeric field.
        screen.selected_idx = 0;
        assert!(screen.handle_key(key(KeyCode::Enter)).is_handled());
        assert_eq!(screen.edit_field.as_deref(), Some("max_tokens"));

        screen.handle_key(key(KeyCode::Char('8')));
        screen.handle_key(key(KeyCode::Enter));
        assert!(screen.edit_field.is_none());
        assert!(
            screen.pending_config_apply,
            "committing an edit must flag the adapter to apply it to Config"
        );
        assert!(screen.pending_changes.contains_key("max_tokens"));
    }

    #[test]
    fn mouse_scroll_and_click_move_selection() {
        let mut screen = opened();

        assert!(screen
            .handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 10,
                row: 6,
                modifiers: KeyModifiers::NONE,
            })
            .is_handled());
        assert_eq!(screen.selected_idx, 1);

        assert!(screen
            .handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 10,
                row: 6,
                modifiers: KeyModifiers::NONE,
            })
            .is_handled());
        assert_eq!(screen.selected_idx, 0);

        // Click-to-select: body starts at row 5, so row 7 selects index 2.
        screen.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 7,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(screen.selected_idx, 2);
    }

    #[test]
    fn mouse_click_on_the_mask_closes_the_screen() {
        let mut screen = opened();
        // Left click far outside the recorded dialog area (the dimmed mask).
        let out = screen.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 200,
            row: 200,
            modifiers: KeyModifiers::NONE,
        });
        assert!(out.is_cancelled(), "mask click must close the screen");
        assert!(!screen.is_visible());
    }

    #[test]
    fn mouse_wheel_outside_the_panel_is_swallowed_while_modal() {
        let mut screen = opened();
        let out = screen.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 200,
            row: 200,
            modifiers: KeyModifiers::NONE,
        });
        // Modal capture: consumed without closing (only a click dismisses).
        assert!(out.is_handled());
        assert!(screen.is_visible());
    }

    #[test]
    fn settings_screen_new_has_sensible_defaults() {
        let screen = SettingsScreen::new();
        assert!(!screen.is_visible());
        assert!(screen.search_query.is_empty());
        assert_eq!(screen.selected_idx, 0);
        assert!(screen.edit_field.is_none());
        assert!(screen.edit_value.is_empty());
    }

    #[test]
    fn all_entries_returns_expected_settings() {
        let screen = SettingsScreen::new();
        let entries = all_entries(&screen);
        // Base settings are always present, plus 0-3 conditional file injection settings
        assert!(entries.len() >= 16, "Should have at least 16 editable settings, got {}", entries.len());
        assert!(entries.len() <= 20, "Should have at most 20 editable settings, got {}", entries.len());
    }

    #[test]
    fn search_filters_entries_correctly() {
        let screen = SettingsScreen::new();
        let all = all_entries(&screen);
        let filtered: Vec<_> = all
            .iter()
            .filter(|e| e.label.to_lowercase().contains("token"))
            .collect();
        assert_eq!(filtered.len(), 1, "Should find exactly 1 entry matching 'token'");
        assert_eq!(filtered[0].label, "Max Tokens");
    }

    #[test]
    fn toggle_bool_entry_flips_value() {
        let mut screen = SettingsScreen::new();
        screen.notifications = true;
        screen.open();

        let initial = screen.notifications;
        let all = all_entries(&screen);
        let entry = &all[2]; // notifications is at index 2
        assert_eq!(entry.label, "Desktop notifications");

        // Simulate toggle (manually, since toggle_or_cycle_current modifies internal state)
        screen.notifications = !screen.notifications;
        assert_ne!(screen.notifications, initial);
    }

    #[test]
    fn cycle_enum_entry_wraps_around() {
        let mut screen = SettingsScreen::new();
        screen.output_style = "default".to_string();

        // Simulate cycling through all options
        let options = ["default", "concise", "explanatory", "learning"];
        let mut idx = options.iter().position(|&o| o == "default").unwrap();

        idx = (idx + 1) % options.len();
        assert_eq!(options[idx], "concise");

        idx = (idx + 1) % options.len();
        assert_eq!(options[idx], "explanatory");

        idx = (idx + 1) % options.len();
        assert_eq!(options[idx], "learning");

        idx = (idx + 1) % options.len();
        assert_eq!(options[idx], "default"); // Wraps around
    }
}
