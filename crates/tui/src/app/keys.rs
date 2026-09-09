//! Keyboard input: normalization and key-event dispatch.

use claurst_core::keybindings::{KeyContext, KeybindingResult, ParsedKeystroke};
use crate::agents_view::AgentsRoute;
use crate::dialogs::dialog::DialogBehavior as _;
use crate::notifications::NotificationKind;
use crate::input::normalize_char_with_shift;
use crate::overlays::HistorySearchOverlay;
use crate::prompt_input::VimMode;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use super::App;
use super::types::{FocusTarget, HistorySearch};

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
    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        // Make Ctrl shortcuts layout-independent before any handler runs: on
        // non-Latin layouts (Ukrainian / Russian, …) a Ctrl combo reports the
        // Cyrillic glyph at the physical key, which would otherwise miss the
        // literal `KeyCode::Char(..)` arms below — including Ctrl+C / Ctrl+D,
        // which are matched here rather than via the keybinding table (issue #47).
        let key = normalize_layout_shortcut_key(key);

        // ---- DialogBehavior dialogs capture EVERY key -----------------------
        // While one of these dialogs is visible it owns ALL keyboard input,
        // unconditionally: no global shortcuts, no keybinding resolver, no
        // paste handling, no Esc-dismiss-anything runs first. Every raw key
        // is routed to the focused dialog; whatever the dialog ignores is
        // swallowed by its modal capture (Ctrl+V included — list pickers
        // don't accept pastes, so nothing can leak into the prompt).
        if self.connect_dialog.is_visible()
            || self.import_config_picker.is_visible()
            || self.command_palette.is_visible()
        {
            if self.connect_dialog.is_visible() {
                let out = self.connect_dialog.handle_key(key);
                if out.is_confirmed() {
                    if let Some(selected) = self.connect_dialog.take_selected() {
                        self.activate_provider_from_picker(selected);
                    }
                }
            } else if self.import_config_picker.is_visible() {
                let out = self.import_config_picker.handle_key(key);
                if out.is_confirmed() {
                    if let Some(selected) = self.import_config_picker.take_selected() {
                        if let Some(selection) = Self::import_selection_from_picker(&selected.id) {
                            self.open_import_config_preview(selection);
                        }
                    }
                }
            } else {
                let out = self.command_palette.handle_key(key);
                if out.is_confirmed() {
                    if let Some(selected) = self.command_palette.take_selected() {
                        // Put the command in the input and signal for execution
                        self.prompt_input.replace_text(selected.id.clone());
                        return true; // signal to submit this as input
                    }
                }
            }
            return false;
        }

        // Dismiss error modal with Esc
        if key.code == KeyCode::Esc && self.notifications.current_is_error() {
            self.dismiss_error_notifications();
            return false;
        }


        if self.global_search.visible {
            return self.handle_global_search_key(key);
        }

        // ---- Context menu handling (highest priority for menu navigation) ----
        if self.context_menu_state.is_some() {
            match key.code {
                KeyCode::Esc => {
                    self.dismiss_context_menu();
                    return false;
                }
                KeyCode::Up | KeyCode::Down => {
                    self.navigate_context_menu(key.code);
                    return false;
                }
                KeyCode::Enter => {
                    self.execute_context_menu_item();
                    return false;
                }
                _ => {}
            }
        }

        // Bypass-permissions dialog: highest-priority gate — user must accept or the
        // session exits immediately. Mirrors TS BypassPermissionsModeDialog.tsx.
        // Accepting is remembered in settings.json (skipDangerousModePermissionPrompt)
        // so the warning is shown once, not on every launch.
        if self.bypass_permissions_dialog.is_visible() {
            let out = self.bypass_permissions_dialog.handle_key(key);
            if out.is_confirmed() {
                // "Yes, I accept" — dismiss and continue.
                let _ = Self::persist_bypass_permissions_accepted();
            } else if out.is_cancelled() {
                // "No, exit" — quit immediately.
                self.should_exit = true;
            }
            return false;
        }

        // File injection dialog: shown when oversized files are detected in @refs.
        if self.file_injection_dialog.is_visible() {
            let out = self.file_injection_dialog.handle_key(key);
            if out.is_cancelled() {
                // Abort path (Esc, or Enter on directory-only): restore the
                // stashed input to the prompt so the user can edit it.
                if let Some(input) = self.file_injection_dialog.pending_input.clone() {
                    self.set_prompt_text(input);
                }
            }
            return false;
        }

        // Onboarding dialog: shown on first launch, dismissed with Enter/→/Esc.
        if self.onboarding_dialog.is_visible() {
            let out = self.onboarding_dialog.handle_key(key);
            if out.is_confirmed() {
                // Reached the final page — persist that onboarding is complete
                // (best-effort).
                let _ = Self::persist_onboarding_complete();
            }
            return false;
        }

        // Effort picker dialog (/effort). The selector is horizontal
        // (Faster ← → Smarter), so ←/→ (and vi h/l) move the selection.
        if self.effort_picker.visible {
            match key.code {
                KeyCode::Esc => self.effort_picker.close(),
                KeyCode::Left | KeyCode::Char('h') => self.effort_picker.select_prev(),
                KeyCode::Right | KeyCode::Char('l') => self.effort_picker.select_next(),
                KeyCode::Enter => {
                    // Applying `Ultracode` here is equivalent to typing the
                    // `ultracode` keyword: it sets the effort to the top level.
                    let chosen = self.effort_picker.current();
                    self.effort_level = chosen;
                    self.effort_picker.close();
                    self.status_message = Some(format!(
                        "Effort set to {} {}.",
                        chosen.symbol(),
                        chosen.label()
                    ));
                }
                _ => {}
            }
            return false;
        }

        // Device code / browser auth dialog (GitHub Copilot, Anthropic OAuth).
        // DialogCore-based: the dialog swallows every key while waiting; any
        // key after Success closes it as `Confirmed` (store the credential),
        // any key after Error / Esc closes it as `Cancelled`.
        if self.device_auth_dialog.is_visible() {
            let out = self.device_auth_dialog.handle_key(key);
            if out.is_close() {
                if out.is_confirmed() {
                    if let crate::dialogs::device_auth_dialog::DeviceAuthStatus::Success(ref token) =
                        self.device_auth_dialog.status
                    {
                        let provider_id = self.device_auth_dialog.provider_id.clone();
                        let provider_name = self.device_auth_dialog.provider_name.clone();
                        let token = token.clone();
                        if provider_id == "anthropic-oauth" {
                            // The claude.ai OAuth flow already persisted the Bearer
                            // tokens via save_and_register; the anthropic provider
                            // reads them directly. Switch to the real "anthropic"
                            // provider without re-storing the token as an API key.
                            self.device_auth_pending = None;
                            self.device_auth_dialog.close();
                            self.activate_provider(
                                "anthropic".to_string(),
                                "Anthropic".to_string(),
                                "Connected to",
                            );
                            // The live client was built at startup with no
                            // credential; ask the main loop to re-resolve the
                            // freshly-saved Bearer and swap in a working client.
                            self.pending_provider_reload = true;
                            return false;
                        }
                        let credential = if provider_id == "github-copilot" {
                            claurst_core::StoredCredential::OAuthToken {
                                access: token.clone(),
                                refresh: token,
                                expires: 0,
                            }
                        } else {
                            claurst_core::StoredCredential::ApiKey { key: token }
                        };
                        self.auth_store.set(
                            &provider_id,
                            credential,
                        );
                        self.activate_provider(provider_id, provider_name, "Connected to");
                    }
                }
                self.device_auth_pending = None;
            }
            return false;
        }

        // Ask-user question dialog (AskUserQuestion tool)
        if self.ask_user_dialog.is_visible() {
            let out = self.ask_user_dialog.handle_key(key);
            if out.is_cancelled() {
                // Esc — send an empty reply so the tool result signals
                // "user dismissed".
                self.ask_user_dialog.dismiss();
            }
            return false;
        }

        if self.key_input_dialog.is_visible() {
            // Ctrl/Super+V paste stays app-level — it needs the notification
            // system (terminals that don't emit Event::Paste).
            if key.code == KeyCode::Char('v')
                && (key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::SUPER))
            {
                if let Some(text) = crate::image_paste::read_clipboard_text() {
                    if text.is_empty() {
                        self.push_notification(NotificationKind::Warning, "Clipboard is empty".to_string(), Some(2));
                    } else {
                        for ch in text.chars() {
                            self.key_input_dialog.insert_char(ch);
                        }
                    }
                } else {
                    self.push_notification(NotificationKind::Warning, "Could not read clipboard".to_string(), Some(2));
                }
                return false;
            }
            let out = self.key_input_dialog.handle_key(key);
            if out.is_confirmed() {
                let provider_id = self.key_input_dialog.provider_id.clone();
                let provider_name = self.key_input_dialog.provider_name.clone();
                let api_key = self.key_input_dialog.take_key();
                if !api_key.is_empty() {
                    self.auth_store.set(
                        &provider_id,
                        claurst_core::StoredCredential::ApiKey { key: api_key },
                    );
                    self.activate_provider(provider_id, provider_name, "Connected to");
                }
            }
            return false;
        }

        // "Free" composite-provider setup dialog (collects any subset of the
        // free-tier upstream keys; min 1 to enable, more = better).
        // DialogCore-based: navigation / editing / Enter live in the dialog's
        // `on_key`; Ctrl/Super+V paste stays app-level because it needs the
        // notification system.
        if self.free_mode_dialog.is_visible() {
            if key.code == KeyCode::Char('v')
                && (key.modifiers.contains(KeyModifiers::CONTROL)
                    || key.modifiers.contains(KeyModifiers::SUPER))
            {
                // Paste clipboard text into the focused field (terminals
                // that don't emit Event::Paste, e.g. Windows Terminal).
                if let Some(text) = crate::image_paste::read_clipboard_text() {
                    if text.is_empty() {
                        self.push_notification(NotificationKind::Warning, "Clipboard is empty".to_string(), Some(2));
                    } else {
                        for ch in text.chars() {
                            self.free_mode_dialog.insert_char(ch);
                        }
                    }
                } else {
                    self.push_notification(NotificationKind::Warning, "Could not read clipboard".to_string(), Some(2));
                }
                return false;
            }
            let out = self.free_mode_dialog.handle_key(key);
            if out.is_confirmed() {
                let values = self.free_mode_dialog.take_values();
                for (provider_id, key) in values {
                    self.auth_store.set(
                        provider_id,
                        claurst_core::StoredCredential::ApiKey { key },
                    );
                }
                self.activate_provider(
                    "free".to_string(),
                    "Free Mode".to_string(),
                    "Connected to",
                );
            }
            return false;
        }

        // Custom provider dialog (URL + API key for OpenAI-compatible providers)
        if self.custom_provider_dialog.is_visible() {
            // Ctrl/Super+V paste stays app-level — it needs the notification
            // system (terminals that don't emit Event::Paste).
            if key.code == KeyCode::Char('v')
                && (key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::SUPER))
            {
                if let Some(text) = crate::image_paste::read_clipboard_text() {
                    if text.is_empty() {
                        self.push_notification(NotificationKind::Warning, "Clipboard is empty".to_string(), Some(2));
                    } else {
                        for ch in text.chars() {
                            self.custom_provider_dialog.insert_char(ch);
                        }
                    }
                } else {
                    self.push_notification(NotificationKind::Warning, "Could not read clipboard".to_string(), Some(2));
                }
                return false;
            }
            let out = self.custom_provider_dialog.handle_key(key);
            if out.is_confirmed() {
                let provider_id = self.custom_provider_dialog.provider_id.clone();
                let provider_name = self.custom_provider_dialog.provider_name.clone();
                let (base_url, api_key) = self.custom_provider_dialog.take_values();
                self.persist_custom_provider_base_url(&base_url);
                self.auth_store.set(
                    &provider_id,
                    claurst_core::StoredCredential::ApiKey { key: api_key },
                );
                self.activate_provider(provider_id, provider_name, "Connected to");
            }
            return false;
        }

        // Import-config preview dialog
        if self.import_config_dialog.is_visible() {
            let out = self.import_config_dialog.handle_key(key);
            if out.is_confirmed() {
                self.perform_import_config();
            }
            return false;
        }

        // Invalid-config dialog intercepts Enter/Esc to dismiss
        if self.invalid_config_dialog.is_visible() {
            let _ = self.invalid_config_dialog.handle_key(key);
            return false;
        }

        // Model picker — routed through the generic DialogBehavior pipeline
        // (crate::dialogs::dialog): navigation, effort ←/→, filter typing and Esc are
        // handled by the dialog itself; Enter returns Confirmed and the
        // confirmed model is consumed here.
        if self.model_picker.is_visible() {
            let out = self.model_picker.handle_key(key);
            if out.is_confirmed() {
                if let Some((model_id, effort)) = self.model_picker.confirm() {
                    // If user picked a model other than the fast-mode model
                    // while fast mode was active, turn fast mode off.
                    if self.fast_mode && !self.model_picker.is_selected_fast_mode_model(&model_id) {
                        self.fast_mode = false;
                    }
                    if let Some(e) = effort {
                        self.effort_level = e;
                    }
                    // Store explicit selections in the canonical
                    // "provider/model" form for non-Anthropic providers.
                    // The "free" composite's picker entries already carry
                    // a routing prefix (`free/…`, `zen/…`, `openrouter/…`)
                    // so re-prefixing would produce nonsense like
                    // `free/free/auto`.
                    let provider = self.config.provider.as_deref().unwrap_or("anthropic");
                    let full_model = if provider == "anthropic" || provider == "free" {
                        model_id.clone()
                    } else {
                        format!("{}/{}", provider, model_id)
                    };
                    self.set_model(full_model.clone());
                    self.persist_provider_and_model();
                    let effort_hint = effort.map(|e| format!(" [{}]", e.label())).unwrap_or_default();
                    self.status_message = Some(format!("Model: {}{}", full_model, effort_hint));
                }
            }
            return false;
        }

        // Session branching overlay intercepts navigation and Esc
        if self.session_branching.is_visible() {
            let out = self.session_branching.handle_key(key);
            if out.is_confirmed() {
                use crate::dialogs::session_branching::BranchBrowserMode;
                match self.session_branching.mode {
                    BranchBrowserMode::Browse => {
                        if let Some(branch) = self.session_branching.selected_branch() {
                            self.status_message = Some(format!("Switched to branch: {}", branch.name));
                        }
                        self.session_branching.close();
                    }
                    BranchBrowserMode::CreateNew => {
                        if let Some((name, at_msg)) = self.session_branching.confirm_create_new() {
                            self.status_message = Some(format!("Created branch: {} at message {}", name, at_msg));
                            self.session_branching.close();
                        }
                    }
                    BranchBrowserMode::ConfirmDelete => {
                        if let Some(branch_id) = self.session_branching.confirm_delete() {
                            self.status_message = Some(format!("Deleted branch: {}", branch_id));
                        }
                    }
                }
            }
            return false;
        }

        // Session browser intercepts navigation and Esc
        if self.session_browser.is_visible() {
            let out = self.session_browser.handle_key(key);
            if out.is_confirmed() {
                use crate::dialogs::session_browser::SessionBrowserMode;
                match self.session_browser.mode {
                    SessionBrowserMode::Rename => {
                        if let Some((_id, name)) = self.session_browser.confirm_rename() {
                            self.session_title = Some(name.clone());
                            self.status_message = Some(format!("Renamed to: {}", name));
                        }
                        self.session_browser.close();
                    }
                    SessionBrowserMode::Confirm => {
                        self.session_browser.close();
                    }
                    _ => {}
                }
            }
            return false;
        }

        // Tasks overlay intercepts navigation and Esc
        if self.tasks_overlay.visible {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.tasks_overlay.close(),
                KeyCode::Up => self.tasks_overlay.select_prev(),
                KeyCode::Down => self.tasks_overlay.select_next(),
                KeyCode::Enter => {
                    if let Some((task_id, new_status)) = self.tasks_overlay.cycle_and_persist_status() {
                        self.status_message = Some(format!("Task {} → {}", task_id, new_status));
                    }
                }
                _ => {}
            }
            return false;
        }

        // Export dialog key handling
        if self.export_dialog.is_visible() {
            let out = self.export_dialog.handle_key(key);
            if out.is_confirmed() {
                if let Some(path) = self.perform_export() {
                    self.push_notification(
                        NotificationKind::Info,
                        format!("Exported to {}", path),
                        Some(4),
                    );
                } else {
                    self.push_notification(
                        NotificationKind::Warning,
                        "Export failed: could not write file.".to_string(),
                        Some(4),
                    );
                }
            }
            return false;
        }

        // Context visualization overlay key handling
        if self.context_viz.visible {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.context_viz.close();
                }
                _ => {}
            }
            return false;
        }

        // MCP approval dialog
        if self.mcp_approval.visible {
            if let Some(choice) = crate::dialogs::handle_mcp_approval_key(&mut self.mcp_approval, key) {
                self.handle_mcp_approval_decision(choice);
            }
            return false;
        }

        // Feedback survey intercepts digit keys and Esc
        if self.feedback_survey.is_visible() {
            let _ = self.feedback_survey.handle_key(key);
            return false;
        }

        // Memory file selector intercepts navigation and Esc
        if self.memory_file_selector.is_visible() {
            let _ = self.memory_file_selector.handle_key(key);
            return false;
        }

        // Hooks config menu intercepts navigation and Esc
        if self.hooks_config_menu.visible {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.hooks_config_menu.back(),
                KeyCode::Enter => self.hooks_config_menu.enter(),
                KeyCode::Up | KeyCode::Char('k') => self.hooks_config_menu.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.hooks_config_menu.select_next(),
                _ => {}
            }
            return false;
        }

        if self.paste_viewer.visible {
            self.handle_paste_viewer_key(key);
            return false;
        }

        if self.diff_viewer.is_visible() {
            // 'd' toggles the diff scope and needs the project root (on App).
            if key.code == KeyCode::Char('d') && key.modifiers.is_empty() {
                let root = self.project_root();
                self.diff_viewer.toggle_diff_type(&root);
                return false;
            }
            let _ = self.diff_viewer.handle_key(key);
            return false;
        }

        if self.agents_menu.visible {
            self.handle_agents_menu_key(key);
            return false;
        }

        if self.mcp_view.visible {
            return self.handle_mcp_view_key(key);
        }

        if self.stats_dialog.is_visible() {
            let _ = self.stats_dialog.handle_key(key);
            return false;
        }

        // Settings screen intercepts keys
        if self.settings_screen.visible {
            crate::settings_screen::handle_settings_key(
                &mut self.settings_screen,
                &mut self.config,
                key,
            );
            return false;
        }

        // Theme picker intercepts keys
        if self.theme_screen.visible {
            if let Some(theme_name) =
                crate::theme_screen::handle_theme_key(&mut self.theme_screen, key)
            {
                self.apply_theme(&theme_name);
            }
            return false;
        }

        // Privacy screen intercepts keys
        // Rewind flow overlay intercepts keys first
        if self.rewind_flow.visible {
            return self.handle_rewind_flow_key(key);
        }

        // Help overlay intercepts keys next
        if self.help_overlay.visible {
            return self.handle_help_overlay_key(key);
        }

        // New history-search overlay
        if self.history_search_overlay.visible {
            return self.handle_history_search_overlay_key(key);
        }

        if self.global_search.visible {
            return self.handle_global_search_key(key);
        }

        // Legacy history-search mode intercepts most keys
        if self.history_search.is_some() {
            return self.handle_history_search_key(key);
        }

        // Permission dialog mode intercepts most keys
        if self.permission_request.is_some() {
            self.handle_permission_key(key);
            return false;
        }

        // Notification dismiss
        if key.code == KeyCode::Esc && !self.notifications.is_empty() {
            self.notifications.dismiss_current();
            return false;
        }

        // Plugin hint dismiss
        if key.code == KeyCode::Esc {
            if let Some(hint) = self.plugin_hints.iter_mut().find(|h| h.is_visible()) {
                hint.dismiss();
                return false;
            }
        }

        // Overage upsell dismiss
        if key.code == KeyCode::Esc && self.overage_upsell.visible {
            self.overage_upsell.dismiss();
            return false;
        }


        // Desktop upsell startup dialog
        if self.desktop_upsell.is_visible() {
            let _ = self.desktop_upsell.handle_key(key);
            return false;
        }

        // Memory update notification dismiss
        if key.code == KeyCode::Esc && self.memory_update_notification.visible {
            self.memory_update_notification.dismiss();
            return false;
        }

        // MCP elicitation dialog — highest priority modal
        if self.elicitation.is_visible() {
            let out = self.elicitation.handle_key(key);
            if out.is_cancelled() {
                // Esc — queue a Cancelled result so the caller can take_result().
                self.elicitation.cancel();
            }
            return false;
        }

        // ---- Keybinding processor (runs AFTER all dialog checks) ----------
        let key_context = self.current_key_context();
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
                let label = img.label.clone();
                let dims = img.dimensions;
                self.prompt_input.add_image(img);
                let msg = if let Some((w, h)) = dims {
                    format!("Image attached: {} ({}x{})", label, w, h)
                } else {
                    format!("Image attached: {}", label)
                };
                self.push_notification(NotificationKind::Info, msg, Some(3));
            } else if let Some(text) = read_clipboard_text().or_else(read_primary_text) {
                self.handle_paste_data(text);
                self.refresh_prompt_input();
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
                    let copied = crate::image_paste::write_clipboard_text(&sel_text);
                    self.selection_anchor = None;
                    self.selection_focus = None;
                    *self.selection_text.borrow_mut() = String::new();
                    if copied {
                        self.push_notification(NotificationKind::Info, "Copied to clipboard".to_string(), Some(2));
                    }
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

            // ---- History search ----------------------------------------
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Open the new overlay-based history search
                let overlay = HistorySearchOverlay::open(&self.prompt_input.history);
                self.history_search_overlay = overlay;
                // Also open legacy for backwards compat
                let mut hs = HistorySearch::new();
                hs.update_matches(&self.prompt_input.history);
                self.history_search = Some(hs);
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.global_search.open();
                self.refresh_global_search();
            }

            // ---- Tasks overlay (Ctrl+T) --------------------------------
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.tasks_overlay.toggle();
            }

            // ---- Help overlay ------------------------------------------
            KeyCode::F(1) => {
                self.show_help = !self.show_help;
                self.help_overlay.toggle();
            }
            KeyCode::Char('?')
                if !self.is_streaming
                    && self.prompt_input.is_empty()
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.show_help = !self.show_help;
                self.help_overlay.toggle();
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
                self.show_help = !self.show_help;
                self.help_overlay.toggle();
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
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.move_word_forward();
                self.sync_legacy_prompt_fields();
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
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Right => {
                if key.modifiers.contains(KeyModifiers::SUPER) {
                    self.prompt_input.cursor = self.prompt_input.text.len();
                } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.prompt_input.move_word_forward();
                } else {
                    self.prompt_input.move_right();
                }
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Home => {
                self.prompt_input.cursor = 0;
                self.sync_legacy_prompt_fields();
            }
            KeyCode::End => {
                self.prompt_input.cursor = self.prompt_input.text.len();
                self.sync_legacy_prompt_fields();
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
                // Auto-dismiss all error notifications when user sends a message
                self.dismiss_error_notifications();
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
            self.exit_key_sequence_start = None;
        }

        false
    }

    pub(super) fn current_key_context(&self) -> KeyContext {
        if self.diff_viewer.is_visible() {
            KeyContext::DiffDialog
        } else if self.agents_menu.visible || self.mcp_view.visible || self.stats_dialog.is_visible() {
            KeyContext::Select
        } else if self.import_config_dialog.is_visible() {
            KeyContext::Confirmation
        } else if self.settings_screen.visible {
            KeyContext::Settings
        } else if self.theme_screen.visible {
            KeyContext::ThemePicker
        } else if self.rewind_flow.visible {
            KeyContext::Confirmation
        } else if self.help_overlay.visible {
            KeyContext::Help
        } else if self.history_search_overlay.visible || self.history_search.is_some() {
            KeyContext::HistorySearch
        } else if self.permission_request.is_some() {
            KeyContext::Confirmation
        } else if self.show_help {
            KeyContext::Help
        } else {
            KeyContext::Chat
        }
    }

    pub(super) fn handle_mcp_view_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.mcp_view.close(),
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => self.mcp_view.switch_pane(),
            KeyCode::Up => self.mcp_view.select_prev(),
            KeyCode::Down => self.mcp_view.select_next(),
            KeyCode::Backspace => self.mcp_view.pop_search_char(),
            KeyCode::Char('e') => self.mcp_view.toggle_error_detail(),
            KeyCode::Char('a')
                if self.mcp_view.active_pane == crate::mcp_view::McpViewPane::ServerList =>
            {
                let selected_server = self
                    .mcp_view
                    .servers
                    .get(self.mcp_view.selected_server)
                    .map(|server| server.name.clone());
                if let Some(server_name) = selected_server {
                    self.pending_mcp_panel_auth = Some(server_name);
                    self.mcp_view.close();
                    self.status_message = Some("Starting MCP auth...".to_string());
                }
            }
            KeyCode::Char('r') => {
                self.pending_mcp_reconnect = true;
                self.status_message = Some("Reconnecting MCP runtime...".to_string());
            }
            KeyCode::Char(c) if key.modifiers.is_empty()
                && self.mcp_view.active_pane != crate::mcp_view::McpViewPane::ServerList => {
                    self.mcp_view.push_search_char(c);
                }
            _ => {}
        }
        false
    }

    pub(super) fn handle_agents_menu_key(&mut self, key: KeyEvent) {
        if matches!(self.agents_menu.route, AgentsRoute::Editor(_)) {
            match key.code {
                KeyCode::Esc => self.agents_menu.go_back(),
                KeyCode::Tab | KeyCode::Down => self.agents_menu.editor_next_field(),
                KeyCode::BackTab | KeyCode::Up => self.agents_menu.editor_prev_field(),
                KeyCode::Enter => self.agents_menu.editor_insert_newline(),
                KeyCode::Backspace => self.agents_menu.editor_backspace(),
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    match self.agents_menu.save_editor() {
                        Ok(msg) => self.status_message = Some(msg),
                        Err(err) => {
                            self.agents_menu.editor.error = Some(err.clone());
                            self.agents_menu.editor.saved_message = None;
                            self.status_message = Some(err);
                        }
                    }
                }
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let ch = self.shift_normalize(ch, key.modifiers);
                    self.agents_menu.editor_insert_char(ch);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => self.agents_menu.go_back(),
            KeyCode::Up => self.agents_menu.select_prev(),
            KeyCode::Down => self.agents_menu.select_next(),
            KeyCode::Enter | KeyCode::Right => self.agents_menu.confirm_selection(),
            KeyCode::Left => self.agents_menu.go_back(),
            _ => {}
        }
    }

    pub(super) fn handle_help_overlay_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::F(1) => {
                self.help_overlay.close();
                self.show_help = false;
            }
            KeyCode::Char('?')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.help_overlay.close();
                self.show_help = false;
            }
            KeyCode::Up => {
                self.help_overlay.scroll_up();
            }
            KeyCode::Down => {
                let max = 50u16; // generous upper bound; renderer will clamp
                self.help_overlay.scroll_down(max);
            }
            KeyCode::Backspace => {
                self.help_overlay.pop_filter_char();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.help_overlay.push_filter_char(c);
            }
            _ => {}
        }
        false
    }

    pub(super) fn handle_history_search_overlay_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.history_search_overlay.close();
                self.history_search = None;
            }
            KeyCode::Enter => {
                if let Some(entry) = self
                    .history_search_overlay
                    .current_entry(&self.prompt_input.history)
                {
                    self.set_prompt_text(entry.to_string());
                }
                self.history_search_overlay.close();
                self.history_search = None;
            }
            KeyCode::Up => {
                self.history_search_overlay.select_prev();
                if let Some(hs) = self.history_search.as_mut() {
                    let count = hs.matches.len();
                    if count > 0 {
                        if hs.selected == 0 {
                            hs.selected = count - 1;
                        } else {
                            hs.selected -= 1;
                        }
                    }
                }
            }
            KeyCode::Down => {
                self.history_search_overlay.select_next();
                if let Some(hs) = self.history_search.as_mut() {
                    let count = hs.matches.len();
                    if count > 0 {
                        hs.selected = (hs.selected + 1) % count;
                    }
                }
            }
            KeyCode::Backspace => {
                let history = self.prompt_input.history.clone();
                self.history_search_overlay.pop_char(&history);
                if let Some(hs) = self.history_search.as_mut() {
                    hs.query.pop();
                    hs.update_matches(&history);
                }
            }
            // 'p' with no modifiers and an empty query = pin/unpin the selected entry.
            // When the query is non-empty 'p' is treated as a filter character so
            // the user can still search for prompts containing the letter 'p'.
            KeyCode::Char('p')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.history_search_overlay.query.is_empty() =>
            {
                self.history_search_overlay.toggle_pin();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let c = self.shift_normalize(c, key.modifiers);
                let history = self.prompt_input.history.clone();
                self.history_search_overlay.push_char(c, &history);
                if let Some(hs) = self.history_search.as_mut() {
                    hs.query.push(c);
                    hs.update_matches(&history);
                }
            }
            _ => {}
        }
        false
    }

    pub(super) fn handle_rewind_flow_key(&mut self, key: KeyEvent) -> bool {
        use crate::overlays::RewindStep;
        match &self.rewind_flow.step {
            RewindStep::Selecting => match key.code {
                KeyCode::Esc => {
                    self.rewind_flow.close();
                }
                KeyCode::Enter => {
                    self.rewind_flow.confirm_selection();
                }
                KeyCode::Up => {
                    self.rewind_flow.selector.select_prev();
                }
                KeyCode::Down => {
                    self.rewind_flow.selector.select_next();
                }
                _ => {}
            },
            RewindStep::Confirming { .. } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(idx) = self.rewind_flow.accept_confirm() {
                        // Truncate conversation to the selected message index.
                        self.messages.truncate(idx);
                        // Remove system annotations placed after the truncation point.
                        self.system_annotations.retain(|a| a.after_index <= idx);
                        self.push_notification(
                            NotificationKind::Success,
                            format!("Rewound to message #{}", idx),
                            Some(4),
                        );
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.rewind_flow.reject_confirm();
                }
                _ => {}
            },
        }
        false
    }

    pub(super) fn handle_global_search_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.global_search.close();
            }
            KeyCode::Enter => {
                if let Some(selected) = self.global_search.selected_ref() {
                    self.set_prompt_text(selected);
                }
                self.global_search.close();
            }
            KeyCode::Up => self.global_search.select_prev(),
            KeyCode::Down => self.global_search.select_next(),
            KeyCode::Backspace => {
                self.global_search.pop_char();
                self.refresh_global_search();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let c = self.shift_normalize(c, key.modifiers);
                self.global_search.push_char(c);
                self.refresh_global_search();
            }
            _ => {}
        }
        false
    }

    pub(super) fn handle_exit_key_confirmation(&mut self, mut key_char: char) {
        fn exit_message(key: char) -> &'static str {
            if key == 'c' {
                "Press Ctrl+C again to exit"
            } else {
                "Press Ctrl+D again to exit"
            }
        }

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
        self.push_notification(NotificationKind::Info, exit_message(key_char).to_string(), Some(2));
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
                        // First press or timeout expired: show exit confirmation.
                        self.push_notification(NotificationKind::Info, "Press Ctrl+C again to exit".to_string(), Some(2));
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
                let overlay = HistorySearchOverlay::open(&self.prompt_input.history);
                self.history_search_overlay = overlay;
                let mut hs = HistorySearch::new();
                hs.update_matches(&self.prompt_input.history);
                self.history_search = Some(hs);
                false
            }
            "openSearch" => {
                self.global_search.open();
                self.refresh_global_search();
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
                    self.sync_legacy_prompt_fields();
                }
                false
            }
            "goLineEnd" => {
                if !self.is_streaming {
                    self.prompt_input.cursor = self.prompt_input.text.len();
                    self.sync_legacy_prompt_fields();
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
                self.show_help = false;
                self.help_overlay.close();
                false
            }
            "select" => {
                // Legacy history search select
                if let Some(hs) = self.history_search.as_ref() {
                    if let Some(entry) = hs.current_entry(&self.prompt_input.history) {
                        self.set_prompt_text(entry.to_string());
                    }
                }
                self.history_search = None;
                self.history_search_overlay.close();
                false
            }
            "cancel" => {
                self.history_search = None;
                self.history_search_overlay.close();
                false
            }
            "prevResult" => {
                if let Some(hs) = self.history_search.as_mut() {
                    let count = hs.matches.len();
                    if count > 0 {
                        if hs.selected == 0 {
                            hs.selected = count - 1;
                        } else {
                            hs.selected -= 1;
                        }
                    }
                }
                self.history_search_overlay.select_prev();
                false
            }
            "nextResult" => {
                if let Some(hs) = self.history_search.as_mut() {
                    let count = hs.matches.len();
                    if count > 0 {
                        hs.selected = (hs.selected + 1) % count;
                    }
                }
                self.history_search_overlay.select_next();
                false
            }
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
                self.show_help = !self.show_help;
                self.help_overlay.toggle();
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

    /// Handle a key event while in legacy history-search mode.
    pub(super) fn handle_history_search_key(&mut self, key: KeyEvent) -> bool {
        let hs = match self.history_search.as_mut() {
            Some(h) => h,
            None => return false,
        };
        match key.code {
            KeyCode::Esc => {
                self.history_search = None;
                self.history_search_overlay.close();
            }
            KeyCode::Enter => {
                if let Some(entry) = hs.current_entry(&self.prompt_input.history) {
                    self.set_prompt_text(entry.to_string());
                }
                self.history_search = None;
                self.history_search_overlay.close();
            }
            KeyCode::Up => {
                let count = hs.matches.len();
                if count > 0 {
                    if hs.selected == 0 {
                        hs.selected = count - 1;
                    } else {
                        hs.selected -= 1;
                    }
                }
            }
            KeyCode::Down => {
                let count = hs.matches.len();
                if count > 0 {
                    hs.selected = (hs.selected + 1) % count;
                }
            }
            KeyCode::Backspace => {
                hs.query.pop();
                let history = self.prompt_input.history.clone();
                if let Some(hs) = self.history_search.as_mut() {
                    hs.update_matches(&history);
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                hs.query.push(c);
                let history = self.prompt_input.history.clone();
                if let Some(hs) = self.history_search.as_mut() {
                    hs.update_matches(&history);
                }
            }
            _ => {}
        }
        false
    }

    /// Handle a key event while a permission dialog is active.
    pub(super) fn handle_permission_key(&mut self, key: KeyEvent) {
        let pr = match self.permission_request.as_mut() {
            Some(p) => p,
            None => return,
        };

        match key.code {
            KeyCode::Char(c) => {
                if let Some(digit) = c.to_digit(10) {
                    let idx = (digit as usize).saturating_sub(1);
                    if idx < pr.options.len() {
                        pr.selected_option = idx;
                    }
                } else {
                    // Check if any option matches this key.
                    let mut matched_idx = None;
                    for (i, opt) in pr.options.iter().enumerate() {
                        if opt.key == c {
                            matched_idx = Some(i);
                            break;
                        }
                    }
                    if let Some(idx) = matched_idx {
                        pr.selected_option = idx;
                        // If this is the prefix-allow option ('P'), record the prefix.
                        self.maybe_record_bash_prefix();
                        self.permission_request = None;
                    }
                }
            }
            KeyCode::Enter => {
                // If the currently selected option is the prefix-allow option, record it.
                self.maybe_record_bash_prefix();
                self.permission_request = None;
            }
            KeyCode::Up => {
                let pr = self.permission_request.as_mut().unwrap();
                if pr.selected_option > 0 {
                    pr.selected_option -= 1;
                }
            }
            KeyCode::Down => {
                let pr = self.permission_request.as_mut().unwrap();
                if pr.selected_option + 1 < pr.options.len() {
                    pr.selected_option += 1;
                }
            }
            KeyCode::Esc => {
                self.permission_request = None;
            }
            _ => {}
        }
    }

    /// If the active permission dialog's selected option is the prefix-allow
    /// option ('P') for a Bash dialog, extract the suggested prefix and add it
    /// to `bash_prefix_allowlist` so future requests with the same prefix are
    /// silently approved.
    pub(super) fn maybe_record_bash_prefix(&mut self) {
        use crate::dialogs::PermissionDialogKind;
        let pr = match self.permission_request.as_ref() {
            Some(p) => p,
            None => return,
        };
        // Only act on Bash dialogs where the selected option key is 'P'.
        let selected_key = pr.options.get(pr.selected_option).map(|o| o.key);
        if selected_key != Some('P') {
            return;
        }
        if let PermissionDialogKind::Bash { command, .. } = &pr.kind {
            // Always normalize to the first whitespace-delimited word so
            // that the allowlist check in `bash_command_allowed_by_prefix`
            // (which also uses `split_whitespace().next()`) matches correctly.
            let first_word = command.split_whitespace().next().unwrap_or("").to_string();
            if !first_word.is_empty() {
                self.bash_prefix_allowlist.insert(first_word.clone());
                // Persist so the "always allow" choice survives restarts.
                if let Ok(mut settings) = claurst_core::config::Settings::load_sync() {
                    if !settings.allowed_bash_prefixes.contains(&first_word) {
                        settings.allowed_bash_prefixes.push(first_word);
                        let _ = settings.save_sync();
                    }
                }
            }
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
            && !self.history_search_overlay.visible
            && self.history_search.is_none()
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
                let img = PastedImage { path, label: label.clone(), dimensions: None };
                self.prompt_input.add_image(img);
                self.push_notification(
                    crate::notifications::NotificationKind::Info,
                    format!("Image attached: {}", label),
                    Some(3),
                );
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

    /// Returns `true` when the app is in a state where the prompt can accept
    /// regular text input. NOTE: this list predates the DialogSelect pickers
    /// (connect dialog, command palette, …) and is NOT modal-complete —
    /// use `paste_burst_allowed` for paste gating instead.
    #[allow(dead_code)]
    pub(super) fn prompt_is_accepting_text(&self) -> bool {
        !self.is_streaming
            && self.permission_request.is_none()
            && !self.ask_user_dialog.is_visible()
            && !self.history_search_overlay.visible
            && self.history_search.is_none()
            && !self.settings_screen.visible
            && !self.theme_screen.visible
            && !self.custom_provider_dialog.is_visible()
            && !self.key_input_dialog.is_visible()
            && !self.free_mode_dialog.is_visible()
            && !self.elicitation.is_visible()
            && self.prompt_input.vim_mode == crate::prompt_input::VimMode::Insert
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
