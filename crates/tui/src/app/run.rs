//! Query events, jump-to-error, file open.

use claurst_core::types::Message;
use claurst_core::{sample_completion_verb, sample_spinner_verb};
use claurst_query::QueryEvent;
use tracing::debug;
use super::App;
use super::turns::format_elapsed_ms;
use super::types::{ToolStatus, ToolUseBlock};

pub(super) fn open_file_externally(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    // Try to open with the system's default application
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(&["/C", "start", ""])
            .arg(path)
            .spawn()?;
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        // Fallback for other systems: try common editors in order
        for editor in &["nano", "vi", "vim", "emacs"] {
            match std::process::Command::new(editor)
                .arg(path)
                .spawn()
            {
                Ok(_) => return Ok(()),
                Err(_) => continue,
            }
        }
        Err("No suitable editor found".into())
    }
}

impl App {
    /// Push a completed assistant message and trigger auto-scroll bookkeeping.
    pub(super) fn push_assistant_message(&mut self, text: String) {
        let msg = Message::assistant(text);
        self.messages.push(msg);
        self.invalidate_transcript();
        self.on_new_message();
    }

    /// Process a query event from the agentic loop.
    pub fn handle_query_event(&mut self, event: QueryEvent) {
        match event {
            QueryEvent::Stream(stream_evt) => {
                if !self.is_streaming {
                    let seed = self.frame_count as usize ^ (self.messages.len() * 17);
                    self.spinner_verb = Some(sample_spinner_verb(seed).to_string());
                    // turn_start is set in begin_user_turn_snapshot (prompt
                    // submission time).  Only fall back here if somehow no
                    // user message was pushed before streaming began (e.g.
                    // headless / programmatic callers).
                    if self.turn_start.is_none() {
                        self.turn_start = Some(std::time::Instant::now());
                    }
                    self.streaming_thinking.clear();
                }
                self.is_streaming = true;
                match stream_evt {
                    claurst_api::AnthropicStreamEvent::ContentBlockDelta { delta, .. } => {
                        // Reset stall timer on any incoming delta — we're making progress.
                        self.stall_start = None;
                        match delta {
                            claurst_api::streaming::ContentDelta::TextDelta { text } => {
                                self.streaming_text.push_str(&text);
                                self.invalidate_transcript();
                            }
                            claurst_api::streaming::ContentDelta::ThinkingDelta { thinking } => {
                                debug!(len = thinking.len(), "Thinking delta received");
                                self.streaming_thinking.push_str(&thinking);
                                self.invalidate_transcript();
                            }
                            _ => {}
                        }
                    }
                    claurst_api::AnthropicStreamEvent::MessageStop => {
                        self.is_streaming = false;
                        self.spinner_verb = None;
                        self.stall_start = None;
                        self.flush_streamed_assistant_message();
                    }
                    _ => {
                        // Any other stream event: if we have no stall_start yet,
                        // record now so the red-spinner timer can begin.
                        if self.stall_start.is_none() {
                            self.stall_start = Some(std::time::Instant::now());
                        }
                    }
                }
            }

            QueryEvent::ToolStart { tool_name, tool_id, input_json } => {
                if !self.is_streaming && self.spinner_verb.is_none() {
                    let seed = self.frame_count as usize ^ (self.messages.len() * 17);
                    self.spinner_verb = Some(sample_spinner_verb(seed).to_string());
                }
                self.is_streaming = true;
                self.status_message = Some(format!("Running {}…", tool_name));
                let turn_index = self.current_user_turn_index();
                if let Some(existing) =
                    self.tool_use_blocks.iter_mut().find(|b| b.id == tool_id)
                {
                    existing.turn_index = turn_index;
                    existing.status = ToolStatus::Running;
                    existing.output_preview = None;
                    existing.input_json = input_json;
                } else {
                    self.tool_use_blocks.push(ToolUseBlock {
                        id: tool_id,
                        name: tool_name,
                        turn_index,
                        status: ToolStatus::Running,
                        output_preview: None,
                        input_json,
                    });
                }
                self.invalidate_transcript();
            }

            QueryEvent::ToolEnd {
                tool_name: _,
                tool_id,
                result,
                is_error,
            } => {
                // Build a multi-line preview: show up to 3 lines, truncate if more.
                let all_lines: Vec<&str> = result.lines().collect();
                let preview_lines = all_lines.len().min(3);
                let mut preview = all_lines[..preview_lines].join("\n");
                let remaining = all_lines.len().saturating_sub(preview_lines);
                if remaining > 0 {
                    preview.push_str(&format!("\n\u{2026} {} more lines", remaining));
                }
                if let Some(block) =
                    self.tool_use_blocks.iter_mut().find(|b| b.id == tool_id)
                {
                    block.status = if is_error {
                        ToolStatus::Error
                    } else {
                        ToolStatus::Done
                    };
                    block.output_preview = Some(preview);
                }
                self.invalidate_transcript();
                if is_error {
                    // The transcript keeps the full tool result, and the corner
                    // carries the headline — repeating it in the dim status row
                    // would just be a second copy of the same line.
                    self.status_message = None;
                    self.notify_error(format!("Tool error: {}", result));
                } else {
                    self.status_message = None;
                }
                self.refresh_turn_diff_from_history();
            }

            QueryEvent::TurnComplete { turn, stop_reason, usage, .. } => {
                debug!(turn, stop_reason, "Turn complete");
                self.is_streaming = false;
                self.spinner_verb = None;

                // Update context window usage from the usage info.
                if let Some(ref u) = usage {
                    let turn_tokens = u.input_tokens + u.output_tokens
                        + u.cache_creation_input_tokens + u.cache_read_input_tokens;
                    self.context_used_tokens = self.context_used_tokens.saturating_add(turn_tokens);
                }
                // Record elapsed time and pick a completion verb
                let seed = self.frame_count as usize ^ (self.messages.len() * 7);
                let elapsed = self.turn_start.take()
                    .map(|start| format_elapsed_ms(start.elapsed().as_millis()));
                self.last_turn_elapsed = Some(
                    elapsed.unwrap_or_else(|| "0s".to_string())
                );
                self.last_turn_verb = Some(sample_completion_verb(seed));
                self.flush_streamed_assistant_message();
                self.tool_use_blocks.retain(|b| b.status != ToolStatus::Running);
                self.complete_current_turn_snapshot(stop_reason.contains("abort") || stop_reason.contains("cancel"));
                self.invalidate_transcript();
                self.refresh_turn_diff_from_history();
            }

            QueryEvent::Status(msg) => {
                // Status text doubles as a notification source: retries, budget
                // stops and fallbacks belong in the prompt corner, while plain
                // progress ("Compacting context...") stays in the status row.
                if let Some(kind) = crate::notifications::NotificationKind::classify(&msg) {
                    self.notify(kind, msg.clone());
                }
                self.status_message = Some(msg);
            }

            QueryEvent::Error(msg) => {
                self.is_streaming = false;
                self.spinner_verb = None;
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.invalidate_transcript();
                let err_msg = format!("Error: {}", msg);
                self.notify_error(err_msg.clone());
                self.push_assistant_message(err_msg);
            }
            QueryEvent::TokenWarning { state, pct_used } => {
                // The footer already tracks context usage continuously; the
                // corner only speaks up once the window is genuinely running
                // out (≥80% = warning, ≥95% = error).
                use claurst_query::TokenWarningState;
                let pct = (pct_used * 100.0).round().clamp(0.0, 100.0) as u32;
                match state {
                    TokenWarningState::Warning => {
                        self.notify_warning(format!("{}% context used", pct))
                    }
                    TokenWarningState::Critical => {
                        self.notify_error(format!("{}% context used — run /compact", pct))
                    }
                    TokenWarningState::Ok => {}
                }
            }
        }

        // Update token count from tracker.
        self.token_count = self.cost_tracker.total_tokens() as u32;
    }

