//! Keyboard input: normalization and key-event dispatch.

use claurst_core::keybindings::{KeyContext, KeybindingResult, ParsedKeystroke};
use crate::input::normalize_char_with_shift;
use crate::prompt_input::VimMode;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use super::App;
use super::types::FocusTarget;

/// Map a character to its QWERTY Latin keyboard-position equivalent.
///
/// When a modifier key (Ctrl, Alt) is held together with a non-ASCII character
/// (e.g. Cyrillic С on a Ukrainian/Russian layout), the char produced by
/// crossterm is the non-Latin glyph rather than the Latin letter that occupies
/// the same physical key.  Keybinding strings are always written as Latin
/// letters (`ctrl+c`, `alt+b`, …), so the lookup fails.
///
/// This function converts the reported character to the Latin letter that sits
/// at the same physical QWERTY position, covering the standard Russian JCUKEN
/// and Ukrainian layouts which share the same physical-key→Latin mapping.
/// For characters outside any known mapping the original (lowercased) char is
/// returned unchanged — this is always safe since unrecognised chars just
/// produce no keybinding match.
pub(super) fn layout_to_latin(c: char) -> String {
    // Standard Russian/Ukrainian JCUKEN → QWERTY position mapping.
    // Both upper- and lower-case Cyrillic variants are covered by
    // converting to lowercase first.
    let lower = c.to_lowercase().next().unwrap_or(c);
    let mapped: Option<char> = match lower {
        // Row 1
        'й' => Some('q'), 'ц' => Some('w'), 'у' => Some('e'),
        'к' => Some('r'), 'е' => Some('t'), 'н' => Some('y'),
        'г' => Some('u'), 'ш' => Some('i'), 'щ' => Some('o'),
        'з' => Some('p'),
        // Row 2
        'ф' => Some('a'), 'ы' => Some('s'), 'в' => Some('d'),
        'а' => Some('f'), 'п' => Some('g'), 'р' => Some('h'),
        'о' => Some('j'), 'л' => Some('k'), 'д' => Some('l'),
        // Row 3
        'я' => Some('z'), 'ч' => Some('x'), 'с' => Some('c'),
        'м' => Some('v'), 'и' => Some('b'), 'т' => Some('n'),
        'ь' => Some('m'),
        // Ukrainian-specific letters on standard positions
        'і' => Some('s'), 'ї' => Some(']'), 'є' => Some('\''),
        _ => None,
    };
    mapped.unwrap_or(lower).to_string()
}

/// The shift normalization lives in [`crate::input::normalize_char_with_shift`]
/// so the DialogCore-based dialogs can share it.
pub(super) fn key_event_to_keystroke(key: &KeyEvent) -> Option<ParsedKeystroke> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt  = key.modifiers.contains(KeyModifiers::ALT);

    let normalized_key = match key.code {
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Delete    => "delete".to_string(),
        KeyCode::Down      => "down".to_string(),
        KeyCode::End       => "end".to_string(),
        KeyCode::Enter     => "enter".to_string(),
        KeyCode::Esc       => "escape".to_string(),
        KeyCode::Home      => "home".to_string(),
        KeyCode::Left      => "left".to_string(),
        KeyCode::PageDown  => "pagedown".to_string(),
        KeyCode::PageUp    => "pageup".to_string(),
        KeyCode::Right     => "right".to_string(),
        KeyCode::Tab       => "tab".to_string(),
        KeyCode::Up        => "up".to_string(),
        KeyCode::BackTab   => "tab".to_string(),
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(c) => {
            // For modifier-key combos (Ctrl/Alt + letter), normalize to the
            // ASCII Latin key at the same physical QWERTY position.  This
            // makes shortcuts like Ctrl+C work regardless of the active
            // keyboard layout (Ukrainian, Russian, Greek, …).
            if (ctrl || alt) && !c.is_ascii() {
                layout_to_latin(c)
            } else {
                c.to_lowercase().to_string()
            }
        }
        _ => return None,
    };

    Some(ParsedKeystroke {
        key: normalized_key,
        ctrl,
        alt,
        shift: key.modifiers.contains(KeyModifiers::SHIFT),
        meta: key.modifiers.contains(KeyModifiers::SUPER),
    })
}

