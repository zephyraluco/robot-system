// render.rs â€” All ratatui rendering logic.

use std::cell::RefCell;

use crate::app::{App, SystemAnnotation, SystemMessageStyle, ToolStatus};
use crate::rustle::rustle_lines;
use crate::dialogs::dialog::DialogBehavior as _;
use crate::figures;
use crate::messages::{
    render_transcript_assistant_message_tagged,
    render_transcript_assistant_meta, render_transcript_live_text, render_transcript_user_message,
    render_thinking_live_content,
    RenderContext,
};
use crate::overlays::CLAURST_ACCENT;
use crate::plugin_views::render_plugin_hints;
use crate::prompt_input::{InputMode, TypeaheadSource, VimMode, input_height, render_prompt_input};
use crate::transcript_turn::{build_transcript_turns, TranscriptTurn};
use crate::virtual_list::{VirtualItem, VirtualList};
use claurst_core::constants::APP_VERSION;
use claurst_core::types::Role;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

// Spinner frames matching the TypeScript SpinnerGlyph: platform-specific base
// characters mirrored (forward + reverse) for a smooth pulse effect.
// Windows uses '*' instead of '✳'/'✽' for better font coverage.
#[cfg(target_os = "windows")]
const SPINNER: &[char] = &['\u{00b7}', '\u{2722}', '*', '\u{2736}', '\u{273b}', '\u{273d}',
                            '\u{273d}', '\u{273b}', '\u{2736}', '*', '\u{2722}', '\u{00b7}'];
#[cfg(not(target_os = "windows"))]
const SPINNER: &[char] = &['\u{00b7}', '\u{2722}', '\u{2733}', '\u{2736}', '\u{273b}', '\u{273d}',
                            '\u{273d}', '\u{273b}', '\u{2736}', '\u{2733}', '\u{2722}', '\u{00b7}'];
const CLAUDE_ORANGE: Color = Color::Rgb(233, 30, 99);
/// Dim grey for secondary prompt-row text (hints, provider suffix).
const PROMPT_DIM: Color = Color::Rgb(110, 110, 124);
const WELCOME_BOX_HEIGHT: u16 = 9;
const STATUS_THINKING: &str = "thinking";
const STATUS_THINKING_ELLIPSIS: &str = "thinking\u{2026}";

fn spinner_char(frame_count: u64) -> char {
    SPINNER[(frame_count as usize) % SPINNER.len()]
}

/// Returns the colour to use for the streaming spinner: claurst red normally,
/// brightening to a hot red when no stream data has arrived for over 3 seconds.
fn spinner_color(app: &App) -> Color {
    if let Some(start) = app.stall_start {
        if start.elapsed() > std::time::Duration::from_secs(3) {
            return Color::Rgb(255, 70, 70);
        }
    }
    CLAUDE_ORANGE
}

// -----------------------------------------------------------------------
// Text truncation helpers
// -----------------------------------------------------------------------

/// Short relative timestamp for the welcome screen's recent-activity list:
/// "just now", "5m ago", "2h ago", "3d ago". Clock skew (mtime in the future)
/// degrades gracefully to "just now".
fn short_relative_time(mtime: std::time::SystemTime) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(mtime)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    short_relative_secs(secs)
}

