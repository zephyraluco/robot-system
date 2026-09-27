//! Transcript operations: add/replace/push, scroll, notifications.

use std::time::Duration;

use claurst_core::types::{Message, Role};
use super::App;
use super::types::{SystemAnnotation, SystemMessageStyle};
use crate::notifications::NotificationKind;

impl App {
    /// Add a message directly (e.g. from a non-streaming source).
    pub fn add_message(&mut self, role: Role, text: String) {
        let msg = match role {
            Role::User => Message::user(text),
            Role::Assistant => Message::assistant(text),
        };
        if role == Role::User {
            self.begin_user_turn_snapshot();
        }
        self.messages.push(msg);
        self.invalidate_transcript();
        self.on_new_message();
    }

    pub fn replace_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        self.sync_turn_metadata_to_messages();
        self.invalidate_transcript();
    }

    pub fn push_message(&mut self, message: Message) {
        if message.role == Role::User {
            self.begin_user_turn_snapshot();
        }
        self.messages.push(message);
        self.sync_turn_metadata_to_messages();
        self.invalidate_transcript();
        self.on_new_message();
    }

    /// Push a synthetic system annotation into the conversation pane.
    /// It will appear after the current last message.
    pub fn push_system_message(&mut self, text: String, style: SystemMessageStyle) {
        self.system_annotations.push(SystemAnnotation {
            after_index: self.messages.len(),
            text,
            style,
        });
        self.invalidate_transcript();
    }

    /// Called whenever a new message is appended to `messages`.
    /// Manages the auto-scroll / new-message-counter state.
    pub(super) fn on_new_message(&mut self) {
        if self.auto_scroll {
            // Auto-scroll: keep offset at 0 so render shows the bottom.
            self.scroll_offset = 0;
        } else {
            self.new_messages_while_scrolled =
                self.new_messages_while_scrolled.saturating_add(1);
        }
    }

    pub fn invalidate_transcript(&self) {
        self.transcript_version
            .set(self.transcript_version.get().wrapping_add(1));
    }

    /// Take the current input buffer, push it to history, and return it.
    pub fn take_input(&mut self) -> String {
        let input = self.prompt_input.take();
        if !input.is_empty() {
            self.prompt_input.history.push(input.clone());
            self.prompt_input.history_pos = None;
            self.prompt_input.history_draft.clear();
        }
        self.refresh_prompt_input();
        input
    }

    /// Scroll the transcript up by `amount` lines and disable auto-follow.
    ///
    /// `scroll_offset` counts lines above the bottom (0 = pinned to the newest
    /// content). It is clamped to `last_max_scroll` — the maximum meaningful
    /// offset from the last render — so scrolling up past the top of the
    /// transcript can't inflate it unboundedly. Without the clamp, an over-scroll
    /// would leave `scroll_offset` far above `max_scroll`, and the user would
    /// have to press Down that many times before the view moved (#223).
    pub(super) fn scroll_up_by(&mut self, amount: usize) {
        self.scroll_offset = self
            .scroll_offset
            .saturating_add(amount)
            .min(self.last_max_scroll.get());
        self.auto_scroll = false;
    }

    /// Compute the number of lines to scroll per wheel/trackpad event.
    /// Implements a simple acceleration model: rapid events (< 40 ms apart) are
    /// treated as trackpad bursts and accelerate up to 2×; slower events (mouse
    /// wheel) stay at the base 3-line step.
    pub(super) fn scroll_step(&mut self) -> usize {
        let now = std::time::Instant::now();
        let elapsed_ms = self.scroll_last_time
            .map(|t| now.duration_since(t).as_millis())
            .unwrap_or(u128::MAX);
        self.scroll_last_time = Some(now);
        if elapsed_ms < 40 {
            // Trackpad burst — gradually accelerate
            self.scroll_accel = (self.scroll_accel + 0.4).min(6.0);
        } else {
            // Mouse click or first event — reset to base
            self.scroll_accel = 3.0;
        }
        self.scroll_accel.round() as usize
    }
}

// ---------------------------------------------------------------------------
// Notifications (warnings / errors for the prompt row)
// ---------------------------------------------------------------------------

/// How long a warning stays on the prompt row before it fades out. Errors get no
/// lifetime at all — they stay until the next prompt is submitted.
const WARNING_TTL: Duration = Duration::from_secs(5);

/// Longest message kept in the queue. The prompt row shows one truncated line,
/// and the entry survives until the next prompt, so there is no point holding on
/// to a whole tool result.
const MAX_MESSAGE_CHARS: usize = 200;

impl App {
    /// Queue a notification for the prompt row.
    ///
    /// `lifetime` of `None` keeps it until the next submitted prompt clears it.
    pub fn push_notification(
        &mut self,
        kind: NotificationKind,
        message: impl Into<String>,
        lifetime: Option<Duration>,
    ) {
        self.notifications.push(kind, headline(&message.into()), lifetime);
    }

    /// Report a failure — red, and visible until the next prompt is submitted.
    pub fn notify_error(&mut self, message: impl Into<String>) {
        self.notify(NotificationKind::Error, message);
    }

    /// Report a warning — yellow, fading out after [`WARNING_TTL`].
    pub fn notify_warning(&mut self, message: impl Into<String>) {
        self.notify(NotificationKind::Warning, message);
    }

    /// Queue a notification using the default lifetime for its severity.
    pub fn notify(&mut self, kind: NotificationKind, message: impl Into<String>) {
        let lifetime = match kind {
            NotificationKind::Error => None,
            NotificationKind::Warning => Some(WARNING_TTL),
        };
        self.push_notification(kind, message, lifetime);
    }
}

/// Collapse `message` to a single line and cap its length.
///
/// Callers pass user-facing text that can contain raw tool output or a provider
/// error body (newlines, megabytes). The prompt row is one line wide, so the
/// headline is what matters; the full text stays wherever it was already
/// reported.
fn headline(message: &str) -> String {
    let one_line = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_MESSAGE_CHARS {
        return one_line;
    }
    let mut capped: String = one_line.chars().take(MAX_MESSAGE_CHARS).collect();
    capped.push('\u{2026}');
    capped
}

#[cfg(test)]
mod notification_tests {
    use super::*;
    use claurst_core::config::Config;
    use claurst_core::cost::CostTracker;

    fn app() -> App {
        App::new(Config::default(), CostTracker::new())
    }

    #[test]
    fn errors_persist_while_warnings_expire() {
        let mut app = app();
        // A zero lifetime stands in for "the timer already elapsed".
        app.push_notification(NotificationKind::Warning, "warn", Some(Duration::ZERO));
        app.notify_error("boom");
        assert_eq!(app.notifications.current().unwrap().message, "boom");

        // The expired warning is pruned, the persistent error survives.
        app.notifications.tick();
        assert_eq!(app.notifications.current().unwrap().message, "boom");

        app.notifications.dismiss_errors();
        assert!(app.notifications.is_empty());
    }

    #[test]
    fn notify_warning_gets_a_bounded_lifetime() {
        let mut app = app();
        app.notify_warning("warn");
        assert!(app.notifications.current().unwrap().expires_at.is_some());
    }

    #[test]
    fn headline_collapses_newlines_and_caps_length() {
        assert_eq!(headline("line one\n\n   line two"), "line one line two");
        let long = "x".repeat(MAX_MESSAGE_CHARS + 50);
        let capped = headline(&long);
        assert_eq!(capped.chars().count(), MAX_MESSAGE_CHARS + 1);
        assert!(capped.ends_with('\u{2026}'));
    }
}