/// Rewrite a Ctrl-modified keystroke that carries a non-ASCII character to the
/// Latin letter at the same physical QWERTY position.
///
/// A few core shortcuts — most importantly Ctrl+C (interrupt / exit) and Ctrl+D
/// (exit) — are matched directly against `KeyEvent::code` in `handle_key_event`
/// rather than going through the keybinding table (they are intentionally absent
/// from `default_bindings`, see `NON_REBINDABLE`). On a non-Latin layout
/// (Ukrainian / Russian JCUKEN, …) the reported character is the Cyrillic glyph
/// at that physical key — e.g. Ctrl+С arrives as `Char('с')` — so the literal
/// `KeyCode::Char('c')` arms never fire and the shortcut is dead.
///
/// Normalizing once at the top of `handle_key_event` lets every downstream
/// `key.code` comparison (and the keybinding layer, idempotently) see the Latin
/// letter, mirroring what `key_event_to_keystroke` already does for bound keys.
///
/// Restricted to **pure Ctrl (Ctrl without Alt)** on purpose: Ctrl+<letter>
/// never produces literal text, so rewriting it cannot corrupt text entry,
/// whereas Alt / AltGr (reported as Ctrl+Alt) is used to compose characters on
/// some layouts and must be left untouched. Characters with no known
/// position mapping (or that map to a non-ASCII result) are returned unchanged.
pub(super) fn normalize_layout_shortcut_key(key: KeyEvent) -> KeyEvent {
    if let KeyCode::Char(c) = key.code {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if ctrl && !alt && !c.is_ascii() {
            if let Some(latin) = layout_to_latin(c).chars().next() {
                if latin.is_ascii() {
                    return KeyEvent {
                        code: KeyCode::Char(latin),
                        ..key
                    };
                }
            }
        }
    }
    key
}

impl App {
    /// Resolve the character to insert for a printable key press, applying the
    /// US-QWERTY shift map only when the kitty keyboard protocol is active.
    ///
    /// On terminals that do NOT speak the kitty protocol (Windows conhost / CMD
    /// / legacy PowerShell and most default terminals) the character is already
    /// final and layout-correct — Shift has been applied by the OS — so we pass
    /// it through untouched. Re-shifting it here would double-shift and corrupt
    /// input, e.g. turning a literal `/` (typed via Shift on many non-US
    /// layouts) into `?` (issue #183).
    pub(super) fn shift_normalize(&self, c: char, modifiers: KeyModifiers) -> char {
        if self.kitty_keyboard_active {
            normalize_char_with_shift(c, modifiers)
        } else {
            c
        }
    }

    /// Process a keyboard event. Returns `true` when the input should be
    /// submitted (Enter pressed with no blocking dialog).
    ///
    /// Keyboard input has exactly two layers:
    ///
    /// 1. **Modal layer** — while any modal is visible it owns the keyboard
    ///    unconditionally (`crate::dialogs::modal_keys::handle_modal_key`).
    ///    Modal key handling lives with the modals, never here.
    /// 2. **Main UI layer** — prompt editing, scrolling and global shortcuts
    ///    (`handle_main_key`), reached only when no modal is visible.
    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        // Make Ctrl shortcuts layout-independent before any handler runs: on
        // non-Latin layouts (Ukrainian / Russian, …) a Ctrl combo reports the
        // Cyrillic glyph at the physical key, which would otherwise miss the
        // literal `KeyCode::Char(..)` arms in the main-UI handler — including
        // Ctrl+C / Ctrl+D, which are matched there rather than via the
        // keybinding table (issue #47).
        let key = normalize_layout_shortcut_key(key);

        match crate::dialogs::modal_keys::handle_modal_key(self, key) {
            crate::dialogs::modal_keys::ModalKeyOutcome::NoModal => {}
            crate::dialogs::modal_keys::ModalKeyOutcome::Consumed => return false,
            crate::dialogs::modal_keys::ModalKeyOutcome::Submit => return true,
        }