/// Formatter split out from [`short_relative_time`] so it can be unit-tested
/// without depending on the wall clock.
fn short_relative_secs(secs: u64) -> String {
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3_600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3_600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Build the body lines for the welcome box's "Recent activity" section.
///
/// Renders up to five recent sessions as `<label> <relative-time>` (the label
/// truncated to fit `width`), or a single dimmed "No recent activity" line when
/// there are none. Split out from [`render_welcome_box`] so it can be unit
/// tested from controlled state without the surrounding layout.
fn recent_activity_lines(recent: &[crate::app::RecentSession], width: usize) -> Vec<Line<'static>> {
    if recent.is_empty() {
        return vec![Line::from(Span::styled(
            "No recent activity",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    recent
        .iter()
        .take(5)
        .map(|s| {
            let when = short_relative_time(s.mtime);
            // Reserve room for the trailing " <time>" so the label truncates
            // instead of wrapping onto a second line.
            let label_w = width.saturating_sub(when.chars().count() + 1);
            let label = truncate_end(&s.label, label_w.max(1));
            Line::from(vec![
                Span::styled(label, Style::default().fg(Color::Gray)),
                Span::raw(" "),
                Span::styled(when, Style::default().fg(Color::DarkGray)),
            ])
        })
        .collect()
}

fn truncate_end(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 1 {
        return "\u{2026}".to_string();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
        if width + ch_width >= max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push('\u{2026}');
    out
}

fn truncate_middle(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 3 {
        return truncate_end(text, max_width);
    }
    let keep_each_side = (max_width.saturating_sub(1)) / 2;
    let left: String = text.chars().take(keep_each_side).collect();
    let right: String = text
        .chars()
        .rev()
        .take(keep_each_side)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{left}\u{2026}{right}")
}

fn truncate_text(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let next = format!("{out}{ch}");
        if next.width() > max_width {
            if max_width > 1 && out.width() < max_width {
                out.push('\u{2026}');
            }
            break;
        }
        out.push(ch);
    }
    out
}

// -----------------------------------------------------------------------
// Startup notice helpers
// -----------------------------------------------------------------------

fn startup_notice_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let max_width = width.saturating_sub(10) as usize;

    if let Some(summary) = app.away_summary.as_deref() {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", crate::figures::REFERENCE_MARK),
                Style::default().fg(CLAUDE_ORANGE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_end(summary, max_width),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    match &app.bridge_state {
        crate::bridge_state::BridgeConnectionState::Connected { peer_count, .. } => {
            let label = if *peer_count > 0 {
                format!("Remote session active \u{00b7} {} peer{}", peer_count, if *peer_count == 1 { "" } else { "s" })
            } else {
                "Remote session active".to_string()
            };
            lines.push(Line::from(vec![
                Span::styled(" remote ", Style::default().fg(CLAUDE_ORANGE)),
                Span::styled(label, Style::default().fg(Color::DarkGray)),
            ]));
        }
        crate::bridge_state::BridgeConnectionState::Reconnecting { attempt } => {
            lines.push(Line::from(vec![
                Span::styled(" remote ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!("Reconnecting remote session (attempt #{attempt})"),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
        crate::bridge_state::BridgeConnectionState::Failed { reason } => {
            lines.push(Line::from(vec![
                Span::styled(" remote ", Style::default().fg(Color::Red)),
                Span::styled(
                    truncate_end(reason, max_width),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
        _ => {}
    }

    if let Some(url) = app.remote_session_url.as_deref() {
        lines.push(Line::from(vec![
            Span::styled(" link ", Style::default().fg(CLAUDE_ORANGE)),
            Span::styled(
                truncate_end(url, max_width),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    // Additional directories (from --add-dir)
    for dir in &app.config.additional_dirs {
        lines.push(Line::from(vec![
            Span::styled(" +dir ", Style::default().fg(Color::Cyan)),
            Span::styled(
                truncate_end(&dir.display().to_string(), max_width),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines
}

fn render_startup_notices(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let lines = startup_notice_lines(app, area.width);
    if lines.is_empty() {
        return;
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

#[derive(Clone)]
struct RenderedLineItem {
    line: Line<'static>,
    search_text: String,
    is_header: bool,
    message_index: Option<usize>,
    /// If this line is the clickable header of a thinking block, its hash.
    thinking_hash: Option<u64>,
}

impl VirtualItem for RenderedLineItem {
    fn measure_height(&self, _width: u16) -> u16 {
        1
    }

    fn render(&self, area: Rect, buf: &mut Buffer, _selected: bool) {
        Paragraph::new(vec![self.line.clone()]).render(area, buf);
    }

    fn search_text(&self) -> String {
        self.search_text.clone()
    }

    fn is_section_header(&self) -> bool {
        self.is_header
    }
}

fn flatten_line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.to_string())
        .collect::<Vec<_>>()
        .join("")
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct MessageLinesCacheKey {
    width: u16,
    transcript_version: u64,
    messages_ptr: usize,
    messages_len: usize,
    annotations_ptr: usize,
    annotations_len: usize,
    thinking_expanded_len: usize,
}

#[derive(Clone)]
struct MessageLinesCache {
    key: MessageLinesCacheKey,
    lines: Vec<RenderedLineItem>,
}

/// Cache key for the *committed prefix* served during streaming: all messages
/// before the live (actively-streaming) turn.
///
/// Deliberately keyed by message/annotation identity — NOT by
/// `transcript_version`, which bumps on every streaming token and would churn
/// the entry away each frame (issue #222). During streaming the committed
/// messages do not change, so `messages_ptr`/`messages_len` stay stable and the
/// prefix is a cache hit every frame; when the committed set changes (a turn
/// completes, session switch/fork/revert/compaction) the pointer, length, or
/// `prefix_len` shifts and the entry is rebuilt. `prefix_len` is the number of
/// committed messages that precede the live turn, so growing the transcript by
/// one turn re-keys the prefix cleanly.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CompletedMsgCacheKey {
    width: u16,
    prefix_len: usize,
    messages_ptr: usize,
    messages_len: usize,
    annotations_ptr: usize,
    annotations_len: usize,
    thinking_expanded_len: usize,
}

#[derive(Clone)]
struct CompletedMsgCache {
    key: CompletedMsgCacheKey,
    lines: Vec<RenderedLineItem>,
}

thread_local! {
    static MESSAGE_LINES_CACHE: RefCell<Option<MessageLinesCache>> = const { RefCell::new(None) };
    /// Stores rendered lines for the committed prefix (all messages before the
    /// live turn); valid and reused across streaming deltas.
    static COMPLETED_MSG_CACHE: RefCell<Option<CompletedMsgCache>> = const { RefCell::new(None) };
}

// Instrumentation so tests can prove the committed prefix is served from cache
// (a hit) rather than rebuilt on every streaming frame. Compiled out of release
// builds.
#[cfg(test)]
thread_local! {
    static PREFIX_CACHE_HITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static PREFIX_CACHE_MISSES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn record_prefix_cache_hit() {
    PREFIX_CACHE_HITS.with(|c| c.set(c.get() + 1));
}
#[cfg(test)]
fn record_prefix_cache_miss() {
    PREFIX_CACHE_MISSES.with(|c| c.set(c.get() + 1));
}
#[cfg(not(test))]
#[inline(always)]
fn record_prefix_cache_hit() {}
#[cfg(not(test))]
#[inline(always)]
fn record_prefix_cache_miss() {}

/// Test-only: `(hits, misses)` for the committed-prefix cache.
#[cfg(test)]
fn prefix_cache_counts() -> (u64, u64) {
    (
        PREFIX_CACHE_HITS.with(|c| c.get()),
        PREFIX_CACHE_MISSES.with(|c| c.get()),
    )
}

/// Test-only: reset the render caches and counters so a test starts clean and
/// is not affected by cache state left over from a previous render on this
/// thread.
#[cfg(test)]
fn reset_render_caches() {
    MESSAGE_LINES_CACHE.with(|c| *c.borrow_mut() = None);
    COMPLETED_MSG_CACHE.with(|c| *c.borrow_mut() = None);
    PREFIX_CACHE_HITS.with(|c| c.set(0));
    PREFIX_CACHE_MISSES.with(|c| c.set(0));
}

// -----------------------------------------------------------------------
// Top-level layout
// -----------------------------------------------------------------------

/// Render the entire application into the current frame.
pub fn render_app(frame: &mut Frame, app: &App) {
    let size = frame.area();
    app.last_selectable_area.set(size);

    // Fill the entire frame with the terminal's default background so the
    // user's terminal theme shows through. `Color::Reset` emits the SGR
    // default-background sequence instead of a hard-coded color, so cells not
    // covered by widgets stay transparent rather than forcing black.
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Reset).fg(Color::White)),
        size,
    );

    let prompt_focused = app.permission_request.is_none();
    // Suggestions popup tracks whether the prompt accepts input, not whether
    // it is the focused widget. Text entry is allowed during streaming so the
    // user can queue the next message, so the typeahead popup must follow
    // that same affordance.
    let suggestions_visible = app.permission_request.is_none();
    let status_visible = should_render_status_row(app);
    // One blank separator row above the status/input area when status is active,
    // matching the visual breathing room in the TS layout.
    let separator_height: u16 = if status_visible { 1 } else { 0 };
    let status_height: u16 = if status_visible {
        if app.is_streaming {
            // The spinner row is always a short single line.
            1
        } else if let Some(text) = app.status_message.as_deref() {
            // Measure how many terminal rows the message needs so that long
            // error strings (e.g. "Error: overloaded_error (529): …") wrap
            // instead of overflowing the input area.  Cap at 3 lines.
            let usable_width = size.width.max(1) as usize;
            let char_count = text.chars().count();
            char_count.div_ceil(usable_width).clamp(1, 3) as u16
        } else {
            1
        }
    } else {
        0
    };
    let suggestions_height = if suggestions_visible && !app.prompt_input.suggestions.is_empty() {
        app.prompt_input.suggestions.len().min(5) as u16
    } else {
        0
    };
    // The prompt body width is the terminal width minus the prompt prefix
    // ("> ") and the right-margin padding used inside `render_prompt_input`.
    // Keep this in sync with prefix_width=2 + right_pad=2 there.
    let prompt_text_width = size.width.saturating_sub(4);
    let prompt_height = input_height(&app.prompt_input, prompt_text_width) + 1; // +1 for model/mode status line

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(separator_height),
            Constraint::Length(status_height),
            Constraint::Length(prompt_height),
            Constraint::Length(suggestions_height),
            Constraint::Length(1),
        ])
        .split(size);

    render_messages(frame, app, chunks[0]);
    // chunks[1] is the blank separator — intentionally left empty
    if status_height > 0 {
        render_status_row(frame, app, chunks[2]);
    }
    // The `/effort` picker is a regular centered modal (rendered with the other
    // overlays below), so the prompt box is always drawn normally.
    render_input(frame, app, chunks[3], prompt_focused);
    app.last_input_area.set(chunks[3]);
    if suggestions_height > 0 {
        render_prompt_suggestions(frame, app, chunks[4]);
    }
    render_footer(frame, app, chunks[5]);

    // Overlays (rendered on top in Z-order)

    // Permission dialog (highest priority) — routed through DialogBehavior.
    if let Some(ref pr) = app.permission_request {
        pr.render(frame, size);
    }

    // New help overlay
    if app.help_dialog.is_visible() {
        app.help_dialog.render(frame, size);
    }

    // Settings screen (highest-priority full-screen overlay) — routed through
    // its DialogBehavior `render`.
    app.settings_screen.render(frame, size);

    // Theme picker overlay — routed through its DialogBehavior `render`.
    app.theme_screen.render(frame, size);

    if app.stats_dialog.is_visible() {
        app.stats_dialog.render(frame, size);
    }

    if app.diff_viewer.is_visible() {
        app.diff_viewer.render(frame, size);
    }

    if app.feedback_survey.is_visible() {
        app.feedback_survey.render(frame, size);
    }

    if app.memory_file_selector.is_visible() {
        app.memory_file_selector.render(frame, size);
    }

    // Hooks config menu — routed through its DialogBehavior `render`.
    app.hooks_config_menu.render(frame, size);


    // Desktop upsell startup modal
    if app.desktop_upsell.is_visible() {
        app.desktop_upsell.render(frame, size);
    }

    // Import-config preview dialog
    if app.import_config_dialog.is_visible() {
        app.import_config_dialog.render(frame, size);
    }

    // Invalid config/settings dialog (shown when settings.json or AGENTS.md is malformed)
    if app.invalid_config_dialog.is_visible() {
        app.invalid_config_dialog.render(frame, size);
    }

    // Bypass-permissions confirmation dialog (topmost — rendered last so it sits above all)
    if app.bypass_permissions_dialog.is_visible() {
        app.bypass_permissions_dialog.render(frame, size);
    }

    // File injection warning dialog (shown when oversized/binary files detected)
    if app.file_injection_dialog.is_visible() {
        app.file_injection_dialog.render(frame, size);
    }

    // AskUserQuestion dialog — renders above bypass-permissions so the model's
    // question is never obscured by the startup confirmation prompt.
    if app.ask_user_dialog.is_visible() {
        app.ask_user_dialog.render(frame, size);
    }

    // First-launch onboarding dialog (shown after bypass dialog, below elicitation)
    if app.onboarding_dialog.is_visible() {
        app.onboarding_dialog.render(frame, size);
    }

    // `/effort` picker — the shared list picker modal, rendered like every other
    // DialogSelect (title bar, search field, highlight bar).
    if app.effort_dialog.is_visible() {
        app.effort_dialog.render(frame, size);
    }

    // Import-config source picker
    if app.import_config_picker.is_visible() {
        app.import_config_picker.render(frame, size);
    }

    // Connect-a-provider dialog (/connect command)
    if app.connect_dialog.is_visible() {
        app.connect_dialog.render(frame, size);
    }

    // API key input dialog (opened from /connect for key-based providers)
    if app.key_input_dialog.is_visible() {
        app.key_input_dialog.render(frame, size);
    }

    // Custom provider URL + API key dialog.
    if app.custom_provider_dialog.is_visible() {
        app.custom_provider_dialog.render(frame, size);
    }

    // "Free" composite-provider setup dialog (Zen + OpenRouter).
    if app.free_mode_dialog.is_visible() {
        app.free_mode_dialog.render(frame, size);
    }

    // Device code / browser auth dialog (GitHub Copilot, Anthropic OAuth)
    if app.device_auth_dialog.is_visible() {
        app.device_auth_dialog.render(frame, size);
    }

    // Ctrl+K command palette
    if app.command_palette.is_visible() {
        app.command_palette.render(frame, size);
    }

    // MCP elicitation dialog (highest priority modal — rendered last to sit on top)
    if app.elicitation.is_visible() {
        app.elicitation.render(frame, size);
    }

    // Model picker overlay
    if app.model_picker.is_visible() {
        app.model_picker.render(frame, size);
    }

    // Session browser overlay
    if app.session_browser.is_visible() {
        app.session_browser.render(frame, size);
    }

    // Session branching overlay
    if app.session_branching.is_visible() {
        app.session_branching.render(frame, size);
    }

    // Export format picker dialog
    if app.export_dialog.is_visible() {
        app.export_dialog.render(frame, size);
    }

    // MCP approval dialog — routed through DialogBehavior.
    if app.mcp_approval.is_visible() {
        app.mcp_approval.render(frame, size);
    }

    // ---- Text selection highlight (topmost post-pass) ---------------------
    apply_selection_highlight(frame, app);
    cache_selectable_row_text(frame, app);
    // Right-click context menu — a `DialogSelectState` list modal, drawn last
    // so it always sits above the transcript and the selection highlight.
    app.context_menu.render(frame, size);
}

/// Snapshot the rendered text of every row inside the selectable area into
/// `app.last_row_text` so that subsequent double/triple-clicks can locate
/// word and paragraph boundaries (issue #149 follow-up).
fn cache_selectable_row_text(frame: &mut Frame, app: &App) {
    let selectable_area = app.last_selectable_area.get();
    if selectable_area.width == 0 || selectable_area.height == 0 {
        app.last_row_text.borrow_mut().clear();
        return;
    }
    let buf = frame.buffer_mut();
    let max_row = selectable_area
        .y
        .saturating_add(selectable_area.height)
        .saturating_sub(1);
    let max_col = selectable_area
        .x
        .saturating_add(selectable_area.width)
        .saturating_sub(1);
    let mut cache = app.last_row_text.borrow_mut();
    cache.clear();
    for row in selectable_area.y..=max_row {
        let mut s = String::new();
        for col in selectable_area.x..=max_col {
            if let Some(cell) = buf.cell_mut((col, row)) {
                let sym = cell.symbol();
                if sym.is_empty() || sym == "\0" {
                    s.push(' ');
                } else {
                    s.push_str(sym);
                }
            }
        }
        cache.insert(row, s);
    }
}

/// Post-render pass: invert colours on selected cells and extract the
/// selection text into `app.selection_text`.
fn apply_selection_highlight(frame: &mut Frame, app: &App) {
    let (anchor, focus) = match (app.selection_anchor, app.selection_focus) {
        (Some(a), Some(f)) => (a, f),
        _ => return,
    };
    if anchor == focus {
        return;
    }

    let selectable_area = app.last_selectable_area.get();
    if selectable_area.width == 0 || selectable_area.height == 0 {
        return;
    }

    // Validate selection is within selectable bounds
    if anchor.0 < selectable_area.x
        || anchor.0 >= selectable_area.x.saturating_add(selectable_area.width)
        || anchor.1 < selectable_area.y
        || anchor.1 >= selectable_area.y.saturating_add(selectable_area.height)
    {
        return;
    }

    let max_row = selectable_area
        .y
        .saturating_add(selectable_area.height)
        .saturating_sub(1);
    let max_col = selectable_area
        .x
        .saturating_add(selectable_area.width)
        .saturating_sub(1);

    // Clamp anchor and focus to selectable bounds
    let anchor = (
        anchor.0.clamp(selectable_area.x, max_col),
        anchor.1.clamp(selectable_area.y, max_row),
    );
    let focus = (
        focus.0.clamp(selectable_area.x, max_col),
        focus.1.clamp(selectable_area.y, max_row),
    );

    // Normalise so start ≤ end (row-major order).
    let (start, end) = if (anchor.1, anchor.0) <= (focus.1, focus.0) {
        (anchor, focus)
    } else {
        (focus, anchor)
    };

    let buf = frame.buffer_mut();
    let mut text = String::new();
    let last_row = end.1.min(max_row);
    for row in start.1..=last_row {
        let col_from = if row == start.1 { start.0 } else { selectable_area.x };
        let col_to = if row == end.1 { end.0 } else { max_col };
        for col in col_from..=col_to {
            if let Some(cell) = buf.cell_mut((col, row)) {
                let sym = cell.symbol().to_owned();
                text.push_str(if sym.is_empty() || sym == "\0" { " " } else { &sym });
                // Highlight: white background, black foreground
                let new_style = Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(200, 200, 200));
                cell.set_style(new_style);
            }
        }
        if row < last_row {
            // Trim trailing spaces from line before newline
            while text.ends_with(' ') { text.pop(); }
            text.push('\n');
        }
    }
    while text.ends_with(|c: char| c.is_whitespace()) { text.pop(); }
    *app.selection_text.borrow_mut() = text;
}

// -----------------------------------------------------------------------
// Messages pane
// -----------------------------------------------------------------------

fn render_messages(frame: &mut Frame, app: &App, area: Rect) {
    // Reserve space at the top for plugin hint banners
    let hint_height = if app.plugin_hints.iter().any(|h| h.is_visible()) {
        3u16
    } else {
        0
    };

    let (hint_area, content_area) = if hint_height > 0 && area.height > hint_height + 2 {
        let splits = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(hint_height), Constraint::Min(1)])
            .split(area);
        (Some(splits[0]), splits[1])
    } else {
        (None, area)
    };

    // Render plugin hint banner if there is one
    if let Some(ha) = hint_area {
        render_plugin_hints(frame, &app.plugin_hints, ha);
    }

    // The rich two-column welcome box is a full welcome SCREEN, shown only while
    // the transcript is empty. Once a conversation starts it is NOT kept as a
    // fixed header (which permanently ate ~9 rows, issue #310); instead a compact
    // banner is prepended to the transcript (see `welcome_banner_lines`) so it
    // scrolls away with the content and the conversation reclaims the space.
    let transcript_empty = app.messages.is_empty()
        && app.streaming_text.is_empty()
        && app.streaming_thinking.is_empty()
        && app.tool_use_blocks.is_empty();

    if transcript_empty {
        app.last_msg_area.set(Rect::default());
        app.message_row_map.borrow_mut().clear();
        app.thinking_row_map.borrow_mut().clear();
        render_welcome_box(frame, app, content_area);
        // Startup notices (remote session, +dir, away summary) sit just below
        // the welcome box on the empty screen.
        let notice_lines = startup_notice_lines(app, content_area.width);
        if !notice_lines.is_empty() && content_area.height > WELCOME_BOX_HEIGHT {
            let notices_area = Rect {
                x: content_area.x,
                y: content_area.y + WELCOME_BOX_HEIGHT,
                width: content_area.width,
                height: content_area.height - WELCOME_BOX_HEIGHT,
            };
            render_startup_notices(frame, app, notices_area);
        }
        return;
    }

    // Active conversation: the whole content area is the (scrollable) transcript.
    // The welcome box is no longer on screen, so clear the anchor rect used to
    // position error modals against its right column.
    app.footer_right_column_area.set(Rect::default());
    let msg_area = content_area;

    // Store the actual message pane bounds for mouse event handling (text selection, scrolling).
    app.last_msg_area.set(msg_area);

    let lines = render_message_items(app, msg_area.width);

    // Compute total virtual height and apply scroll clamping.
    // When auto_scroll is on we always show the tail; otherwise we respect
    // the user's scroll_offset.
    let content_height = lines.len() as u16;
    let visible_height = msg_area.height;  // no borders, full height available
    let max_scroll = content_height.saturating_sub(visible_height) as usize;
    // Publish the max meaningful scroll offset so the next scroll event can
    // clamp `scroll_offset` against it (the content height is only known here,
    // at render time). Prevents unbounded inflation when scrolling past the top
    // (#223).
    app.last_max_scroll.set(max_scroll);
    // scroll_offset counts lines above the bottom (0 = at bottom).
    // ratatui scroll() takes an absolute top-row index, so convert:
    //   top_row = max_scroll - scroll_offset  (clamped to [0, max_scroll])
    let scroll = if app.auto_scroll {
        max_scroll
    } else {
        max_scroll.saturating_sub(app.scroll_offset)
    };

    let mut visible_rows: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
    let mut thinking_rows: std::collections::HashMap<u16, u64> = std::collections::HashMap::new();
    for (idx, item) in lines.iter().enumerate().skip(scroll).take(msg_area.height as usize) {
        let screen_row = msg_area.y.saturating_add((idx.saturating_sub(scroll)) as u16);
        if let Some(message_index) = item.message_index {
            visible_rows.insert(screen_row, message_index);
        }
        if let Some(hash) = item.thinking_hash {
            thinking_rows.insert(screen_row, hash);
        }
    }
    *app.message_row_map.borrow_mut() = visible_rows;
    *app.thinking_row_map.borrow_mut() = thinking_rows;

    // No border — messages render directly into the area.
    let mut list = VirtualList::new();
    list.viewport_height = msg_area.height;
    list.sticky_bottom = app.auto_scroll;
    list.set_items(lines);
    list.scroll_offset = scroll as u16;

    // Track scroll offset for selection validation
    app.last_render_scroll_offset.set(scroll as u16);

    list.render(msg_area, frame.buffer_mut());

    // Scrollbar: thin vertical strip flush with the right edge — no arrow
    // caps, no visible track, muted thumb color. Mirrors Windows Terminal /
    // most modern terminal scrollbars rather than ratatui's chunky default.
    if content_height > visible_height {
        use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

        // ratatui 0.29's Scrollbar maps `position` over `content_length - 1`,
        // not over a 0..=max_scroll range. Passing `content_height` directly
        // makes the thumb top out at `content / (content + viewport)` of the
        // track when fully scrolled — i.e. it never reaches the bottom.
        // Fix: tell ratatui the content length is the number of distinct
        // scroll positions (`max_scroll + 1`), keeping `viewport_content_length`
        // for the proportional thumb size.
        let content_len = max_scroll + 1;
        let mut scrollbar_state = ScrollbarState::new(content_len)
            .position(scroll.min(max_scroll))
            .viewport_content_length(visible_height as usize);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(None)
            .thumb_symbol("\u{2590}") // ▐ right half block — thin vertical strip
            .thumb_style(Style::default().fg(Color::Rgb(110, 110, 130)));

        frame.render_stateful_widget(scrollbar, msg_area, &mut scrollbar_state);
    }

    // “â†” N new messages” indicator when scrolled up and new messages arrived.
    if app.new_messages_while_scrolled > 0 && msg_area.height > 4 && msg_area.width > 20 {
        let indicator = format!(
            " \u{2193} {} new message{} ",
            app.new_messages_while_scrolled,
            if app.new_messages_while_scrolled == 1 { "" } else { "s" }
        );
        let ind_len = indicator.len() as u16;
        let ind_x = msg_area
            .x
            .saturating_add(msg_area.width.saturating_sub(ind_len + 2));
        let ind_y = msg_area.y + msg_area.height.saturating_sub(1);
        let ind_area = Rect {
            x: ind_x,
            y: ind_y,
            width: ind_len.min(msg_area.width.saturating_sub(2)),
            height: 1,
        };
        let ind_line = Line::from(vec![Span::styled(
            indicator,
            Style::default()
                .fg(Color::Black)
                .bg(CLAUDE_ORANGE)
                .add_modifier(Modifier::BOLD),
        )]);
        frame.render_widget(Paragraph::new(vec![ind_line]), ind_area);
    }
}

fn push_rendered_items(
    items: &mut Vec<RenderedLineItem>,
    lines: Vec<Line<'static>>,
    message_index: Option<usize>,
    mark_first_header: bool,
) {
    for (index, line) in lines.into_iter().enumerate() {
        items.push(RenderedLineItem {
            search_text: flatten_line_text(&line),
            is_header: mark_first_header && index == 0,
            message_index,
            thinking_hash: None,
            line,
        });
    }
}

/// Push tagged lines from `render_transcript_assistant_message_tagged`.
/// Lines with `Some(hash)` become clickable thinking headers.
fn push_rendered_items_tagged(
    items: &mut Vec<RenderedLineItem>,
    tagged: Vec<(Line<'static>, Option<u64>)>,
    message_index: Option<usize>,
) {
    for (line, thinking_hash) in tagged {
        items.push(RenderedLineItem {
            search_text: flatten_line_text(&line),
            is_header: false,
            message_index,
            thinking_hash,
            line,
        });
    }
}

fn push_blank_item(items: &mut Vec<RenderedLineItem>) {
    push_rendered_items(items, vec![Line::from("")], None, false);
}

fn render_live_thinking_lines(turn: &TranscriptTurn<'_>, frame_count: u64, width: u16) -> Vec<Line<'static>> {
    let mut header_spans = vec![Span::raw("  ▼ ")];
    header_spans.extend(shimmer_spans("Thinking", frame_count));
    if let Some(heading) = turn.reasoning_heading() {
        header_spans.push(Span::styled(
            format!(": {}", heading),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        ));
    }
    let mut lines = vec![Line::from(header_spans)];
    if let Some(text) = turn.live_thinking {
        lines.extend(render_thinking_live_content(text, width));
    }
    lines
}

fn append_turn_items(
    items: &mut Vec<RenderedLineItem>,
    turn: &TranscriptTurn<'_>,
    ctx: &RenderContext,
    frame_count: u64,
    accent: Color,
) {
    let width = ctx.width;
    push_rendered_items(
        items,
        render_transcript_user_message(turn.user_message, turn.metadata, width),
        Some(turn.user_index),
        true,
    );

    enum SectionContent {
        Plain(Vec<Line<'static>>),
        Tagged(Vec<(Line<'static>, Option<u64>)>),
    }

    let mut sections: Vec<(SectionContent, Option<usize>)> = Vec::new();
    for (message_index, message) in &turn.assistant_messages {
        let tagged = render_transcript_assistant_message_tagged(message, ctx);
        if !tagged.is_empty() {
            sections.push((SectionContent::Tagged(tagged), Some(*message_index)));
        }
    }

    for block in &turn.tool_blocks {
        let mut lines = Vec::new();
        render_tool_block_lines(&mut lines, block, frame_count);
        if !lines.is_empty() {
            sections.push((SectionContent::Plain(lines), Some(turn.primary_message_index())));
        }
    }

    if turn.active && turn.live_thinking.is_some() {
        sections.push((
            SectionContent::Plain(render_live_thinking_lines(turn, frame_count, width)),
            Some(turn.primary_message_index()),
        ));
    }

    // Show a "Thinking" shimmer when the turn is active but no text or
    // thinking content has arrived yet — gives visual feedback that the
    // model is working (especially for providers without thinking support).
    if turn.active
        && turn.live_text.is_none()
        && turn.live_thinking.is_none()
        && turn.tool_blocks.iter().all(|b| b.status != ToolStatus::Running)
    {
        let mut spans = vec![Span::raw("  ")];
        spans.extend(shimmer_spans("Thinking", frame_count));
        sections.push((
            SectionContent::Plain(vec![Line::from(spans)]),
            Some(turn.primary_message_index()),
        ));
    }

    if let Some(text) = turn.live_text {
        let lines = render_transcript_live_text(text, width);
        if !lines.is_empty() {
            sections.push((SectionContent::Plain(lines), Some(turn.primary_message_index())));
        }
    }

    if !turn.active {
        if let Some(meta_line) = render_transcript_assistant_meta(turn.metadata, accent) {
            if turn.has_visible_assistant_content() {
                sections.push((SectionContent::Plain(vec![meta_line]), Some(turn.primary_message_index())));
            }
        }
    }

    if !sections.is_empty() {
        push_blank_item(items);
        let total_sections = sections.len();
        for (index, (content, message_index)) in sections.into_iter().enumerate() {
            match content {
                SectionContent::Plain(lines) => push_rendered_items(items, lines, message_index, false),
                SectionContent::Tagged(tagged) => push_rendered_items_tagged(items, tagged, message_index),
            }
            if index + 1 < total_sections {
                push_blank_item(items);
            }
        }
    }

    push_blank_item(items);
}

/// Append rendered items for the transcript messages in `[start, end)` to
/// `items`, mirroring the single linear pass used by the full transcript build.
///
/// System annotations are emitted at the top of each landed index exactly as
/// the full pass does; `emit_end_annotations` additionally flushes the
/// annotations anchored at `end` (used when `end` is the true message count so
/// trailing annotations are not lost).
///
/// Splitting the pass at a turn boundary is byte-identical to building the whole
/// range in one shot: `range(0, k, false)` followed by `range(k, total, true)`
/// produces exactly the same items as `range(0, total, true)` whenever `k` is an
/// index the linear pass lands on (i.e. a turn's user index). This is what lets
/// the streaming path serve the committed prefix from cache and rebuild only the
/// live tail without any risk of ghosting.
#[allow(clippy::too_many_arguments)]
fn build_message_items_range(
    app: &App,
    width: u16,
    ctx: &RenderContext,
    turn_map: &std::collections::HashMap<usize, &TranscriptTurn<'_>>,
    start: usize,
    end: usize,
    emit_end_annotations: bool,
    items: &mut Vec<RenderedLineItem>,
) {
    let mut index = start;
    while index < end {
        for ann in app.system_annotations.iter().filter(|ann| ann.after_index == index) {
            let mut lines = Vec::new();
            render_system_annotation_lines(&mut lines, ann, width as usize);
            push_rendered_items(items, lines, None, false);
        }

        let message = &app.messages[index];
        if message.role == Role::User {
            if let Some(&turn) = turn_map.get(&index) {
                append_turn_items(items, turn, ctx, app.frame_count, app.accent_color);
                index = turn.end_message_index + 1;
                continue;
            }
        }

        let tagged = render_transcript_assistant_message_tagged(message, ctx);
        push_rendered_items_tagged(items, tagged, Some(index));
        push_blank_item(items);
        index += 1;
    }

    if emit_end_annotations {
        for ann in app.system_annotations.iter().filter(|ann| ann.after_index == end) {
            let mut lines = Vec::new();
            render_system_annotation_lines(&mut lines, ann, width as usize);
            push_rendered_items(items, lines, None, false);
        }
    }
}

/// Build the full transcript item list from scratch (no caching). Used for the
/// non-streaming path, the streaming fallback, and as the correctness reference
/// in tests.
fn build_all_items(app: &App, width: u16) -> Vec<RenderedLineItem> {
    // Build `tool_names` and the render context ONCE per rebuild and lend them
    // to every message renderer (issue #222).
    let tool_names = build_tool_names(&app.messages);
    let ctx = RenderContext {
        width,
        highlight: true,
        show_thinking: false,
        tool_names: &tool_names,
        expanded_thinking: &app.thinking_expanded,
    };
    let turns = build_transcript_turns(app);
    let mut turn_map = std::collections::HashMap::new();
    for turn in &turns {
        turn_map.insert(turn.user_index, turn);
    }

    let total = app.messages.len();
    let mut items = Vec::new();
    // Prepend the compact welcome banner as ordinary scrollable content so it
    // scrolls away with the conversation instead of sitting in a fixed header
    // (issue #310).
    push_rendered_items(&mut items, welcome_banner_lines(app, width), None, false);
    build_message_items_range(app, width, &ctx, &turn_map, 0, total, true, &mut items);

    if total == 0 && !app.tool_use_blocks.is_empty() {
        for block in &app.tool_use_blocks {
            let mut lines = Vec::new();
            render_tool_block_lines(&mut lines, block, app.frame_count);
            push_rendered_items(&mut items, lines, None, false);
            push_blank_item(&mut items);
        }
    }

    items
}

fn render_message_items(app: &App, width: u16) -> Vec<RenderedLineItem> {
    let streaming = app.is_streaming
        || !app.streaming_text.is_empty()
        || !app.streaming_thinking.is_empty();
    let has_running_tool_blocks = app
        .tool_use_blocks
        .iter()
        .any(|block| block.status == ToolStatus::Running);
    let cacheable = !streaming && !has_running_tool_blocks;

    if !cacheable {
        // Live content is on screen. Instead of re-rendering the whole backlog
        // every frame (the O(messages^2) hot path from issue #222), serve the
        // committed prefix from cache and rebuild only the live tail.
        return render_streaming_items(app, width);
    }

    // Fast path: nothing live — use the full-result cache (ptr-stable check).
    let full_key = MessageLinesCacheKey {
        width,
        transcript_version: app.transcript_version.get(),
        messages_ptr: app.messages.as_ptr() as usize,
        messages_len: app.messages.len(),
        annotations_ptr: app.system_annotations.as_ptr() as usize,
        annotations_len: app.system_annotations.len(),
        thinking_expanded_len: app.thinking_expanded.len(),
    };
    if let Some(lines) = MESSAGE_LINES_CACHE.with(|cache| {
        cache
            .borrow()
            .as_ref()
            .filter(|c| c.key == full_key)
            .map(|c| c.lines.clone())
    }) {
        return lines;
    }

    let items = build_all_items(app, width);
    MESSAGE_LINES_CACHE.with(|cache| {
        *cache.borrow_mut() = Some(MessageLinesCache {
            key: full_key,
            lines: items.clone(),
        });
    });
    items
}

/// Render the transcript while there is live content on screen.
///
/// The only part of the transcript that changes between streaming frames is the
/// last turn (its live text/thinking and any running tool blocks). Every earlier
/// turn is already committed and byte-identical to a full rebuild, so we serve
/// that committed prefix from `COMPLETED_MSG_CACHE` and rebuild only the live
/// tail. Because `build_message_items_range` splits the exact same linear pass
/// at a turn boundary, `prefix ++ tail` is identical to `build_all_items` — no
/// ghosting, no missing content.
fn render_streaming_items(app: &App, width: u16) -> Vec<RenderedLineItem> {
    let tool_names = build_tool_names(&app.messages);
    let ctx = RenderContext {
        width,
        highlight: true,
        show_thinking: false,
        tool_names: &tool_names,
        expanded_thinking: &app.thinking_expanded,
    };
    let turns = build_transcript_turns(app);

    // The live tail is the last turn; its user index is the prefix boundary.
    // Without a turn (e.g. tool-blocks-only welcome state) there is no stable
    // prefix to reuse, so fall back to a full rebuild.
    let split_idx = match turns.last() {
        Some(last) => last.user_index,
        None => return build_all_items(app, width),
    };

    let mut turn_map = std::collections::HashMap::new();
    for turn in &turns {
        turn_map.insert(turn.user_index, turn);
    }

    let total = app.messages.len();
    let prefix_key = CompletedMsgCacheKey {
        width,
        prefix_len: split_idx,
        messages_ptr: app.messages.as_ptr() as usize,
        messages_len: total,
        annotations_ptr: app.system_annotations.as_ptr() as usize,
        annotations_len: app.system_annotations.len(),
        thinking_expanded_len: app.thinking_expanded.len(),
    };

    // Committed prefix: messages before the live turn. Stable across streaming
    // deltas, so keyed by identity (not `transcript_version`) and served from
    // cache every frame after the first. The cached prefix does NOT include the
    // welcome banner, so the entry stays byte-identical to the non-streaming
    // build's committed range.
    let prefix = if let Some(lines) = COMPLETED_MSG_CACHE.with(|cache| {
        cache
            .borrow()
            .as_ref()
            .filter(|c| c.key == prefix_key)
            .map(|c| c.lines.clone())
    }) {
        record_prefix_cache_hit();
        lines
    } else {
        record_prefix_cache_miss();
        let mut prefix = Vec::new();
        build_message_items_range(app, width, &ctx, &turn_map, 0, split_idx, false, &mut prefix);
        COMPLETED_MSG_CACHE.with(|cache| {
            *cache.borrow_mut() = Some(CompletedMsgCache {
                key: prefix_key,
                lines: prefix.clone(),
            });
        });
        prefix
    };

    // The welcome banner leads the transcript and scrolls away with content
    // (issue #310). Prepended here, outside the committed-prefix cache, so
    // banner ++ prefix ++ tail stays byte-identical to `build_all_items`.
    let mut items = Vec::new();
    push_rendered_items(&mut items, welcome_banner_lines(app, width), None, false);
    items.extend(prefix);

    // Live tail: the actively-streaming turn, rebuilt fresh every frame.
    build_message_items_range(app, width, &ctx, &turn_map, split_idx, total, true, &mut items);
    items
}

// â”€â”€ Welcome / startup screen â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Render the two-column orange round-bordered welcome box (matches TS LogoV2).
fn render_welcome_box(frame: &mut Frame, app: &App, area: Rect) {

    // --- Box dimensions ---
    // The box should be at most the full area width, and a fixed height.
    let box_width = area.width;
    let box_height: u16 = WELCOME_BOX_HEIGHT;
    if area.height < box_height || box_width < 30 {
        // Too small: fall back to a single line
        let line = Line::from(vec![
            Span::styled("Claurst ", Style::default().fg(CLAUDE_ORANGE).add_modifier(Modifier::BOLD)),
            Span::styled(format!("v{}", APP_VERSION), Style::default().fg(Color::DarkGray)),
        ]);
        frame.render_widget(Paragraph::new(vec![line]), area);
        return;
    }
    let box_area = Rect { x: area.x, y: area.y, width: box_width, height: box_height };

    // Outer border with title "Claurst vX.Y"
    let accent = app.accent_color;
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
        .title(Line::from(vec![
            Span::styled(" Claurst ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::styled(format!("v{} ", APP_VERSION), Style::default().fg(Color::DarkGray)),
        ]));
    frame.render_widget(outer_block, box_area);

    // Inner area (inside the border)
    let inner = Rect {
        x: box_area.x + 1,
        y: box_area.y + 1,
        width: box_area.width.saturating_sub(2),
        height: box_area.height.saturating_sub(2),
    };

    // Split inner into left | divider(1) | right
    // Left width: ~28 chars or half the inner width, whichever is smaller
    let left_w = (inner.width / 2).clamp(22, 32).min(inner.width.saturating_sub(3));
    let right_w = inner.width.saturating_sub(left_w + 1);
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(left_w),
            Constraint::Length(1),
            Constraint::Length(right_w),
        ])
        .split(inner);

    // Store the right column area for error modal positioning
    app.footer_right_column_area.set(h_chunks[2]);

    // Draw vertical divider in accent color
    let divider_lines: Vec<Line> = (0..inner.height)
        .map(|_| Line::from(Span::styled("\u{2502}", Style::default().fg(accent))))
        .collect();
    frame.render_widget(Paragraph::new(divider_lines), h_chunks[1]);

    // --- Left column ---
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|u| !u.is_empty());
    let welcome_msg = if let Some(ref name) = username {
        format!("Welcome back {}!", name)
    } else {
        "Welcome back!".to_string()
    };
    let rustle = rustle_lines(&app.rustle_current_pose);
    let mut left_lines: Vec<Line> = Vec::new();
    left_lines.push(Line::from(Span::styled(
        welcome_msg,
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    )));
    left_lines.push(Line::from(""));
    // Center mascot in left column
    let mascot_indent = left_w.saturating_sub(11) / 2;
    let pad = " ".repeat(mascot_indent as usize);
    for cl in &rustle {
        let mut spans = vec![Span::raw(pad.clone())];
        spans.extend(cl.spans.iter().cloned());
        left_lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(left_lines).wrap(Wrap { trim: false }), h_chunks[0]);

    // --- Right column ---
    let tip_text = claurst_core::tips::select_tip(0)
        .map(|t| t.content.to_string())
        .unwrap_or_else(|| "Edit AGENTS.md to add instructions for Claurst".to_string());

    let mut right_lines: Vec<Line> = Vec::new();
    right_lines.push(Line::from(Span::styled(
        "Tips for getting started",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )));
    // Word-wrap the tip text into the right column width
    let right_w_usize = right_w.saturating_sub(1) as usize;
    for chunk in tip_text.chars().collect::<Vec<_>>().chunks(right_w_usize.max(1)) {
        right_lines.push(Line::from(chunk.iter().collect::<String>()));
    }
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(Span::styled(
        "Recent activity",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )));
    right_lines.extend(recent_activity_lines(&app.recent_sessions, right_w_usize));

    frame.render_widget(Paragraph::new(right_lines).wrap(Wrap { trim: false }), h_chunks[2]);
}

/// Build the compact welcome banner shown at the very top of the transcript.
///
/// Unlike the full two-column welcome box (which is a whole welcome *screen*
/// rendered only while the transcript is empty), this banner is prepended to the
/// message list as ordinary scrollable content, so it scrolls away with the
/// conversation instead of occupying a permanent fixed header (issue #310). It
/// carries the greeting the box led with plus a getting-started hint and any
/// startup notices, in just a few rows.
///
/// Deliberately free of disk/IO or per-frame state (no `select_tip`, which reads
/// the tip history from disk) so it is cheap to rebuild every streaming frame and
/// byte-identical between the full and cached-prefix render paths.
fn welcome_banner_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let accent = app.accent_color;

    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|u| !u.is_empty());
    let greeting = match username {
        Some(ref name) => format!("Welcome back, {}!", name),
        None => "Welcome to Claurst".to_string(),
    };

    // Too narrow for a bordered box: fall back to a single title line + notices.
    if width < 24 {
        let mut lines = vec![Line::from(vec![
            Span::styled(
                "Claurst ",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("v{}", APP_VERSION),
                Style::default().fg(Color::DarkGray),
            ),
        ])];
        lines.extend(startup_notice_lines(app, width));
        lines.push(Line::from(""));
        return lines;
    }

    let box_w = width as usize;
    let inner_w = box_w.saturating_sub(4); // "│ " + content + " │"

    // Top border with an embedded title: ╭─ Claurst vX.Y ─…─╮
    // Span widths: "╭─"=2, " Claurst "=9, "v{ver} "=ver+2, dashes=fill, "╮"=1.
    let used = 2 + 9 + (APP_VERSION.len() + 2) + 1;
    let fill = box_w.saturating_sub(used);
    let top = Line::from(vec![
        Span::styled("\u{256d}\u{2500}", Style::default().fg(accent)),
        Span::styled(
            " Claurst ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("v{} ", APP_VERSION),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{}\u{256e}", "\u{2500}".repeat(fill)),
            Style::default().fg(accent),
        ),
    ]);

    let content_row = |text: String, style: Style| -> Line<'static> {
        let text = truncate_end(&text, inner_w);
        let pad = inner_w.saturating_sub(UnicodeWidthStr::width(text.as_str()));
        Line::from(vec![
            Span::styled("\u{2502} ", Style::default().fg(accent)),
            Span::styled(text, style),
            Span::raw(" ".repeat(pad)),
            Span::styled(" \u{2502}", Style::default().fg(accent)),
        ])
    };

    let bottom = Line::from(Span::styled(
        format!("\u{2570}{}\u{256f}", "\u{2500}".repeat(box_w.saturating_sub(2))),
        Style::default().fg(accent),
    ));

    let mut lines = vec![
        top,
        content_row(
            greeting,
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        content_row(
            "/help for commands  \u{00b7}  ? for shortcuts".to_string(),
            Style::default().fg(Color::Gray),
        ),
        bottom,
    ];
    lines.extend(startup_notice_lines(app, width));
    lines.push(Line::from(""));
    lines
}

// â”€â”€ Per-message rendering â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Build a tool_use_id → tool_name lookup from all messages in the transcript.
/// This allows ToolResult blocks to dispatch to tool-specific renderers.
fn build_tool_names(messages: &[claurst_core::types::Message]) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for msg in messages {
        for block in msg.content_blocks() {
            if let claurst_core::types::ContentBlock::ToolUse { id, name, .. } = block {
                map.insert(id.clone(), name.clone());
            }
        }
    }
    map
}

// â”€â”€ System annotation (compact boundary, info notices) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn render_system_annotation_lines(
    lines: &mut Vec<Line<'static>>,
    ann: &SystemAnnotation,
    width: usize,
) {
    // Compact boundary: show âœ» prefix with dimmed text
    if ann.style == SystemMessageStyle::Compact {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} ", figures::TEARDROP_ASTERISK),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                ann.text.clone(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ),
        ]));
        lines.push(Line::from(""));
        return;
    }

    // Terminal failures get the red "API Error" block instead of the centred
    // rule below: the text is a multi-line provider message, so it needs the
    // line-by-line body, wrapped to the transcript width.
    if ann.style == SystemMessageStyle::Error {
        lines.extend(crate::messages::render_system_api_error(&ann.text, None, width));
        lines.push(Line::from(""));
        return;
    }

    let (text_color, border_color) = match ann.style {
        SystemMessageStyle::Info => (Color::DarkGray, Color::DarkGray),
        SystemMessageStyle::Warning => (Color::Yellow, Color::Yellow),
        // Both are handled above with their own block layouts.
        SystemMessageStyle::Error | SystemMessageStyle::Compact => unreachable!(),
    };

    // Centred, padded rule: "â”€â”€â”€ text â”€â”€â”€"
    let text = ann.text.as_str();
    let inner_width = width.saturating_sub(4);
    let text_len = text.len();
    let dashes = inner_width.saturating_sub(text_len + 2);
    let left = dashes / 2;
    let right = dashes - left;

    lines.push(Line::from(vec![
        Span::styled(
            format!("  {}", "\u{2500}".repeat(left)),
            Style::default().fg(border_color),
        ),
        Span::styled(
            format!("\u{2500} {} \u{2500}", text),
            Style::default().fg(text_color).add_modifier(Modifier::DIM),
        ),
        Span::styled(
            "\u{2500}".repeat(right),
            Style::default().fg(border_color),
        ),
    ]));
    lines.push(Line::from(""));
}

// â”€â”€ Tool use block â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Per-tool marker shown at the head of a tool block (the marker conveys the
/// tool, the line then shows the primary argument). Falls back to the generic
/// `~` for unmapped tools.
///
/// These are deliberately ASCII: many terminals render "pretty" Unicode glyphs
/// (arrows, ✱, ☰, …) two cells wide while ratatui's layout counts them as one,
/// which both breaks header alignment and desyncs the scroll redraw. ASCII is
/// guaranteed one cell everywhere, and the shell-flavoured choices read well in
/// context (`<` read, `>` write, `*` glob, `/` grep).
fn tool_icon(normalized: &str) -> &'static str {
    match normalized {
        "bash" | "powershell" => "$",
        "read" => "<",
        "write" | "apply_patch" | "edit" => ">",
        "glob" | "list" => "*",
        "grep" | "codesearch" => "/",
        "webfetch" => "@",
        "websearch" => "?",
        "todowrite" | "todo_write" | "todo" => ":",
        "task" | "agent" => "+",
        _ => "~",
    }
}

/// Replace a leading home-directory prefix with `~` for compact display
/// (mirrors pi's `shortenPath`). Works on Windows too via `dirs::home_dir`.
fn shorten_home_path(s: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy();
        let home = home.trim_end_matches(['/', '\\']);
        if !home.is_empty() && s.starts_with(home) {
            let rest = &s[home.len()..];
            return format!("~{}", rest);
        }
    }
    s.to_string()
}

/// Running-state verb shown (with shimmer) while a tool is in flight.
fn tool_running_label(normalized: &str, fallback: &str) -> String {
    match normalized {
        "bash" | "powershell" => "Running command",
        "read" => "Reading file",
        "write" | "apply_patch" => "Writing file",
        "edit" => "Editing file",
        "glob" | "list" => "Listing files",
        "grep" | "codesearch" => "Searching code",
        "webfetch" => "Fetching page",
        "websearch" => "Searching web",
        "todowrite" | "todo_write" | "todo" => "Updating todos",
        _ => fallback,
    }
    .to_string()
}

fn render_tool_block_lines(lines: &mut Vec<Line<'static>>, block: &crate::app::ToolUseBlock, frame_count: u64) {
    let input_val: serde_json::Value =
        serde_json::from_str(&block.input_json).unwrap_or(serde_json::Value::Null);
    let normalized = block.name.to_ascii_lowercase();
    let running = block.status == ToolStatus::Running;
    let accent = if block.status == ToolStatus::Error {
        Color::Rgb(255, 140, 0)
    } else {
        CLAUDE_ORANGE
    };
    let icon = tool_icon(&normalized);

    // TodoWrite renders as a real checklist rather than a generic tool block.
    if matches!(normalized.as_str(), "todowrite" | "todo_write" | "todo")
        && render_todo_block(lines, &input_val, icon, accent, running, frame_count)
    {
        return;
    }

    // Primary argument shown on the header line (icon + arg), opencode-style.
    let mut summary = crate::messages::extract_tool_summary(&block.name, &input_val);
    let running_label = if normalized == "task" || normalized == "agent" {
        if let Some(description) = input_val.get("description").and_then(|value| value.as_str()) {
            summary = description.to_string();
        }
        crate::messages::subagent_title(&input_val)
    } else {
        tool_running_label(&normalized, &block.name)
    };

    // Shorten home paths in path-bearing summaries.
    if matches!(
        normalized.as_str(),
        "read" | "edit" | "write" | "apply_patch" | "glob" | "list"
    ) {
        summary = shorten_home_path(&summary);
    }

    let mut header_spans = vec![Span::styled(format!("   {} ", icon), Style::default().fg(accent))];
    if running {
        header_spans.extend(shimmer_spans(&running_label, frame_count));
    } else {
        // Show the primary argument; fall back to the tool name when there is none.
        let primary = if summary.is_empty() {
            block.name.clone()
        } else {
            summary
        };
        header_spans.push(Span::styled(
            primary,
            Style::default()
                .fg(if block.status == ToolStatus::Error { accent } else { Color::White })
                .add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::from(header_spans));

    // Output preview (done/error state) — home paths shortened, dimmed.
    if let Some(ref preview) = block.output_preview {
        let preview_style = match block.status {
            ToolStatus::Error => Style::default().fg(Color::Rgb(255, 140, 0)),
            _ => Style::default().fg(Color::DarkGray),
        };
        for line_text in preview.lines() {
            if line_text.starts_with('\u{2026}') {
                lines.push(Line::from(vec![
                    Span::raw("     "),
                    Span::styled(
                        line_text.to_string(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::DIM),
                    ),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw("     "),
                    Span::styled(shorten_home_path(line_text), preview_style),
                ]));
            }
        }
    }
}

/// Render a TodoWrite call as a checklist. Returns `false` (so the caller can
/// fall back to the generic block) when the input carries no `todos` array.
fn render_todo_block(
    lines: &mut Vec<Line<'static>>,
    input_val: &serde_json::Value,
    icon: &str,
    accent: Color,
    running: bool,
    frame_count: u64,
) -> bool {
    let Some(todos) = input_val.get("todos").and_then(|v| v.as_array()) else {
        return false;
    };
    if todos.is_empty() {
        return false;
    }

    fn status_of(t: &serde_json::Value) -> &str {
        t.get("status").and_then(|s| s.as_str()).unwrap_or("pending")
    }
    let done = todos.iter().filter(|t| status_of(t) == "completed").count();
    let total = todos.len();

    // Header: ☰ Todos   <done>/<total>
    let mut header = vec![Span::styled(format!("   {} ", icon), Style::default().fg(accent))];
    if running {
        header.extend(shimmer_spans("Updating todos", frame_count));
    } else {
        header.push(Span::styled(
            "Todos".to_string(),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ));
        header.push(Span::styled(
            format!("  {}/{} done", done, total),
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines.push(Line::from(header));

    // Checklist items: ✓ done (green/dim) · • in-progress (orange) · ○ pending.
    const MAX_ITEMS: usize = 12;
    for t in todos.iter().take(MAX_ITEMS) {
        let content = t
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim();
        if content.is_empty() {
            continue;
        }
        // ASCII checkboxes (markdown-style) so alignment holds on every
        // terminal: [x] done, [>] in-progress, [ ] pending.
        let (glyph, glyph_color, text_style) = match status_of(t) {
            "completed" => (
                "[x]",
                Color::Rgb(120, 200, 120),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            ),
            "in_progress" => (
                "[>]",
                accent,
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            _ => (
                "[ ]",
                Color::Rgb(150, 150, 150),
                Style::default().fg(Color::Rgb(170, 170, 170)),
            ),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("     {} ", glyph), Style::default().fg(glyph_color)),
            Span::styled(content.to_string(), text_style),
        ]));
    }
    if total > MAX_ITEMS {
        lines.push(Line::from(vec![
            Span::raw("     "),
            Span::styled(
                format!("... {} more", total - MAX_ITEMS),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            ),
        ]));
    }
    true
}

// -----------------------------------------------------------------------
// Input pane
// -----------------------------------------------------------------------

fn render_input(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    // Split: 1-row model/mode status line + remaining rows for the prompt input.
    let (status_area, input_area) = if area.height > 2 {
        let splits = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);
        (Some(splits[0]), splits[1])
    } else {
        // Not enough room for the extra line — skip the status row.
        (None, area)
    };

    // Render model + agent mode status line above the prompt.
    if let Some(status_area) = status_area {
        let agent_mode = match app.agent_mode.as_deref() {
            Some(m) => m,
            None if app.plan_mode => "plan",
            _ => "build",
        };

        let pink = app.accent_color;
        let dim = PROMPT_DIM;
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(status_area.width.min(50))])
            .split(status_area);

        let left_line = if app.has_credentials {
            let (provider, model_short) = if let Some((provider, model)) = app.model_name.split_once('/') {
                (provider.to_string(), model.to_string())
            } else {
                ("local".to_string(), app.model_name.clone())
            };
            let mut spans = vec![
                Span::styled(
                    format!(" {} ", agent_mode.to_uppercase()),
                    Style::default()
                        .fg(Color::Black)
                        .bg(pink)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    model_short,
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
            ];
            spans.push(Span::styled(
                format!(" · {}", provider),
                Style::default().fg(dim),
            ));
            if let Some(ref badge) = app.agent_type_badge {
                spans.push(Span::styled(
                    format!(" · {}", badge),
                    Style::default().fg(dim),
                ));
            }
            Line::from(spans)
        } else {
            Line::from(vec![
                Span::styled(
                    " /connect ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(pink)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" connect a provider", Style::default().fg(dim)),
            ])
        };

        // Right side of the model/mode row: the active notification wins over the
        // hint — a failure must not be hidden behind `? shortcuts`.
        let right_hint = prompt_row_right_line(app, chunks[1].width.saturating_sub(1) as usize);

        let left_padded = Rect {
            x: chunks[0].x + 1,
            y: chunks[0].y,
            width: chunks[0].width.saturating_sub(1),
            height: chunks[0].height,
        };
        let right_padded = Rect {
            x: chunks[1].x,
            y: chunks[1].y,
            width: chunks[1].width.saturating_sub(1),
            height: chunks[1].height,
        };
        frame.render_widget(Paragraph::new(vec![left_line]), left_padded);
        frame.render_widget(
            Paragraph::new(vec![right_hint]).alignment(Alignment::Right),
            right_padded,
        );
    }

    render_prompt_input(
        &app.prompt_input,
        input_area,
        frame.buffer_mut(),
        focused,
        if app.is_streaming {
            InputMode::Readonly
        } else if app.plan_mode {
            InputMode::Plan
        } else {
            InputMode::Default
        },
        app.accent_color,
        app.settings_screen.cursor_blink_enabled,
    );
}

/// Right-hand side of the model/mode row that sits above the prompt box.
///
/// The slot normally carries the contextual hint (`? shortcuts`, the paste
/// helper), but the current notification takes it over: an error or warning is
/// the one thing on this row the user must not miss. Info-level statuses never
/// come here — they stay in the status row above the box.
fn prompt_row_right_line(app: &App, max_width: usize) -> Line<'static> {
    if let Some(note) = app.notifications.current() {
        let text = format!("{} {}", note.kind.marker(), note.message);
        return Line::from(vec![Span::styled(
            truncate_end(&text, max_width.max(1)),
            note.kind.style(),
        )]);
    }

    // `?` opens the shortcuts overlay which already lists Ctrl+A / Ctrl+K and
    // friends — surfacing them again here is redundant clutter. It is also
    // suppressed once the prompt has text, so the hint doesn't compete with what
    // the user is typing (matches the footer contract).
    if app.has_credentials && app.prompt_input.text.is_empty() {
        return Line::from(vec![Span::styled(
            "? shortcuts",
            Style::default().fg(PROMPT_DIM),
        )]);
    }
    if app.prompt_input.has_expandable_paste_ref() {
        // A [Pasted text #N ...] placeholder is in the buffer — tell the user how
        // to view the full pasted body before submitting.
        return Line::from(vec![Span::styled(
            "click to view paste · alt+e expands",
            Style::default().fg(PROMPT_DIM),
        )]);
    }
    Line::from(Vec::<Span>::new())
}

fn should_render_status_row(app: &App) -> bool {
    let interesting_stream_status = app
        .status_message
        .as_deref()
        .map(|status| {
            let trimmed = status.trim();
            !trimmed.is_empty()
                && !trimmed.eq_ignore_ascii_case(STATUS_THINKING)
                && !trimmed.eq_ignore_ascii_case(STATUS_THINKING_ELLIPSIS)
        })
        .unwrap_or(false);

    // Note: a completed turn's "Worked for Xs" summary (`last_turn_elapsed`) is
    // intentionally NOT a reason to keep the status row on — it stays set until
    // the next submit, so gating on it pinned the idle spinner glyph on screen
    // permanently after the first turn. The row now shows only while actually
    // active (streaming, or an idle status message).
    (!app.is_streaming && app.status_message.is_some())
        || (app.is_streaming && interesting_stream_status)
}

fn render_status_row(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }

    let spans = if app.is_streaming {
        // Pick a label: use the status message if it has real content,
        // otherwise show a default "Thinking" shimmer so the user always
        // sees that the model is working.
        let raw_label = app.status_message.as_deref()
            .filter(|s| {
                let t = s.trim();
                !t.is_empty()
                    && !t.eq_ignore_ascii_case(STATUS_THINKING)
                    && !t.eq_ignore_ascii_case(STATUS_THINKING_ELLIPSIS)
            })
            .or(app.spinner_verb.as_deref())
            .unwrap_or("Thinking");

        let mut s = vec![Span::styled(
            spinner_char(app.frame_count).to_string(),
            Style::default().fg(spinner_color(app)).add_modifier(Modifier::BOLD),
        )];
        let label = format!("{}…", raw_label.trim_end_matches('…'));

        s.push(Span::raw(" "));
        s.extend(shimmer_spans(&label, app.frame_count));
        s
    } else if let (Some(verb), Some(elapsed)) = (app.last_turn_verb, app.last_turn_elapsed.as_deref()) {
        // "✽ Worked for 2m 5s" — mirrors TS TeammateSpinnerLine idle state
        vec![Span::styled(
            format!("{} {} for {}", figures::TEARDROP_ASTERISK, verb, elapsed),
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
        )]
    } else if let Some(status) = app.status_message.as_deref() {
        vec![Span::styled(status.to_string(), Style::default().fg(Color::DarkGray))]
    } else {
        Vec::new()
    };

    if spans.is_empty() {
        return;
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

/// Build spans for a text string with a right-to-left glimmer sweep, matching
/// the TS `GlimmerMessage` behaviour (glimmerSpeed=200ms, 3-char shimmer window).
///
/// At ~50ms per frame a 4-frame step ≈ 200ms, giving the same cadence as TS.
fn shimmer_spans(text: &str, frame_count: u64) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if len == 0 {
        return Vec::new();
    }

    // Cycle length = text_len + 20 (10 off-screen on each side)
    let cycle_len = len + 20;
    // One step every 4 frames (~200ms at 50ms/frame)
    let cycle_pos = (frame_count as usize / 4) % cycle_len;
    // Glimmer sweeps right→left: starts at len+10 (off right), ends at -10 (off left)
    let glimmer_center = (len + 10).saturating_sub(cycle_pos) as isize;

    let base = Style::default().fg(Color::DarkGray);
    let bright = Style::default().fg(Color::White);

    // Accumulate runs of same style to minimise span count
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_bright = false;

    for (i, &ch) in chars.iter().enumerate() {
        let is_bright = (i as isize - glimmer_center).abs() <= 1
            && glimmer_center >= 0
            && glimmer_center < len as isize;

        if is_bright != run_bright && !run.is_empty() {
            spans.push(Span::styled(run.clone(), if run_bright { bright } else { base }));
            run.clear();
        }
        run_bright = is_bright;
        run.push(ch);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_bright { bright } else { base }));
    }
    spans
}
// Keybinding hints footer
// -----------------------------------------------------------------------

/// Single footer line matching the TS contract more closely:
/// - `? for shortcuts` is suppressed once the prompt becomes non-empty
/// - the right side shows comprehensive status info and notifications
fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }

    // Use only the first line of the footer area, leaving bottom padding
    let footer_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };

    // Left side: ordered pills — PR badge > background task > vim > hint
    let left_spans: Vec<Span> = {
        let mut spans: Vec<Span> = Vec::new();

        // Agent type badge (shown when running as subagent / coordinator)
        if let Some(ref badge) = app.agent_type_badge {
            spans.push(Span::styled(
                format!("\u{2699} {}", badge),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ));
        }

        // PR badge — shows "PR #<n>" in cyan, with optional state in brackets.
        // State color: approved=green, changes_requested=red,
        //              review_required=yellow, else=gray.
        if let Some(pr_num) = app.pr_number {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            let pr_label = match &app.pr_state {
                Some(state) => format!("PR #{} [{}]", pr_num, state),
                None => format!("PR #{}", pr_num),
            };
            // Colors mirror TS PrBadge getPrStatusColor + TS ink color names:
            //   approved → Green, changes_requested → Red (error),
            //   pending / review_required → Yellow (warning), merged → Magenta.
            let pr_color = match app.pr_state.as_deref() {
                Some("approved") => Color::Green,
                Some("changes_requested") => Color::Red,
                Some("merged") => Color::Magenta,
                Some("pending") | Some("review_required") => Color::Yellow,
                Some(_) => Color::Gray,
                None => Color::Cyan,
            };
            spans.push(Span::styled(
                pr_label,
                Style::default().fg(pr_color).add_modifier(Modifier::BOLD),
            ));
        }

        // Background task status pill — shows "⟳ N tasks" when count > 0.
        // Falls back to background_task_status pre-formatted string if set.
        if app.background_task_count > 0 {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            let label = if app.background_task_count == 1 {
                "\u{27f3} 1 task".to_string()
            } else {
                format!("\u{27f3} {} tasks", app.background_task_count)
            };
            spans.push(Span::styled(
                label,
                Style::default().fg(Color::Yellow),
            ));
        } else if let Some(ref task_status) = app.background_task_status {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                format!("\u{27f3} {}", task_status),
                Style::default().fg(Color::Yellow),
            ));
        }

        // Vim mode indicator — shown for all modes using neovim "-- MODE --" convention.
        // INSERT is dim (common, low-noise); other modes use bright colour.
        if app.prompt_input.vim_enabled {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            let (label, style) = match app.prompt_input.vim_mode {
                VimMode::Insert      => ("-- INSERT --",       Style::default().fg(Color::DarkGray)),
                VimMode::Normal      => ("-- NORMAL --",       Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                VimMode::Visual      => ("-- VISUAL --",       Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                VimMode::VisualLine  => ("-- VISUAL LINE --",  Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                VimMode::VisualBlock => ("-- VISUAL BLOCK --", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                VimMode::Command     => ("-- COMMAND --",      Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                VimMode::Search      => ("-- SEARCH --",       Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            };
            spans.push(Span::styled(label, style));
        }

        // Bash prefix indicator — shown when prompt starts with '!'
        if app.prompt_input.text.starts_with('!') {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                "[BASH]",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        }

        // Permission mode badge (left side, mirrors TS bottom-left indicator).
        // Default mode is silent; non-default modes show a badge.
        {
            use claurst_core::config::PermissionMode;
            match &app.config.permission_mode {
                PermissionMode::BypassPermissions => {
                    if !spans.is_empty() { spans.push(Span::raw("  ")); }
                    spans.push(Span::styled(
                        "\u{23f5}\u{23f5} bypass",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ));
                }
                PermissionMode::AcceptEdits => {
                    if !spans.is_empty() { spans.push(Span::raw("  ")); }
                    spans.push(Span::styled(
                        "accept-edits",
                        Style::default().fg(Color::Yellow),
                    ));
                }
                PermissionMode::Plan => {
                    if !spans.is_empty() { spans.push(Span::raw("  ")); }
                    spans.push(Span::styled(
                        "plan",
                        Style::default().fg(Color::Blue),
                    ));
                }
                PermissionMode::Default => {}
            }
        }

        // During streaming show "esc to interrupt". The "? shortcuts" hint is
        // rendered in the top-right status bar (see render_prompt area), so do
        // not duplicate it here (issue #149 follow-up).
        if spans.is_empty() && app.is_streaming {
            spans.push(Span::styled(
                "esc interrupt",
                Style::default().fg(Color::DarkGray),
            ));
        }

        spans
    };

    // Right side: status metrics and lightweight badges.
    let right_spans: Vec<Span> = {
        let mut parts: Vec<Span> = Vec::new();

        // 1. Context window usage — show "N% until auto-compact" mirroring TS TokenWarning.
        //    When an update is available and context is below 85%, show the update notification
        //    instead to keep the status bar uncluttered.
        if app.context_window_size > 0 {
            let used_pct = (app.context_used_tokens as f64 / app.context_window_size as f64 * 100.0) as u64;
            let left_pct = 100u64.saturating_sub(used_pct);

            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }

            if used_pct >= 85 {
                // High usage — always show context window info regardless of update status.
                if used_pct >= 95 {
                    parts.push(Span::styled(
                        format!("{}% context used — /compact now", used_pct),
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ));
                } else {
                    parts.push(Span::styled(
                        format!("{}% until auto-compact", left_pct),
                        Style::default().fg(Color::Yellow),
                    ));
                }
            } else if let Some(ref version) = app.update_available {
                // Update available and context is fine — show update nudge in bottom-right.
                parts.push(Span::styled(
                    format!("⬆ v{} available  Run: /update", version),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));
            } else if used_pct >= 70 {
                // 70–84%: mild warning.
                parts.push(Span::styled(
                    format!("{}% until auto-compact", left_pct),
                    Style::default().fg(Color::Yellow),
                ));
            } else {
                // Normal: dim display.
                let used_k = app.context_used_tokens / 1000;
                let total_k = app.context_window_size / 1000;
                parts.push(Span::styled(
                    format!("{}k/{}k", used_k, total_k),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }

        // 3. Cost — mirrors TS formatCost: 4 decimal places for costs < $0.50, else 2.
        // Display cost if it's >= 0.0, so free models show $0.00
        if app.cost_usd >= 0.0 {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            let cost_str = if app.cost_usd < 0.5 {
                format!("${:.4}", app.cost_usd)
            } else {
                format!("${:.2}", app.cost_usd)
            };
            parts.push(Span::styled(
                cost_str,
                Style::default().fg(Color::DarkGray),
            ));
        }

        // 3b. Token budget (feature-gated)
        #[cfg(feature = "token_budget")]
        if let Some(max_tokens) = app.token_budget {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            let used = app.token_count as u64;
            let max = max_tokens as u64;
            let pct = if max > 0 {
                (used as f64 / max as f64 * 100.0) as u32
            } else {
                0
            };
            let color = if pct >= 90 {
                Color::Red
            } else if pct >= 75 {
                Color::Yellow
            } else {
                Color::DarkGray
            };
            parts.push(Span::styled(
                format!("Tokens: {}/{} ({}%)", used, max, pct),
                Style::default().fg(color),
            ));
        }

        // 4. Rate limits
        if let Some(pct) = app.rate_limit_5h_pct {
            if pct > 0.0 {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                let color = if pct >= 90.0 { Color::Red } else { Color::Yellow };
                parts.push(Span::styled(
                    format!("5h:{:.0}%", pct),
                    Style::default().fg(color),
                ));
            }
        }
        if let Some(pct) = app.rate_limit_7day_pct {
            if pct > 0.0 {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                let color = if pct >= 90.0 { Color::Red } else { Color::Yellow };
                parts.push(Span::styled(
                    format!("7d:{:.0}%", pct),
                    Style::default().fg(color),
                ));
            }
        }

        // 5. Vim mode — displayed on the left side as "-- MODE --"; nothing extra on right.

        // 5b. Goal badge — shown when a goal is active for this session.
        if let Some(ref badge) = app.active_goal_badge {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(Span::styled(
                format!("[goal: {}]", badge),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        }

        // 6. Agent type badge
        if let Some(ref badge) = app.agent_type_badge {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(Span::styled(
                format!("[{}]", badge),
                Style::default().fg(CLAURST_ACCENT),
            ));
        }

        // 7. Worktree branch
        if let Some(ref branch) = app.worktree_branch {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(Span::styled(
                format!("[{}]", branch),
                Style::default().fg(Color::Green),
            ));
        }

        // Git branch (if settings enabled)
        if app.settings_screen.show_git_branch {
            if let Some(ref branch) = app.git_branch {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                parts.push(Span::styled(
                    format!("⎇ {}", branch),
                    Style::default().fg(Color::Cyan),
                ));
            }
        }

        // Current directory (if settings enabled)
        if app.settings_screen.show_cwd {
            if let Some(ref dir) = app.current_dir {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                // Use dirs::home_dir() so this works on Windows (where $HOME
                // is unset and the home is $USERPROFILE). Guard against an
                // empty home string: `str::replace("", "~")` inserts "~"
                // between every character, producing the infamous
                // `~X~:~\~B~i~g~g~e~r~…` output.
                let home = dirs::home_dir()
                    .and_then(|p| p.to_str().map(|s| s.to_string()))
                    .filter(|s| !s.is_empty());
                let display_dir = match home {
                    Some(h) if dir.starts_with(&h) => dir.replacen(&h, "~", 1),
                    _ => dir.clone(),
                };
                parts.push(Span::styled(
                    display_dir,
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }

        // Output style indicator (only when non-default)
        if app.output_style != "auto" {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(Span::styled(
                format!("[{}]", app.output_style),
                Style::default().fg(Color::DarkGray),
            ));
        }

        // External status line override
        if let Some(ref override_text) = app.status_line_override {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            // Strip any ANSI escapes for terminal rendering (plain text)
            let clean: String = override_text
                .chars()
                .filter(|c| c.is_ascii_graphic() || *c == ' ')
                .collect();
            parts.push(Span::styled(clean, Style::default().fg(Color::DarkGray)));
        }

        // 8. Bridge badge
        if let Some(badge) = app.bridge_state.status_badge(app.frame_count) {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(badge);
        } else if app.pending_mcp_reconnect {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(Span::styled(
                "MCP reconnecting",
                Style::default().fg(Color::Yellow),
            ));
        }

        parts
    };

    // Gap fill
    let left_len: usize = left_spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let right_len: usize = right_spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let gap = (footer_area.width.saturating_sub(2) as usize).saturating_sub(left_len + right_len);

    let mut spans = left_spans;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right_spans);

    // Add padding: 1 char on each side
    let padded_area = Rect {
        x: footer_area.x + 1,
        y: footer_area.y,
        width: footer_area.width.saturating_sub(2),
        height: footer_area.height,
    };
    frame.render_widget(Paragraph::new(vec![Line::from(spans)]), padded_area);
}

fn render_prompt_suggestions(frame: &mut Frame, app: &App, area: Rect) {
    let suggestions = &app.prompt_input.suggestions;
    if suggestions.is_empty() || area.height == 0 {
        return;
    }

    let selected = app.prompt_input.suggestion_index.unwrap_or(0);
    let max_visible = area.height as usize;
    let start = selected
        .saturating_sub(max_visible / 2)
        .min(suggestions.len().saturating_sub(max_visible));
    let end = (start + max_visible).min(suggestions.len());
    let label_width = area.width.saturating_div(3).max(12) as usize;

    for (row, suggestion) in suggestions[start..end].iter().enumerate() {
        let is_selected = start + row == selected;
        let accent_style = if is_selected {
            Style::default().fg(CLAUDE_ORANGE).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let label_style = if is_selected {
            Style::default().fg(CLAUDE_ORANGE).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let detail_style = if is_selected {
            Style::default().fg(CLAUDE_ORANGE)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut spans = vec![Span::styled(if is_selected { "\u{203a} " } else { "  " }, accent_style)];
        match suggestion.source {
            TypeaheadSource::SlashCommand => {
                let display_name = truncate_text(&suggestion.text, label_width);
                spans.push(Span::styled(
                    format!("{display_name:<width$}", width = label_width),
                    label_style,
                ));
                spans.push(Span::styled(" [cmd] ", Style::default().fg(Color::DarkGray)));
                if !suggestion.description.is_empty() {
                    spans.push(Span::styled(
                        truncate_text(
                            &suggestion.description,
                            area.width.saturating_sub(label_width as u16 + 10) as usize,
                        ),
                        detail_style,
                    ));
                }
            }
            TypeaheadSource::FileRef => {
                spans.push(Span::styled("+ ", accent_style));
                spans.push(Span::styled(
                    truncate_middle(&suggestion.text, label_width),
                    label_style,
                ));
                if !suggestion.description.is_empty() {
                    spans.push(Span::styled(" \u{2014} ", Style::default().fg(Color::DarkGray)));
                    spans.push(Span::styled(
                        truncate_text(&suggestion.description, area.width as usize / 2),
                        detail_style,
                    ));
                }
            }
            TypeaheadSource::History => {
                let display_name = truncate_text(&suggestion.text, label_width);
                spans.push(Span::styled(
                    format!("{display_name:<width$}", width = label_width),
                    label_style,
                ));
                spans.push(Span::styled(" [history] ", Style::default().fg(Color::DarkGray)));
                if !suggestion.description.is_empty() {
                    spans.push(Span::styled(
                        truncate_text(&suggestion.description, area.width as usize / 2),
                        detail_style,
                    ));
                }
            }
        }

        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                x: area.x,
                y: area.y + row as u16,
                width: area.width,
                height: 1,
            },
        );
    }
}

// -----------------------------------------------------------------------
// Complete status line (T2-8)
// -----------------------------------------------------------------------


// ---------------------------------------------------------------------------
// Multi-agent UI components
// ---------------------------------------------------------------------------

/// Render a single progress-indicator row for a sub-agent.
///
/// Format: `[agent-<id>]` in cyan dim · space · status in colour · ` · ` · tool in dim gray
///
/// # Arguments
/// * `agent_id`    — short agent identifier (e.g. `"abc123"`)
/// * `status`      — current status string: `"working"`, `"done"`, `"error"`, or other
/// * `current_tool` — tool the agent is currently executing, if any
pub fn render_agent_progress_line(
    agent_id: &str,
    status: &str,
    current_tool: Option<&str>,
) -> Line<'static> {
    let status_color = match status {
        "working" | "running" => Color::Yellow,
        "done" | "complete" | "completed" => Color::Green,
        "error" | "failed" => Color::Red,
        _ => Color::DarkGray,
    };

    let mut spans = vec![
        Span::styled(
            format!("[agent-{}]", agent_id),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::DIM),
        ),
        Span::raw(" "),
        Span::styled(
            status.to_string(),
            Style::default().fg(status_color),
        ),
    ];

    if let Some(tool) = current_tool {
        spans.push(Span::styled(
            " · ".to_string(),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::styled(
            tool.to_string(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ));
    }

    Line::from(spans)
}

// ---------------------------------------------------------------------------
// Tests — tool-block rendering (icon headers, path shortening, todo checklist)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tool_block_tests {
    use super::*;
    use crate::app::{ToolStatus, ToolUseBlock};

    fn block(name: &str, status: ToolStatus, input: &str, preview: Option<&str>) -> ToolUseBlock {
        ToolUseBlock {
            id: "t".into(),
            name: name.into(),
            turn_index: None,
            status,
            output_preview: preview.map(|s| s.to_string()),
            input_json: input.into(),
        }
    }

    fn render(b: &ToolUseBlock) -> Vec<String> {
        let mut lines = Vec::new();
        render_tool_block_lines(&mut lines, b, 0);
        lines.iter().map(flatten_line_text).collect()
    }

    #[test]
    fn icons_are_per_tool_and_ascii() {
        assert_eq!(tool_icon("bash"), "$");
        assert_eq!(tool_icon("read"), "<");
        assert_eq!(tool_icon("write"), ">");
        assert_eq!(tool_icon("glob"), "*");
        assert_eq!(tool_icon("grep"), "/");
        assert_eq!(tool_icon("todowrite"), ":");
        assert_eq!(tool_icon("something-unknown"), "~");
        // All markers must be single-byte ASCII (guaranteed one terminal cell).
        for t in ["bash", "read", "write", "glob", "grep", "webfetch", "websearch", "todo", "task", "x"] {
            let icon = tool_icon(t);
            assert_eq!(icon.len(), 1, "{t} icon {icon:?} must be 1 ASCII byte");
            assert!(icon.is_ascii(), "{t} icon {icon:?} must be ASCII");
        }
    }

    #[test]
    fn shorten_home_replaces_prefix() {
        if let Some(home) = dirs::home_dir() {
            let p = home.join("projects").join("x.yaml");
            let shortened = shorten_home_path(&p.to_string_lossy());
            assert!(shortened.starts_with("~"), "got {shortened:?}");
            assert!(shortened.ends_with("x.yaml"));
            assert!(!shortened.contains(home.to_string_lossy().as_ref()));
        }
        // A non-home path is left untouched.
        assert_eq!(shorten_home_path("/etc/hosts"), "/etc/hosts");
    }

    #[test]
    fn bash_header_is_icon_led_and_not_duplicated() {
        let b = block(
            "bash",
            ToolStatus::Done,
            r#"{"command":"python3 - <<'PY'\nfrom pathlib import Path"}"#,
            Some("218183\nMarketing Outbound OS"),
        );
        let lines = render(&b);
        // Header: "$ python3 - <<'PY'"
        assert!(lines[0].contains('$'), "header should be icon-led: {:?}", lines[0]);
        assert!(lines[0].contains("python3 - <<'PY'"), "header shows command: {:?}", lines[0]);
        // The command must appear exactly once (no summary + $-line duplication).
        let joined = lines.join("\n");
        assert_eq!(joined.matches("python3 - <<'PY'").count(), 1, "no dup: {joined:?}");
        // Output preview still shown.
        assert!(joined.contains("218183"));
    }

    #[test]
    fn read_header_shortens_home_path() {
        if let Some(home) = dirs::home_dir() {
            let path = home.join("FOLLOWUPS.md");
            let input = serde_json::json!({
                "file_path": path.to_string_lossy().to_string(),
            })
            .to_string();
            let b = block("read", ToolStatus::Done, &input, None);
            let lines = render(&b);
            assert!(lines[0].contains('<'), "read icon: {:?}", lines[0]);
            assert!(lines[0].contains('~'), "home shortened: {:?}", lines[0]);
            assert!(!lines[0].contains(home.to_string_lossy().as_ref()));
        }
    }

    #[test]
    fn todo_renders_checklist_with_glyphs_and_counts() {
        let b = block(
            "TodoWrite",
            ToolStatus::Done,
            r#"{"todos":[
                {"content":"Locate files","status":"completed"},
                {"content":"Build importer","status":"in_progress"},
                {"content":"Wire adapter","status":"pending"}
            ]}"#,
            Some("Todo list updated (3 total)"),
        );
        let lines = render(&b);
        let joined = lines.join("\n");
        // Header shows count, not the raw "Todo list updated (...)".
        assert!(joined.contains("Todos"), "{joined:?}");
        assert!(joined.contains("1/3 done"), "{joined:?}");
        // Each status has its ASCII checkbox + content.
        assert!(joined.contains("[x] Locate files"), "done marker: {joined:?}");
        assert!(joined.contains("[>] Build importer"), "in-progress marker: {joined:?}");
        assert!(joined.contains("[ ] Wire adapter"), "pending marker: {joined:?}");
        // The raw result-preview string must NOT leak into the checklist view.
        assert!(!joined.contains("Todo list updated"), "preview suppressed: {joined:?}");
    }
}

/// Tests for the streaming transcript cache (issue #222): the committed prefix
/// must be reused across streaming deltas, and streaming output must be
/// byte-identical to a full (non-cached) rebuild.
#[cfg(test)]
mod stream_cache_tests {
    use super::*;
    use crate::app::App;
    use claurst_core::config::Config;
    use claurst_core::cost::CostTracker;
    use claurst_core::types::Message;

    const WIDTH: u16 = 80;

    fn test_app() -> App {
        App::new(Config::default(), CostTracker::new())
    }

    /// A per-item signature that captures the rendered spans+styles (via Debug)
    /// plus all metadata, so equality means byte-identical rendering.
    fn item_sig(item: &RenderedLineItem) -> (String, bool, Option<usize>, Option<u64>) {
        (
            format!("{:?}", item.line),
            item.is_header,
            item.message_index,
            item.thinking_hash,
        )
    }

    fn sigs(items: &[RenderedLineItem]) -> Vec<(String, bool, Option<usize>, Option<u64>)> {
        items.iter().map(item_sig).collect()
    }

    fn joined_text(items: &[RenderedLineItem]) -> String {
        items
            .iter()
            .map(|i| i.search_text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The completed-message items are reused (served from cache) across a
    /// streaming delta, while the live tail updates.
    #[test]
    fn completed_prefix_reused_across_streaming_delta() {
        let mut app = test_app();
        // Turn 0 is fully committed; turn 1 is the live/streaming turn.
        app.messages.push(Message::user("user one prompt"));
        app.messages.push(Message::assistant("assistant one committed reply"));
        app.messages.push(Message::user("user two prompt"));
        app.is_streaming = true;
        app.streaming_text = "streaming tail alpha".to_string();

        reset_render_caches();

        // First render: prefix is built fresh (a miss).
        let render1 = render_message_items(&app, WIDTH);
        assert_eq!(prefix_cache_counts(), (0, 1), "first render builds the prefix");

        // A streaming delta arrives: only the live text grows. Real code bumps
        // transcript_version on every delta — assert that does NOT evict the
        // committed-prefix entry.
        app.streaming_text.push_str(" beta");
        app.invalidate_transcript();

        let render2 = render_message_items(&app, WIDTH);
        let (hits, misses) = prefix_cache_counts();
        assert_eq!(
            (hits, misses),
            (1, 1),
            "committed prefix served from cache after the delta (no rebuild)"
        );

        // The committed content is identical in both renders and appears before
        // the live tail diverges.
        let sig1 = sigs(&render1);
        let sig2 = sigs(&render2);
        let common = sig1
            .iter()
            .zip(sig2.iter())
            .take_while(|(a, b)| a == b)
            .count();
        assert!(common > 0, "some leading items must be identical");
        let leading_text = joined_text(&render1[..common]);
        assert!(
            leading_text.contains("user one prompt")
                && leading_text.contains("assistant one committed reply"),
            "the reused prefix contains the whole committed turn: {leading_text:?}"
        );
        // The reused prefix must not contain any live tail content.
        assert!(
            !leading_text.contains("streaming tail alpha"),
            "prefix must not include the live tail: {leading_text:?}"
        );

        // The live tail updated between renders.
        let text1 = joined_text(&render1);
        let text2 = joined_text(&render2);
        assert!(text1.contains("streaming tail alpha"));
        assert!(!text1.contains("streaming tail alpha beta"));
        assert!(text2.contains("streaming tail alpha beta"), "tail rebuilt with the delta");
    }

    /// Streaming render (cached prefix + rebuilt tail) is byte-identical to a
    /// full rebuild for a multi-message transcript — no ghosting, no missing or
    /// stale content — both on the first (cold) frame and after a delta (warm).
    #[test]
    fn streaming_render_matches_full_rebuild() {
        let mut app = test_app();
        app.messages.push(Message::user("first user question"));
        app.messages.push(Message::assistant("first assistant answer with **markdown**"));
        app.messages.push(Message::user("second user question"));
        app.messages.push(Message::assistant("second assistant answer"));
        app.messages.push(Message::user("third user question"));
        app.is_streaming = true;
        app.streaming_thinking = "pondering the third answer".to_string();
        app.streaming_text = "third answer so far".to_string();

        reset_render_caches();

        // Cold frame: streaming path vs a direct full rebuild.
        let streamed_cold = render_message_items(&app, WIDTH);
        let full_cold = build_all_items(&app, WIDTH);
        assert_eq!(
            sigs(&streamed_cold),
            sigs(&full_cold),
            "cold streaming render must match a full rebuild"
        );

        // Warm frame: after a delta, the prefix is served from cache but the
        // concatenation must still equal a full rebuild.
        app.streaming_text.push_str(" plus more tokens");
        app.invalidate_transcript();
        let streamed_warm = render_message_items(&app, WIDTH);
        let (hits, _) = prefix_cache_counts();
        assert!(hits >= 1, "warm frame served the prefix from cache");
        let full_warm = build_all_items(&app, WIDTH);
        assert_eq!(
            sigs(&streamed_warm),
            sigs(&full_warm),
            "warm streaming render must match a full rebuild"
        );
    }

    /// Swapping the transcript (session switch / fork / revert / compaction)
    /// must NOT serve a stale committed prefix, even mid-stream.
    #[test]
    fn transcript_swap_does_not_ghost_stale_prefix() {
        let mut app = test_app();
        app.messages.push(Message::user("session A user"));
        app.messages.push(Message::assistant("session A assistant reply"));
        app.messages.push(Message::user("session A live turn"));
        app.is_streaming = true;
        app.streaming_text = "A tail".to_string();

        reset_render_caches();
        let render_a = render_message_items(&app, WIDTH);
        assert!(joined_text(&render_a).contains("session A assistant reply"));

        // Swap in a different transcript (new Vec) while still streaming. The
        // prefix cache must be re-keyed by identity, so no session-A content
        // leaks through.
        app.messages = vec![
            Message::user("session B user"),
            Message::assistant("session B assistant reply"),
            Message::user("session B live turn"),
        ];
        app.streaming_text = "B tail".to_string();
        app.invalidate_transcript();

        let render_b = render_message_items(&app, WIDTH);
        let text_b = joined_text(&render_b);
        assert!(text_b.contains("session B assistant reply"), "shows swapped content");
        assert!(
            !text_b.contains("session A"),
            "no stale session-A content ghosts through: {text_b:?}"
        );
        // And the swapped render equals a full rebuild.
        assert_eq!(sigs(&render_b), sigs(&build_all_items(&app, WIDTH)));
    }

    /// The last message toggling streaming -> completed moves cleanly into the
    /// cached (non-streaming) set with identical content.
    #[test]
    fn streaming_to_completed_transition_is_clean() {
        let mut app = test_app();
        app.messages.push(Message::user("q1"));
        app.messages.push(Message::assistant("a1 committed"));
        app.messages.push(Message::user("q2"));
        app.is_streaming = true;
        app.streaming_text = "live answer body".to_string();

        reset_render_caches();
        let _streaming = render_message_items(&app, WIDTH);

        // Commit the streamed message (as flush_streamed_assistant_message would)
        // and end streaming.
        app.messages.push(Message::assistant("live answer body"));
        app.is_streaming = false;
        app.streaming_text.clear();
        app.invalidate_transcript();

        let completed = render_message_items(&app, WIDTH);
        // Non-streaming render equals a full rebuild (correct committed set).
        assert_eq!(sigs(&completed), sigs(&build_all_items(&app, WIDTH)));
        let text = joined_text(&completed);
        assert!(text.contains("a1 committed"));
        assert!(text.contains("live answer body"));
    }
}

/// The `/effort` picker is an ordinary centered list modal (`DialogSelectState`)
/// rendered on top of the transcript; the prompt box keeps rendering underneath.
#[cfg(test)]
mod effort_picker_tests {
    use super::*;
    use crate::app::App;
    use claurst_core::config::Config;
    use claurst_core::cost::CostTracker;
    use ratatui::{backend::TestBackend, Terminal};

    /// The prompt pointer glyph drawn by `render_prompt_input`.
    const PROMPT_POINTER: char = '\u{276f}';

    fn render_screen(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| render_app(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if let Some(cell) = buf.cell((x, y)) {
                    out.push_str(cell.symbol());
                }
            }
        }
        out
    }

    #[test]
    fn effort_dialog_renders_as_a_list_modal() {
        let mut app = App::new(Config::default(), CostTracker::new());

        // Closed: the prompt box (its pointer) is drawn; no picker chrome.
        let closed = render_screen(&app);
        assert!(
            closed.contains(PROMPT_POINTER),
            "prompt pointer should be visible when the picker is closed"
        );
        assert!(
            !closed.contains("ultracode"),
            "picker labels must not show while the picker is closed"
        );

        // Open: a centered list modal is drawn ON TOP — the prompt box stays
        // visible underneath (it is no longer replaced by a docked panel).
        app.open_effort_picker();
        let open = render_screen(&app);
        assert!(
            open.contains("Select effort") && open.contains("ultracode"),
            "the effort list modal should render its ladder"
        );
        assert!(
            open.contains(PROMPT_POINTER),
            "the prompt box must still be drawn under the modal"
        );
    }
}

// ---------------------------------------------------------------------------
// Welcome screen: recent activity (issue #277)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod recent_activity_tests {
    use super::*;
    use crate::app::{App, RecentSession};
    use claurst_core::config::Config;
    use claurst_core::cost::CostTracker;
    use ratatui::{backend::TestBackend, Terminal};
    use std::time::{Duration, SystemTime};

    fn recent(label: &str, secs_ago: u64) -> RecentSession {
        RecentSession {
            label: label.to_string(),
            mtime: SystemTime::now() - Duration::from_secs(secs_ago),
        }
    }

    fn lines_text(recent: &[RecentSession], width: usize) -> Vec<String> {
        recent_activity_lines(recent, width)
            .iter()
            .map(flatten_line_text)
            .collect()
    }

    // -- relative-time formatter ------------------------------------------

    #[test]
    fn short_relative_secs_buckets() {
        assert_eq!(short_relative_secs(0), "just now");
        assert_eq!(short_relative_secs(59), "just now");
        assert_eq!(short_relative_secs(60), "1m ago");
        assert_eq!(short_relative_secs(5 * 60), "5m ago");
        assert_eq!(short_relative_secs(2 * 3_600), "2h ago");
        assert_eq!(short_relative_secs(3 * 86_400), "3d ago");
    }

    #[test]
    fn short_relative_time_handles_future_mtime() {
        // Clock skew (mtime slightly in the future) must not panic.
        let future = SystemTime::now() + Duration::from_secs(120);
        assert_eq!(short_relative_time(future), "just now");
    }

    // -- render-from-state path -------------------------------------------

    #[test]
    fn empty_state_shows_placeholder() {
        let out = lines_text(&[], 40);
        assert_eq!(out, vec!["No recent activity".to_string()]);
    }

    #[test]
    fn populated_state_shows_titles_and_relative_times() {
        let sessions = vec![
            recent("Fix the parser bug", 2 * 3_600),
            recent("Wire up onboarding", 3 * 86_400),
        ];
        let out = lines_text(&sessions, 40).join("\n");
        assert!(out.contains("Fix the parser bug"), "first title: {out:?}");
        assert!(out.contains("2h ago"), "first time: {out:?}");
        assert!(out.contains("Wire up onboarding"), "second title: {out:?}");
        assert!(out.contains("3d ago"), "second time: {out:?}");
        // The placeholder must NOT appear when there is real activity.
        assert!(!out.contains("No recent activity"), "no placeholder: {out:?}");
    }

    #[test]
    fn caps_at_five_entries() {
        let sessions: Vec<RecentSession> =
            (0..8).map(|i| recent(&format!("session {i}"), 60)).collect();
        assert_eq!(recent_activity_lines(&sessions, 40).len(), 5);
    }

    #[test]
    fn long_label_is_truncated_and_leaves_room_for_time() {
        let sessions = vec![recent(
            "an extremely long session title that should be truncated to fit",
            60,
        )];
        let out = lines_text(&sessions, 20);
        assert_eq!(out.len(), 1);
        let line = &out[0];
        assert!(line.contains('\u{2026}'), "should be ellipsised: {line:?}");
        assert!(line.ends_with("1m ago"), "time preserved at end: {line:?}");
    }

    #[test]
    fn welcome_box_renders_recent_activity_from_state() {
        // Full-widget smoke test: the section header renders and, when state is
        // populated, a session label reaches the screen buffer without panic.
        let mut app = App::new(Config::default(), CostTracker::new());
        app.recent_sessions = vec![recent("Sortable label ABCDEF", 2 * 3_600)];

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| render_welcome_box(frame, &app, frame.area()))
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(screen.contains("Recent activity"), "header rendered: present");
        assert!(screen.contains("Sortable label"), "session label rendered");
    }
}

// ---------------------------------------------------------------------------
// Prompt row + prompt box overlays
// ---------------------------------------------------------------------------

/// The model/mode row above the prompt box: hint vs. notification.
#[cfg(test)]
mod prompt_status_row_tests {
    use super::*;
    use crate::app::App;
    use claurst_core::config::Config;
    use claurst_core::cost::CostTracker;

    /// Width of the hint slot on an 80-column terminal: `min(width, 50) - 1`.
    const SLOT: usize = 49;

    /// `has_credentials` + empty prompt: the state in which the hint shows.
    fn hint_app() -> App {
        let mut app = App::new(Config::default(), CostTracker::new());
        app.has_credentials = true;
        app
    }

    fn text_of(line: &Line<'_>) -> String {
        flatten_line_text(line)
    }

    fn color_of(line: &Line<'_>) -> Option<Color> {
        line.spans.first().and_then(|s| s.style.fg)
    }

    #[test]
    fn hint_is_shown_when_no_notification_is_active() {
        let app = hint_app();
        assert_eq!(text_of(&prompt_row_right_line(&app, SLOT)), "? shortcuts");
        assert_eq!(color_of(&prompt_row_right_line(&app, SLOT)), Some(PROMPT_DIM));
    }

    #[test]
    fn error_notification_takes_over_the_hint_slot_in_red() {
        let mut app = hint_app();
        app.notify_error("Tool error: boom");
        let line = prompt_row_right_line(&app, SLOT);
        assert_eq!(text_of(&line), "x Tool error: boom");
        assert_eq!(color_of(&line), Some(Color::Red));
    }

    #[test]
    fn warning_notification_takes_over_the_hint_slot_in_yellow() {
        let mut app = hint_app();
        app.notify_warning("Clipboard is empty");
        let line = prompt_row_right_line(&app, SLOT);
        assert_eq!(text_of(&line), "! Clipboard is empty");
        assert_eq!(color_of(&line), Some(Color::Yellow));
    }

    #[test]
    fn error_outranks_a_queued_warning() {
        let mut app = hint_app();
        app.notify_warning("Clipboard is empty");
        app.notify_error("Tool error: boom");
        assert!(text_of(&prompt_row_right_line(&app, SLOT)).starts_with("x "));

        // Dismissing the error falls back to the still-queued warning.
        app.notifications.dismiss_errors();
        assert!(text_of(&prompt_row_right_line(&app, SLOT)).starts_with("! "));
    }

    #[test]
    fn long_notification_is_truncated_to_the_slot() {
        let mut app = hint_app();
        app.notify_error(format!("Tool error: {}", "boom ".repeat(40)));
        let text = text_of(&prompt_row_right_line(&app, SLOT));
        assert!(
            UnicodeWidthStr::width(text.as_str()) <= SLOT,
            "fits the slot: {text:?}"
        );
        assert!(text.ends_with('\u{2026}'), "ellipsised: {text:?}");
    }

    #[test]
    fn status_message_alone_stays_out_of_the_row() {
        // Info-level statuses belong to the status row above the box; only
        // notifications are painted into this slot.
        let mut app = hint_app();
        app.status_message = Some("Conversation cleared.".to_string());
        assert_eq!(text_of(&prompt_row_right_line(&app, SLOT)), "? shortcuts");
    }
}

/// Inline system annotations: `Error` gets the red API-error block, the other
/// styles keep the centred rule.
#[cfg(test)]
mod system_annotation_tests {
    use super::*;
    use crate::app::{SystemAnnotation, SystemMessageStyle};

    fn annotation(style: SystemMessageStyle) -> SystemAnnotation {
        SystemAnnotation {
            after_index: 0,
            text: "Provider stream error: request timed out after 60s".to_string(),
            style,
        }
    }

    fn render(style: SystemMessageStyle) -> String {
        let mut lines = Vec::new();
        render_system_annotation_lines(&mut lines, &annotation(style), 80);
        lines
            .iter()
            .map(flatten_line_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn error_annotation_renders_the_api_error_block() {
        let text = render(SystemMessageStyle::Error);
        assert!(text.contains("API Error"), "block header: {text:?}");
        assert!(text.contains("timed out after 60s"), "message kept: {text:?}");
    }

    #[test]
    fn warning_annotation_keeps_the_inline_rule() {
        let text = render(SystemMessageStyle::Warning);
        assert!(text.contains("timed out after 60s"), "message kept: {text:?}");
        assert!(!text.contains("API Error"), "no block for warnings: {text:?}");
    }
}
