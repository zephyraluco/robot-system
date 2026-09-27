//! Pure data model: messages, tool blocks, dialogs, state structs.

/// Visual style for inline system messages in the conversation pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemMessageStyle {
    Info,
    Warning,
    /// Terminal failure the user must see (provider/API error, request
    /// timeout, model unavailable). Rendered as the red "API Error" block
    /// (`messages::render_system_api_error`), unlike the inline rule used by
    /// [`Self::Info`] / [`Self::Warning`].
    Error,
    /// Compact / auto-compact boundary marker.
    Compact,
}

/// A synthetic system annotation inserted between conversation messages.
/// `after_index` is the index in `App::messages` after which this annotation
/// should appear (0 = before all messages, 1 = after message 0, etc.).
#[derive(Debug, Clone)]
pub struct SystemAnnotation {
    pub after_index: usize,
    pub text: String,
    pub style: SystemMessageStyle,
}

/// What content the context menu is currently targeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuKind {
    /// A specific transcript message.
    Message { message_index: usize },
    /// The current text selection anywhere in the frame.
    Selection,
}

/// Available context menu items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuItem {
    Copy,
    Fork,
}

impl ContextMenuItem {
    /// Stable id used as the list picker's `SelectItem::id`.
    pub fn id(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Fork => "fork",
        }
    }

    /// Parse an id produced by [`Self::id`].
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "copy" => Some(Self::Copy),
            "fork" => Some(Self::Fork),
            _ => None,
        }
    }
}

/// Status of an active or completed tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Done,
    Error,
}

/// Represents an active or completed tool invocation visible in the UI.
#[derive(Debug, Clone)]
pub struct ToolUseBlock {
    pub id: String,
    pub name: String,
    pub turn_index: Option<usize>,
    pub status: ToolStatus,
    pub output_preview: Option<String>,
    /// JSON-serialised input for the tool call (populated from the API stream).
    pub input_json: String,
}

#[derive(Debug, Clone, Default)]
pub struct TurnMetadata {
    pub submitted_at: Option<String>,
    pub model_name: Option<String>,
    pub agent_mode: Option<String>,
    pub duration: Option<String>,
    pub interrupted: bool,
}

/// Which area of the TUI currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    /// Keyboard input goes to the prompt editor.
    Input,
    /// Keyboard input goes to the transcript/message pane (scroll, etc.).
    Transcript,
}

/// A lightweight record of a recent session, shown in the welcome screen's
/// "Recent activity" list.
///
/// Loaded asynchronously from `session_storage` (see `recent_sessions_pending`
/// in the run loop) so the render path never touches disk. Holds only what the
/// welcome box needs: a display label plus the transcript's modification time,
/// from which a relative timestamp ("2h ago") is computed at render time.
#[derive(Debug, Clone)]
pub struct RecentSession {
    /// Display label: the custom title, else a truncated last prompt, else
    /// `"(untitled)"`.
    pub label: String,
    /// Transcript modification time, used to derive a relative timestamp.
    pub mtime: std::time::SystemTime,
}

/// Build the display label for a recent session: prefer the custom title, fall
/// back to the first line of the last prompt (truncated), else `"(untitled)"`.
pub fn recent_session_label(title: Option<String>, last_prompt: Option<String>) -> String {
    /// Cap stored labels so a huge prompt never bloats `App` state; the render
    /// path truncates further to the column width.
    const MAX_LABEL: usize = 80;

    let pick = |s: String| -> Option<String> {
        // First non-empty line, trimmed.
        let line = s.lines().find(|l| !l.trim().is_empty())?.trim();
        if line.is_empty() {
            return None;
        }
        let truncated: String = line.chars().take(MAX_LABEL).collect();
        Some(truncated)
    };

    title
        .and_then(pick)
        .or_else(|| last_prompt.and_then(pick))
        .unwrap_or_else(|| "(untitled)".to_string())
}