        self.handle_main_key(key)
    }

    /// Main-UI key handling. Only reached when NO modal is visible: every modal
    /// consumes its keys in `crate::dialogs::modal_keys` first, so nothing below
    /// needs to know that modals exist.
    fn handle_main_key(&mut self, key: KeyEvent) -> bool {
        // Plugin hint dismiss: the banner is not a modal, so it is dismissed
        // here while the main UI owns the keyboard.
        if key.code == KeyCode::Esc {
            if let Some(hint) = self.plugin_hints.iter_mut().find(|h| h.is_visible()) {
                hint.dismiss();
                return false;
            }
        }

        // ---- Keybinding processor (runs AFTER the modal layer) -------------
        // The modal layer already returned for every modal context, so this is
        // always the chat context.
        let key_context = KeyContext::Chat;
        if let Some(keystroke) = key_event_to_keystroke(&key) {
            let had_pending_chord = self.keybindings.has_pending_chord();
            match self.keybindings.process(keystroke, &key_context) {
                KeybindingResult::Action(action) => {
                    return self.handle_keybinding_action(&action);
                }
                KeybindingResult::Pending => return false,
                KeybindingResult::NoMatch if had_pending_chord => return false,
                KeybindingResult::Unbound | KeybindingResult::NoMatch => {
                    // Fall through to hardcoded keybinding handlers
                }
            }
        } else {
            self.keybindings.cancel_chord();
        }

        // Clear any active text selection on key press (except Ctrl+C which copies it).
        let is_copy = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        if !is_copy && self.selection_anchor.is_some() {
            self.selection_anchor = None;
            self.selection_focus = None;
            *self.selection_text.borrow_mut() = String::new();
        }

        // ---- Ctrl+V / Cmd+V — clipboard paste (image first, then text fallback) ----
        // Only fires when NOT in vim Normal/Visual/VisualBlock mode (where \x16 is
        // already consumed by the vim handler above to enter VisualBlock mode).
        if key.code == KeyCode::Char('v')
            && (key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER))
            && !matches!(
                self.prompt_input.vim_mode,
                crate::prompt_input::VimMode::Normal
                    | crate::prompt_input::VimMode::Visual
                    | crate::prompt_input::VimMode::VisualBlock
            )
        {
            use crate::image_paste::{read_clipboard_image, read_clipboard_text, read_primary_text};
            if let Some(img) = read_clipboard_image() {
                self.prompt_input.add_image(img);
            } else if let Some(text) = read_clipboard_text().or_else(read_primary_text) {
                self.handle_paste_data(text);
                self.refresh_prompt_input();
            } else {
                // Nothing to paste — say so instead of silently ignoring Ctrl+V.
                self.notify_warning("Clipboard is empty");
            }
            return false;
        }

        // ---- Shift+Insert — selection/clipboard paste fallback -------------
        if key.code == KeyCode::Insert && key.modifiers.contains(KeyModifiers::SHIFT) {
            let _ = self.paste_primary_into_prompt();
            return false;
        }

        // ---- Focus state machine: transcript mode --------------------------
        // When the transcript pane has focus, intercept Escape and scroll keys.
        // Printable characters switch focus back to Input and fall through so the
        // keystroke is processed normally by the prompt editor below.
        if self.focus == FocusTarget::Transcript {
            match key.code {
                KeyCode::Esc => {
                    self.focus = FocusTarget::Input;
                    return false;
                }
                KeyCode::PageUp | KeyCode::PageDown => {
                    // Let these fall through to the normal scroll handling below.
                }
                KeyCode::Char(_) if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    // Printable char: switch focus to Input and process normally.
                    self.focus = FocusTarget::Input;
                }
                _ => {}
            }
        }

        match key.code {
            // ---- ESC: cancel streaming (status bar advertises "esc interrupt") ----
            KeyCode::Esc if self.is_streaming => {
                self.is_streaming = false;
                self.spinner_verb = None;
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.tool_use_blocks.clear();
                self.status_message = Some("Cancelled.".to_string());
                self.complete_current_turn_snapshot(true);
            }

            // ---- Quit / cancel ----------------------------------------
            // Accept both 'c' and 'C' so Shift+Ctrl+C also triggers copy
            // (issue #149 follow-up).
            KeyCode::Char(c) if (c == 'c' || c == 'C') && key.modifiers.contains(KeyModifiers::CONTROL) => {
                // If text is selected, copy it to clipboard instead of quitting.
                let sel_text = self.selection_text.borrow().clone();
                if self.selection_anchor.is_some() && !sel_text.is_empty() {
                    // Text is selected: copy to clipboard.
                    let _ = crate::image_paste::write_clipboard_text(&sel_text);
                    self.selection_anchor = None;
                    self.selection_focus = None;
                    *self.selection_text.borrow_mut() = String::new();
                } else if self.is_streaming {
                    // Cancel streaming.
                    self.is_streaming = false;
                    self.spinner_verb = None;
                    self.streaming_text.clear();
                    self.streaming_thinking.clear();
                    self.tool_use_blocks.clear();
                    self.status_message = Some("Cancelled.".to_string());
                    self.complete_current_turn_snapshot(true);
                } else {
                    // No text selected and not streaming: handle exit confirmation sequence.
                    // Always clear the prompt input on Ctrl+C.
                    if !self.prompt_input.is_empty() {
                        self.prompt_input.clear();
                        self.refresh_prompt_input();
                    }
                    self.handle_exit_key_confirmation('c');
                }
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+D on empty input: trigger two-press exit confirmation (like Ctrl+C).
                if self.prompt_input.is_empty() {
                    self.handle_exit_key_confirmation('d');
                }
            }

            // ---- Help overlay ------------------------------------------
            KeyCode::F(1) => {
                self.help_dialog.toggle();
            }
            KeyCode::Char('?')
                if !self.is_streaming
                    && self.prompt_input.is_empty()
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.help_dialog.toggle();
            }
            // With the kitty keyboard protocol, Shift+/ is reported as Char('/') with
            // SHIFT rather than Char('?'), so also accept that form for the help toggle.
            // This MUST be gated on the kitty protocol being active: on terminals that
            // don't speak it (Windows conhost / CMD / legacy PowerShell), a Char('/')
            // carrying a SHIFT flag is just a literal slash typed on a layout where `/`
            // is a shifted key — it must fall through to text entry so the user can
            // actually start a slash command (issue #183).
            KeyCode::Char('/')
                if self.kitty_keyboard_active
                    && key.modifiers.contains(KeyModifiers::SHIFT)
                    && !self.is_streaming
                    && self.prompt_input.is_empty()
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.help_dialog.toggle();
            }

            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.kill_line_backward();
                self.refresh_prompt_input();
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.kill_word_backward();
                self.refresh_prompt_input();
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.yank();
                self.refresh_prompt_input();
            }

            // ---- Alt/Meta key text editing operations -------------------
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.yank_pop();
                self.refresh_prompt_input();
            }
            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.delete_word_backward();
                self.refresh_prompt_input();
            }
            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.delete_word_backward();
                self.refresh_prompt_input();
            }
            KeyCode::Delete if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.delete_word_forward();
                self.refresh_prompt_input();
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.move_word_backward();
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.move_word_forward();
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.delete_word_at_cursor();
                self.refresh_prompt_input();
            }

            // ---- Text entry (allowed while streaming so users can queue
            // the next message; submission queues via Enter at the CLI layer).
            KeyCode::Char(c) => {
                let c = self.shift_normalize(c, key.modifiers);
                if self.prompt_input.vim_enabled && self.prompt_input.vim_mode != VimMode::Insert {
                    self.prompt_input.vim_command(&c.to_string());
                } else {
                    self.prompt_input.insert_char(c);
                }
                self.refresh_prompt_input();
            }
            KeyCode::Backspace => {
                self.prompt_input.backspace();
                self.refresh_prompt_input();
            }
            KeyCode::Delete if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.delete();
                self.refresh_prompt_input();
            }
            KeyCode::Delete if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.delete_word_forward();
                self.refresh_prompt_input();
            }
            KeyCode::Left => {
                if key.modifiers.contains(KeyModifiers::SUPER) {
                    self.prompt_input.cursor = 0;
                } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.prompt_input.move_word_backward();
                } else {
                    self.prompt_input.move_left();
                }
            }
            KeyCode::Right => {
                if key.modifiers.contains(KeyModifiers::SUPER) {
                    self.prompt_input.cursor = self.prompt_input.text.len();
                } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.prompt_input.move_word_forward();
                } else {
                    self.prompt_input.move_right();
                }
            }
            KeyCode::Home => {
                self.prompt_input.cursor = 0;
            }
            KeyCode::End => {
                self.prompt_input.cursor = self.prompt_input.text.len();
            }
            KeyCode::Tab => {
                if !self.prompt_input.suggestions.is_empty() {
                    // Accept slash-command suggestion. Allowed while streaming
                    // so the typeahead popup is interactive even when a turn
                    // is in flight — Enter then queues the completed command.
                    if self.prompt_input.suggestion_index.is_none() {
                        self.prompt_input.suggestion_index = Some(0);
                    }
                    self.prompt_input.accept_suggestion();
                    self.refresh_prompt_input();
                } else if !self.is_streaming && self.prompt_input.is_empty() {
                    // Cycle agent mode: build → plan → build
                    self.cycle_agent_mode();
                    self.rustle_look_down();
                }
            }

            // ---- Shift+Tab: cycle permission mode ----------------------
            // Default → AcceptEdits → BypassPermissions → Default
            // Mirrors TS bottom-left indicator cycling behaviour.
            KeyCode::BackTab if !self.is_streaming => {
                use claurst_core::config::PermissionMode;
                self.config.permission_mode = match self.config.permission_mode {
                    PermissionMode::Default => PermissionMode::AcceptEdits,
                    PermissionMode::AcceptEdits => PermissionMode::BypassPermissions,
                    PermissionMode::BypassPermissions => PermissionMode::Default,
                    PermissionMode::Plan => PermissionMode::Default,
                };
                let label = match self.config.permission_mode {
                    PermissionMode::Default => "Default permissions",
                    PermissionMode::AcceptEdits => "Accept-edits mode",
                    PermissionMode::BypassPermissions => "Bypass permissions (dangerous)",
                    PermissionMode::Plan => "Plan mode",
                };
                self.status_message = Some(label.to_string());
            }

            // ---- Submit ------------------------------------------------
            // Fallback newline insertion for when the keybinding layer doesn't
            // claim a modified Enter (e.g. Ctrl+Enter, or Shift/Alt+Enter after
            // the user unbinds them): Shift+Enter / Alt+Enter / Ctrl+Enter
            // insert a literal newline so users can compose multi-line prompts
            // before sending (issue #149 / #224). The authoritative bindings
            // live in claurst_core::keybindings (shift+enter, alt+enter, ctrl+j
            // → newline; enter → submit) and are handled above at the resolver.
            KeyCode::Enter
                if !self.is_streaming
                    && (key.modifiers.contains(KeyModifiers::SHIFT)
                        || key.modifiers.contains(KeyModifiers::ALT)
                        || key.modifiers.contains(KeyModifiers::CONTROL)) =>
            {
                self.prompt_input.insert_newline();
                self.refresh_prompt_input();
            }
            KeyCode::Enter if !self.is_streaming => {
                // Fallback Enter handling for when the keybinding layer doesn't
                // claim Enter (e.g. it's been unbound); the default path is the
                // "submit" keybinding action. If a typeahead popup is open, let
                // the shared helper decide whether to complete a suggestion or
                // also run it (issue #183).
                if !self.prompt_input.suggestions.is_empty()
                    && self.prompt_input.suggestion_index.is_some()
                    && !self.accept_suggestion_for_submit()
                {
                    return false;
                }
                // New user input: snap back to bottom.
                self.auto_scroll = true;
                self.new_messages_while_scrolled = 0;
                self.scroll_offset = 0;
                return true;
            }

            // ---- Message boundary navigation (Alt+Up/Alt+Down) ----------
            KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => {
                // Jump up by ~20 lines (approximate message boundary).
                self.scroll_up_by(20);
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => {
                // Jump down by ~20 lines (approximate message boundary).
                let new_off = self.scroll_offset.saturating_sub(20);
                self.scroll_offset = new_off;
                if new_off == 0 {
                    self.auto_scroll = true;
                    self.new_messages_while_scrolled = 0;
                }
            }

            // ---- Input history navigation ------------------------------
            // For multi-line / wrapped prompts: Up/Down move the cursor by
            // one visual row first, only falling through to history recall
            // when the cursor is already on the first/last visual row
            // (issue #149 follow-up).
            KeyCode::Up => {
                if !self.prompt_input.suggestions.is_empty() && (self.prompt_input.text.starts_with('/') || self.prompt_input.has_active_file_ref()) {
                    self.prompt_input.suggestion_prev();
                } else {
                    let area = self.last_input_area.get();
                    let width = area.width.saturating_sub(4) as usize;
                    let moved = !self.prompt_input.text.is_empty()
                        && self.prompt_input.move_visual_up(width);
                    if !moved && !self.prompt_input.history.is_empty() {
                        self.prompt_input.history_up();
                    }
                }
                self.refresh_prompt_input();
            }
            KeyCode::Down => {
                if !self.prompt_input.suggestions.is_empty() && (self.prompt_input.text.starts_with('/') || self.prompt_input.has_active_file_ref()) {
                    self.prompt_input.suggestion_next();
                } else {
                    let area = self.last_input_area.get();
                    let width = area.width.saturating_sub(4) as usize;
                    let moved = !self.prompt_input.text.is_empty()
                        && self.prompt_input.move_visual_down(width);
                    if !moved && self.prompt_input.history_pos.is_some() {
                        self.prompt_input.history_down();
                    }
                }
                self.refresh_prompt_input();
            }

            // ---- Scroll ------------------------------------------------
            KeyCode::PageUp => {
                // Scrolling up disables auto-follow (handled by scroll_up_by).
                self.scroll_up_by(10);
            }
            KeyCode::PageDown => {
                let new_off = self.scroll_offset.saturating_sub(10);
                self.scroll_offset = new_off;
                if new_off == 0 {
                    // Scrolled all the way back to bottom — re-enable auto-follow.
                    self.auto_scroll = true;
                    self.new_messages_while_scrolled = 0;
                }
            }

            // ---- Toggle last thinking block (t key) -------------------
            // (Removed: shadowed by KeyCode::Char(c) prompt input handler.)

            _ => {}
        }

        // Reset exit confirmation sequence if user presses any key other than Ctrl+C or Ctrl+D.
        let is_exit_key = key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char(c) if c == 'c' || c == 'd' || c == 'C' || c == 'D');
        if !is_exit_key {
            self.last_exit_key_warning = None;
        }

        false
    }

    pub(super) fn handle_exit_key_confirmation(&mut self, mut key_char: char) {
        // Check if we have an active warning within the timeout
        if let Some(warning_time) = self.last_exit_key_warning {
            if warning_time.elapsed().as_secs_f64() <= 2.0 {
                if self.exit_key_sequence_start == Some(key_char) {
                    // Matching key - exit
                    self.should_exit = true;
                    self.last_exit_key_warning = None;
                    self.exit_key_sequence_start = None;
                    return;
                }
                if let Some(other_key) = self.exit_key_sequence_start {
                    // Wrong key pressed - show message for the original key and reset timer
                    key_char = other_key;
                }
            }
        }

        // Start new sequence (or show message for wrong key)
        self.last_exit_key_warning = Some(std::time::Instant::now());
        self.exit_key_sequence_start = Some(key_char);
    }

    pub(super) fn handle_keybinding_action(&mut self, action: &str) -> bool {
        match action {
            "interrupt" => {
                if self.is_streaming {
                    self.is_streaming = false;
                    self.spinner_verb = None;
                    self.streaming_text.clear();
                    self.streaming_thinking.clear();
                    self.tool_use_blocks.clear();
                    self.status_message = Some("Cancelled.".to_string());
                } else {
                    // Handle exit confirmation: require two exit key presses within 2 seconds.
                    // Always clear the prompt input on Ctrl+C.
                    if !self.prompt_input.is_empty() {
                        self.prompt_input.clear();
                        self.refresh_prompt_input();
                    }

                    let elapsed = self.last_exit_key_warning.map(|t| t.elapsed().as_secs_f64());
                    let is_valid = elapsed.map(|e| e <= 2.0).unwrap_or(false);

                    if self.last_exit_key_warning.is_some() && is_valid {
                        // A warning is active and within 2 seconds: exit.
                        self.should_exit = true;
                        self.last_exit_key_warning = None;
                        self.exit_key_sequence_start = None;
                    } else {
                        // First press or timeout expired: start the exit sequence.
                        self.last_exit_key_warning = Some(std::time::Instant::now());
                        self.exit_key_sequence_start = Some('c');
                    }
                }
                false
            }
            "exit" => {
                if self.prompt_input.is_empty() {
                    self.should_exit = true;
                }
                false
            }
            "redraw" => false,
            "historySearch" => {
                // Ctrl+R history search was removed.
                false
            }
            "openSearch" => {
                // Ctrl+P global search was removed.
                false
            }
            "submit" => {
                if !self.is_streaming {
                    if !self.prompt_input.suggestions.is_empty()
                        && self.prompt_input.suggestion_index.is_some()
                    {
                        self.accept_suggestion_for_submit()
                    } else {
                        true
                    }
                } else {
                    false
                }
            }
            "historyPrev" => {
                // Suggestions (slash commands or file refs) take priority over cursor/history.
                if !self.prompt_input.suggestions.is_empty()
                    && (self.prompt_input.text.starts_with('/') || self.prompt_input.has_active_file_ref())
                {
                    self.prompt_input.suggestion_prev();
                    self.refresh_prompt_input();
                } else {
                    let width = self.last_input_area.get().width.saturating_sub(4) as usize;
                    let moved = !self.prompt_input.text.is_empty()
                        && self.prompt_input.move_visual_up(width);
                    if !moved && !self.prompt_input.history.is_empty() {
                        self.prompt_input.history_up();
                    }
                    self.refresh_prompt_input();
                }
                false
            }
            "historyNext" => {
                // Suggestions (slash commands or file refs) take priority over cursor/history.
                if !self.prompt_input.suggestions.is_empty()
                    && (self.prompt_input.text.starts_with('/') || self.prompt_input.has_active_file_ref())
                {
                    self.prompt_input.suggestion_next();
                    self.refresh_prompt_input();
                } else {
                    let width = self.last_input_area.get().width.saturating_sub(4) as usize;
                    let moved = !self.prompt_input.text.is_empty()
                        && self.prompt_input.move_visual_down(width);
                    if !moved && self.prompt_input.history_pos.is_some() {
                        self.prompt_input.history_down();
                    }
                    self.refresh_prompt_input();
                }
                false
            }
            "goLineStart" => {
                if !self.is_streaming {
                    self.prompt_input.cursor = 0;
                }
                false
            }
            "goLineEnd" => {
                if !self.is_streaming {
                    self.prompt_input.cursor = self.prompt_input.text.len();
                }
                false
            }
            "killToStart" => {
                if !self.is_streaming {
                    self.prompt_input.kill_line_backward();
                    self.refresh_prompt_input();
                }
                false
            }
            "killWord" => {
                if !self.is_streaming {
                    self.prompt_input.kill_word_backward();
                    self.refresh_prompt_input();
                }
                false
            }
            "expandPaste" => {
                // Alt+E: expand the [Pasted text #N ...] placeholder at the
                // cursor (or the first one in the buffer) so the full pasted
                // body is visible and editable in place. Allowed while
                // streaming — the prompt stays editable for composing queued
                // messages.
                if self.prompt_input.expand_paste_ref_at_cursor() {
                    self.refresh_prompt_input();
                }
                false
            }
            "scrollUp" => {
                self.scroll_up_by(10);
                false
            }
            "scrollDown" => {
                let new_off = self.scroll_offset.saturating_sub(10);
                self.scroll_offset = new_off;
                if new_off == 0 {
                    self.auto_scroll = true;
                    self.new_messages_while_scrolled = 0;
                }
                false
            }
            "yes" => {
                self.permission_request = None;
                false
            }
            "no" => {
                self.permission_request = None;
                false
            }
            "prevOption" => {
                if let Some(pr) = self.permission_request.as_mut() {
                    if pr.selected_option > 0 {
                        pr.selected_option -= 1;
                    }
                }
                false
            }
            "nextOption" => {
                if let Some(pr) = self.permission_request.as_mut() {
                    if pr.selected_option + 1 < pr.options.len() {
                        pr.selected_option += 1;
                    }
                }
                false
            }
            "close" => {
                self.help_dialog.close();
                false
            }
            "select" => false,
            "cancel" => false,
            "prevResult" => false,
            "nextResult" => false,
            // ========== NEW KEYBINDING ACTIONS (Phase 1) ==========
            "clearLine" => {
                // Ctrl+L: Clear the current input line (like bash Ctrl+L)
                if !self.is_streaming {
                    self.prompt_input.text.clear();
                    self.prompt_input.cursor = 0;
                    self.refresh_prompt_input();
                }
                false
            }
            "deleteCharBefore" => {
                // Ctrl+H: Delete character before cursor (backspace equivalent)
                if !self.is_streaming {
                    self.prompt_input.backspace();
                    self.refresh_prompt_input();
                }
                false
            }
            "previousMessage" => {
                // Alt+←: Navigate to previous message in transcript
                self.scroll_up_by(5);
                false
            }
            "nextMessage" => {
                // Alt+→: Navigate to next message in transcript
                let new_off = self.scroll_offset.saturating_sub(5);
                self.scroll_offset = new_off;
                if new_off == 0 {
                    self.auto_scroll = true;
                }
                false
            }
            "jumpToNextError" => {
                // Ctrl+.: Jump to next error/issue in messages
                self.jump_to_next_error();
                false
            }
            "jumpToPreviousError" => {
                // Ctrl+Shift+.: Jump to previous error/issue in messages
                self.jump_to_previous_error();
                false
            }
            "reverseIndent" => {
                // Shift+Tab: Reverse indent (cycle permission mode)
                use claurst_core::config::PermissionMode;
                self.config.permission_mode = match self.config.permission_mode {
                    PermissionMode::Default => PermissionMode::AcceptEdits,
                    PermissionMode::AcceptEdits => PermissionMode::BypassPermissions,
                    PermissionMode::BypassPermissions => PermissionMode::Default,
                    PermissionMode::Plan => PermissionMode::Default,
                };
                let label = match self.config.permission_mode {
                    PermissionMode::Default => "Default permissions",
                    PermissionMode::AcceptEdits => "Accept-edits mode",
                    PermissionMode::BypassPermissions => "Bypass permissions (dangerous)",
                    PermissionMode::Plan => "Plan mode",
                };
                self.status_message = Some(label.to_string());
                false
            }
            "openHelp" => {
                // Alt+H: Open help (alternative to F1)
                self.help_dialog.toggle();
                false
            }
            "openModelPicker" => {
                if !self.is_streaming {
                    self.intercept_slash_command("model");
                }
                false
            }
            "openCommandPalette" => {
                if !self.is_streaming {
                    self.command_palette.open();
                }
                false
            }
            "deleteWord" => {
                // Alt+D: Delete word forward
                if !self.is_streaming {
                    self.prompt_input.delete_word_at_cursor();
                    self.refresh_prompt_input();
                }
                false
            }
            "newline" => {
                // Shift+Enter: insert a literal newline into the prompt.
                if !self.is_streaming {
                    self.prompt_input.insert_newline();
                    self.refresh_prompt_input();
                }
                false
            }
            "indent" => {
                // Tab: cycle agent mode when prompt is empty, accept
                // slash-command suggestion otherwise.
                if !self.is_streaming {
                    if !self.prompt_input.suggestions.is_empty() {
                        if self.prompt_input.suggestion_index.is_none() {
                            self.prompt_input.suggestion_index = Some(0);
                        }
                        self.prompt_input.accept_suggestion();
                        self.refresh_prompt_input();
                    } else if self.prompt_input.is_empty() {
                        self.cycle_agent_mode();
                    self.rustle_look_down();
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Returns `true` if the given bash `command` is covered by the session-local
    /// prefix allowlist (i.e. its first word matches an entry in
    /// `bash_prefix_allowlist`).  Used by callers to skip the permission dialog.
    pub fn bash_command_allowed_by_prefix(&self, command: &str) -> bool {
        let first_word = command.split_whitespace().next().unwrap_or("");
        !first_word.is_empty() && self.bash_prefix_allowlist.contains(first_word)
    }

    pub(super) fn prompt_can_accept_selection_paste(&self) -> bool {
        !self.is_streaming
            && self.permission_request.is_none()
            && !matches!(
                self.prompt_input.vim_mode,
                crate::prompt_input::VimMode::Normal
                    | crate::prompt_input::VimMode::Visual
                    | crate::prompt_input::VimMode::VisualBlock
            )
    }

    pub(super) fn paste_primary_into_prompt(&mut self) -> bool {
        if !self.prompt_can_accept_selection_paste() {
            return false;
        }

        if let Some(text) = crate::image_paste::read_primary_text()
            .or_else(crate::image_paste::read_clipboard_text)
        {
            // A visible text-input dialog captures the paste instead of the
            // main prompt input.
            if self.handle_dialog_paste(&text) {
                self.refresh_prompt_input();
                return true;
            }
            self.focus = FocusTarget::Input;
            self.clear_selection();
            self.prompt_input.paste(&text);
            self.refresh_prompt_input();
            return true;
        }

        false
    }

    /// Handle a paste data string (from `Event::Paste` or Ctrl+V text fallback).
    ///
    /// If the pasted text resolves to an existing filesystem path:
    ///   - image files (png/jpg/gif/webp/bmp) → added as an image attachment pill
    ///   - other files → inserted as `@path` mention text
    ///
    /// Otherwise the text goes through the normal `prompt_input.paste()` path
    /// which applies the multi-line summary placeholder for large pastes.
    pub fn handle_paste_data(&mut self, data: String) {
        use crate::prompt_input::detect_pasted_path;
        use crate::image_paste::PastedImage;

        if let Some(path) = detect_pasted_path(&data) {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase());
            let is_image = matches!(
                ext.as_deref(),
                Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp") | Some("bmp")
            );
            if is_image {
                let label = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("image")
                    .to_string();
                let img = PastedImage { path, label, dimensions: None };
                self.prompt_input.add_image(img);
            } else {
                // Non-image file: insert as an @mention so the path is visible
                // but clearly marked as a file reference.
                let mention = format!("@{}", path.display());
                self.prompt_input.paste(&mention);
            }
        } else {
            self.prompt_input.paste(&data);
        }
    }

    /// Route a paste into a visible text-input dialog (Connect Custom URL/API
    /// key, API-key dialog, free-mode keys, ask-user custom answer, MCP
    /// elicitation fields).  Returns `true` when a dialog consumed the paste;
    /// `false` when no text dialog is open — callers MUST then swallow the
    /// paste if any modal is visible (it must never reach the main prompt).
    pub fn handle_dialog_paste(&mut self, data: &str) -> bool {
        if self.custom_provider_dialog.is_visible() {
            for ch in data.chars() {
                self.custom_provider_dialog.insert_char(ch);
            }
            return true;
        }
        if self.key_input_dialog.is_visible() {
            for ch in data.chars() {
                self.key_input_dialog.insert_char(ch);
            }
            return true;
        }
        if self.free_mode_dialog.is_visible() {
            for ch in data.chars() {
                self.free_mode_dialog.insert_char(ch);
            }
            return true;
        }
        if self.ask_user_dialog.is_visible() && self.ask_user_dialog.in_custom_input {
            for ch in data.chars() {
                self.ask_user_dialog.push_char(ch);
            }
            return true;
        }
        if self.elicitation.is_visible() {
            for ch in data.chars() {
                self.elicitation.insert_char(ch);
            }
            return true;
        }
        false
    }

    /// Gate for paste-burst detection in the live CLI event loop: keystrokes
    /// are currently flowing into the prompt (no modal is capturing input and
    /// vim is in insert mode). Unlike `prompt_is_accepting_text`, streaming
    /// does NOT disable it — the prompt stays editable during a turn for
    /// queued composition, and a raw-key paste flood must be captured there
    /// too instead of submitting on every pasted newline.
    pub fn paste_burst_allowed(&self) -> bool {
        !self.any_modal_open()
            && self.prompt_input.vim_mode == crate::prompt_input::VimMode::Insert
    }

    /// Drain any immediately-available key events from the crossterm event
    /// queue (zero-timeout poll) and return them alongside `first` as a single
    /// pasted string if the burst is large enough to be a paste.
    ///
    /// On Windows Terminal, Ctrl+V causes the terminal emulator to write the
    /// clipboard content directly to stdin as raw character events — every
    /// newline becomes an Enter keypress. Because a paste dumps ALL characters
    /// into the queue at
    /// once, a zero-timeout drain immediately after the first character
    /// reliably yields 3+ chars for any non-trivial paste, while normal
    /// keyboard typing (even at 120 WPM) almost never queues more than one
    /// char in the same 50 ms window.
    ///
    /// Returns `Some(text)` when a paste burst is detected (caller should
    /// route through `handle_paste_data`).  Returns `None` for a normal
    /// single keystroke.  If a non-character key is encountered while
    /// draining, it is stored in `self.pending_key` and will be replayed at
    /// the top of the next event-loop iteration.
    pub fn try_detect_paste_burst(
        &mut self,
        first: char,
    ) -> Option<String> {
        use crossterm::event::{Event, KeyCode, KeyEventKind};

        // Minimum number of chars (including `first`) to classify as a paste.
        // Two or more is enough: at 120 WPM the inter-key interval is ~60 ms,
        // so a second char in the same zero-timeout drain is extremely unlikely
        // from a human typist but guaranteed from a clipboard paste.
        const BURST_THRESHOLD: usize = 2;

        // Quick exit: don't bother if nothing is queued immediately.
        if !crossterm::event::poll(std::time::Duration::ZERO).unwrap_or(false) {
            return None;
        }

        let mut buf = String::new();
        buf.push(first);

        while let Ok(true) = crossterm::event::poll(std::time::Duration::ZERO) {
            match crossterm::event::read() {
                Ok(Event::Key(k)) => {
                    // Windows emits Press+Release pairs for every keystroke,
                    // so Release events are interleaved with the flood — skip
                    // them instead of treating them as end-of-burst (which
                    // capped every burst at a single character).
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    match k.code {
                        // A raw LF (0x0A) in the flood arrives as Ctrl+J —
                        // map it back to a newline or Unix pastes lose their
                        // line breaks (they'd insert a literal 'j').
                        KeyCode::Char('j')
                            if k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) =>
                        {
                            buf.push('\n')
                        }
                        KeyCode::Char(c) => buf.push(c),
                        // A raw CR (0x0D) arrives as Enter. Push '\r', not
                        // '\n': normalize_newlines() collapses CRLF pairs and
                        // lone CRs later, so CRLF pastes (Windows) don't end
                        // up with doubled line breaks.
                        KeyCode::Enter => buf.push('\r'),
                        // Raw tabs are indentation in pasted code; ending the
                        // burst on them would truncate the paste and replay
                        // Tab as a completion keypress.
                        KeyCode::Tab => buf.push('\t'),
                        _ => {
                            // Non-character key — save it for replay.
                            self.pending_key = Some(k);
                            break;
                        }
                    }
                }
                // Non-key event (mouse, resize, …) — leave in queue by
                // not reading it; we already checked poll() so it will
                // be re-read next iteration. But we already read it, so
                // we just break (the event is consumed but benign).
                _ => break,
            }
        }

        if buf.chars().count() >= BURST_THRESHOLD {
            Some(buf)
        } else {
            None
        }
    }

}
