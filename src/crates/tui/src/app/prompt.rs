//! Prompt input state machine.

use crate::overlays::SelectorMessage;
use crate::prompt_input::InputMode;
use super::App;
use super::commands::PROMPT_SLASH_COMMANDS;

impl App {
    /// Open the rewind flow with the current message list converted to
    /// `SelectorMessage` entries.
    pub fn open_rewind_flow(&mut self) {
        let selector_msgs: Vec<SelectorMessage> = self
            .messages
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let text = m.get_all_text();
                let preview: String = text.chars().take(80).collect();
                let has_tool_use = !m.get_tool_use_blocks().is_empty();
                SelectorMessage {
                    idx: i,
                    role: format!("{:?}", m.role).to_lowercase(),
                    preview,
                    has_tool_use,
                }
            })
            .collect();
        self.rewind_flow.open(selector_msgs);
    }

    pub(super) fn prompt_mode(&self) -> InputMode {
        // Note: previously returned Readonly while streaming, but the prompt
        // now accepts input during streaming so the user can compose / queue
        // a follow-up message. Plan mode still wins.
        if self.plan_mode {
            InputMode::Plan
        } else {
            InputMode::Default
        }
    }

    pub(super) fn sync_legacy_prompt_fields(&mut self) {
        self.input = self.prompt_input.text.clone();
        self.cursor_pos = self.prompt_input.cursor;
        self.history_index = self.prompt_input.history_pos;
    }

    pub fn refresh_prompt_input(&mut self) {
        self.prompt_input.mode = self.prompt_mode();
        if self.file_injection_dialog.visible {
            // Don't update suggestions while the injection dialog is open.
            self.sync_legacy_prompt_fields();
            return;
        }
        let file_autocomplete_limit = self.config.file_autocomplete_limit;
        let file_autocomplete_show_hidden = self.config.file_autocomplete_show_hidden_files;
        self.prompt_input.update_suggestions(PROMPT_SLASH_COMMANDS, file_autocomplete_limit, file_autocomplete_show_hidden);
        self.sync_legacy_prompt_fields();
    }

    pub fn set_prompt_text(&mut self, text: String) {
        self.prompt_input.replace_text(text);
        self.refresh_prompt_input();
    }

    pub(super) fn clear_prompt(&mut self) {
        self.prompt_input.clear();
        self.refresh_prompt_input();
    }

    /// Handle Enter while a typeahead popup is open. Accepts the highlighted
    /// suggestion and returns whether the prompt should now be submitted.
    ///
    /// - Slash command: complete the highlighted command *and* run it in a
    ///   single Enter — the popup acts as a command menu, so a second Enter to
    ///   "run" it should not be required (issue #183). Returns `true`.
    /// - File reference: complete the path, append a space, and keep editing so
    ///   the user can continue the prompt. Returns `false`.
    /// - History recall (or anything else): complete and keep editing so the
    ///   recalled text isn't fired off unexpectedly. Returns `false`.
    ///
    /// Callers must only invoke this when a suggestion is actually selected.
    pub(super) fn accept_suggestion_for_submit(&mut self) -> bool {
        use crate::prompt_input::TypeaheadSource;
        let source = self
            .prompt_input
            .suggestion_index
            .and_then(|i| self.prompt_input.suggestions.get(i))
            .map(|s| s.source.clone());
        match source {
            Some(TypeaheadSource::SlashCommand) => {
                self.prompt_input.accept_suggestion();
                // Sync legacy mirror fields without recomputing suggestions, so
                // the just-completed command isn't re-suggested behind the popup.
                self.sync_legacy_prompt_fields();
                true
            }
            Some(TypeaheadSource::FileRef) => {
                self.prompt_input.accept_suggestion();
                self.prompt_input.insert_char(' ');
                self.refresh_prompt_input();
                false
            }
            _ => {
                self.prompt_input.accept_suggestion();
                self.refresh_prompt_input();
                false
            }
        }
    }

}
