//! Warning / error notifications, surfaced in the prompt box's top-right corner.
//!
//! Scope is deliberately narrow: only the two severities a user must not miss
//! are queued. Everything else keeps using the plain `status_message` row above
//! the prompt. Nothing is drawn here either — `render.rs::render_prompt_corners`
//! paints the current entry into the prompt box corner, so a notification can
//! never cover the transcript (the reason the original floating-toast version
//! was dropped in 62ebebf).
//!
//! Lifetime model:
//!
//! * **warnings** expire on their own — [`NotificationQueue::tick`] prunes them;
//! * **errors** persist until [`NotificationQueue::dismiss_errors`] runs, which
//!   the TUI does when the next prompt is submitted. A failure therefore stays
//!   on screen while the user reads it and disappears once they move on.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::style::{Color, Modifier, Style};

/// Severity of a notification — drives colour, marker and precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    Warning,
    Error,
}

impl NotificationKind {
    /// Best-effort severity for free-form status text (provider retries, budget
    /// stops, tool failures …). `None` means "plain progress" — those stay in
    /// the status row and are not promoted to the corner.
    ///
    /// The TUI carries a single `status_message: String` for every call site, so
    /// the wording is the only signal available; keep the table small and
    /// obvious rather than clever.
    pub fn classify(message: &str) -> Option<Self> {
        let m = message.trim().to_ascii_lowercase();
        if m.contains("error")
            || m.contains("failed")
            || m.contains("failure")
            || m.contains("unavailable")
            || m.contains("budget limit")
        {
            Some(NotificationKind::Error)
        } else if m.contains("warn") || m.contains("retrying") {
            Some(NotificationKind::Warning)
        } else {
            None
        }
    }

    /// Style used by the prompt corner: errors are bold red, warnings plain
    /// yellow (matching the rest of the TUI's critical / advisory split).
    pub fn style(&self) -> Style {
        match self {
            NotificationKind::Warning => Style::default().fg(Color::Yellow),
            NotificationKind::Error => {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            }
        }
    }

    /// Single-cell marker drawn in front of the message.
    ///
    /// Deliberately ASCII: `⚠` / `✗` are East Asian ambiguous-width, and a
    /// terminal that renders them two cells wide desyncs the right-aligned
    /// corner (same reason `tool_icon` in `render.rs` is ASCII).
    pub fn marker(&self) -> &'static str {
        match self {
            NotificationKind::Warning => "!",
            NotificationKind::Error => "x",
        }
    }
}

/// One queued notification.
#[derive(Debug, Clone)]
pub struct Notification {
    pub kind: NotificationKind,
    pub message: String,
    /// `None` = stays until dismissed (errors), `Some` = expires (warnings).
    pub expires_at: Option<Instant>,
}

/// The active notifications, oldest first.
#[derive(Debug, Default)]
pub struct NotificationQueue {
    entries: VecDeque<Notification>,
}