    /// Jump to the next error/issue in messages.
    /// Searches for common error indicators: "Error:", "ERROR:", "error", "failed", "FAIL".
    pub(super) fn jump_to_next_error(&mut self) {
        const ERROR_KEYWORDS: &[&str] = &["error:", "failed:", "fail"];

        // Search forward from current position
        for i in 0..self.messages.len() {
            let msg = &self.messages[i];
            let content = msg.get_all_text().to_lowercase();

            // Check if message contains error keywords
            let has_error = ERROR_KEYWORDS.iter().any(|keyword| {
                content.contains(keyword)
            });

            if has_error && i > (self.messages.len().saturating_sub(self.scroll_offset / 2)) {
                // Found an error message, scroll to it
                let new_offset = self.messages.len().saturating_sub(i);
                self.scroll_offset = new_offset.saturating_mul(2);
                self.auto_scroll = false;
                self.status_message = Some(format!("Error found in message {}", i + 1));
                return;
            }
        }

        self.status_message = Some("No more errors found.".to_string());
    }

    /// Jump to the previous error/issue in messages.
    /// Searches backwards for common error indicators.
    pub(super) fn jump_to_previous_error(&mut self) {
        const ERROR_KEYWORDS: &[&str] = &["error:", "failed:", "fail"];

        // Search backward from current position
        for i in (0..self.messages.len()).rev() {
            let msg = &self.messages[i];
            let content = msg.get_all_text().to_lowercase();

            // Check if message contains error keywords
            let has_error = ERROR_KEYWORDS.iter().any(|keyword| {
                content.contains(keyword)
            });

            if has_error && i < (self.messages.len().saturating_sub(self.scroll_offset / 2)) {
                // Found an error message, scroll to it
                let new_offset = self.messages.len().saturating_sub(i);
                self.scroll_offset = new_offset.saturating_mul(2);
                self.auto_scroll = false;
                self.status_message = Some(format!("Error found in message {}", i + 1));
                return;
            }
        }

        self.status_message = Some("No previous errors found.".to_string());
    }

}