impl NotificationQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue `message`, shown until `lifetime` elapses (`None` = persistent).
    ///
    /// At most one entry per severity is kept: the corner shows a single line, so
    /// a second warning would only ever hide the first behind its own expiry.
    /// Re-pushing an identical message refreshes the existing entry instead of
    /// duplicating it — a tool failing in a loop must not flood the queue.
    pub fn push(
        &mut self,
        kind: NotificationKind,
        message: impl Into<String>,
        lifetime: Option<Duration>,
    ) {
        let message = message.into();
        let expires_at = lifetime.map(|d| Instant::now() + d);

        // Same wording from a different severity: the newer report wins.
        self.entries.retain(|n| n.message != message);
        if let Some(existing) = self.entries.iter_mut().find(|n| n.kind == kind) {
            existing.message = message;
            existing.expires_at = expires_at;
            return;
        }
        self.entries.push_back(Notification { kind, message, expires_at });
    }

    /// The entry the prompt corner should show: errors outrank warnings, and
    /// within a severity the newest entry wins.
    pub fn current(&self) -> Option<&Notification> {
        self.entries
            .iter()
            .rev()
            .find(|n| n.kind == NotificationKind::Error)
            .or_else(|| {
                self.entries
                    .iter()
                    .rev()
                    .find(|n| n.kind == NotificationKind::Warning)
            })
    }

    /// Drop every error — called when the user submits the next prompt.
    pub fn dismiss_errors(&mut self) {
        self.entries.retain(|n| n.kind != NotificationKind::Error);
    }

    /// Drop expired warnings. Called once per frame from the event loop.
    pub fn tick(&mut self) {
        let now = Instant::now();
        self.entries.retain(|n| n.expires_at.is_none_or(|at| at > now));
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of queued entries (at most one per severity).
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_then_current() {
        let mut q = NotificationQueue::new();
        assert!(q.current().is_none());
        q.push(NotificationKind::Warning, "slow", Some(Duration::from_secs(5)));
        assert_eq!(q.current().unwrap().message, "slow");
    }

    #[test]
    fn errors_outrank_warnings() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Warning, "warn", Some(Duration::from_secs(5)));
        q.push(NotificationKind::Error, "boom", None);
        assert_eq!(q.current().unwrap().message, "boom");
        assert_eq!(q.current().unwrap().kind, NotificationKind::Error);
    }

    #[test]
    fn warning_is_shown_when_no_error_is_queued() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Error, "boom", None);
        q.dismiss_errors();
        q.push(NotificationKind::Warning, "warn", None);
        assert_eq!(q.current().unwrap().message, "warn");
    }

    #[test]
    fn at_most_one_entry_per_severity() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Warning, "first", None);
        q.push(NotificationKind::Warning, "second", None);
        assert_eq!(q.len(), 1);
        assert_eq!(q.current().unwrap().message, "second");
    }

    #[test]
    fn duplicate_message_refreshes_instead_of_stacking() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Warning, "same", Some(Duration::from_secs(1)));
        q.push(NotificationKind::Warning, "same", None);
        assert_eq!(q.len(), 1);
        // The refresh replaced the short lifetime with the persistent one.
        q.tick();
        assert_eq!(q.current().unwrap().message, "same");
    }

    #[test]
    fn same_message_from_the_other_severity_replaces_it() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Warning, "boom", None);
        q.push(NotificationKind::Error, "boom", None);
        assert_eq!(q.len(), 1);
        assert_eq!(q.current().unwrap().kind, NotificationKind::Error);
    }

    #[test]
    fn tick_drops_expired_warnings_and_keeps_persistent_errors() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Warning, "gone", Some(Duration::ZERO));
        q.push(NotificationKind::Error, "stays", None);
        q.tick();
        assert_eq!(q.len(), 1);
        assert_eq!(q.current().unwrap().message, "stays");
    }

    #[test]
    fn dismiss_errors_clears_the_corner() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Error, "boom", None);
        q.dismiss_errors();
        assert!(q.is_empty());
    }

    #[test]
    fn classify_matches_the_wording_used_by_call_sites() {
        // Errors.
        assert_eq!(
            NotificationKind::classify("Tool error: boom"),
            Some(NotificationKind::Error)
        );
        assert_eq!(
            NotificationKind::classify("Error: overloaded_error (529)"),
            Some(NotificationKind::Error)
        );
        assert_eq!(
            NotificationKind::classify("Stream error \u{2014} retrying (2 left)\u{2026}"),
            Some(NotificationKind::Error)
        );
        assert_eq!(
            NotificationKind::classify("Budget limit $1.0000 exceeded \u{2014} stopping."),
            Some(NotificationKind::Error)
        );
        // Warnings.
        assert_eq!(
            NotificationKind::classify("No response for 45s \u{2014} retrying (2 left)\u{2026}"),
            Some(NotificationKind::Warning)
        );
        assert_eq!(
            NotificationKind::classify("Warning: home directory is read-only"),
            Some(NotificationKind::Warning)
        );
        // Plain progress stays in the status row.
        assert_eq!(NotificationKind::classify("Conversation cleared."), None);
        assert_eq!(NotificationKind::classify("Running Bash\u{2026}"), None);
        assert_eq!(NotificationKind::classify("Compacting context..."), None);
        assert_eq!(NotificationKind::classify("Accomplishing\u{2026}"), None);
    }
}
