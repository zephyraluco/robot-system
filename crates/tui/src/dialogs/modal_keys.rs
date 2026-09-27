//! modal_keys.rs — THE modal key layer.
//!
//! Keyboard input in the TUI has exactly **two layers**, and this module owns
//! the first one:
//!
//! 1. **Modal layer** (this module). While any modal is visible it owns the
//!    keyboard unconditionally: the key is fed to the modal through the shared
//!    protocol below, the modal's confirmation effect is applied, and the key
//!    never reaches the main UI. Modal implementations live with the modals
//!    (`dialogs/*`), never here.
//! 2. **Main UI layer** (`crate::app::keys`). Only runs when `NoModal` comes
//!    back: prompt editing, scrolling, global shortcuts.
//!
//! ## The shared protocol
//!
//! Every modal is a `DialogCore` + `DialogBehavior` dialog, so it is driven by
//! the same one call — [`DialogBehavior::handle_key`] — which already owns the
//! capture rules:
//!
//! * `Esc` → the modal's `on_escape` (or its default "close as `Cancelled`"),
//! * `Tab` / `Shift+Tab` → focus cycling,
//! * everything else → the modal's own [`DialogBehavior::on_key`],
//! * and because dialogs are modal, anything the behaviour ignores is still
//!   swallowed, so nothing leaks into the UI underneath.
//!
//! Three modals are not `DialogCore`-based (the full-screen help overlay, the
//! context menu and the permission dialogs, which expose the same `handle_key`
//! protocol through their own entry points); those are the only arms below that
//! do not go through [`drive`].
//!
//! Adding a modal therefore never touches `app/keys.rs`: add the state to
//! `App`, add one arm to [`handle_modal_key`], render it, and implement its
//! `DialogBehavior`.
//!
//! ## Layering note
//!
//! This module takes `&mut App` because a modal's confirmation effect (activate
//! a provider, switch model, write an export) is app state, not dialog state —
//! keeping it here means the modal's *own* key logic still lives in its dialog
//! module. This mirrors the existing `dialogs::settings_screen::handle_settings_key`
//! / `dialogs::theme_screen::handle_theme_key` adapters; `app/keys.rs` remains
//! free of modal knowledge.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::dialogs::dialog::{DialogBehavior, DialogOutcome};

/// What happened to a key after the modal layer ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKeyOutcome {
    /// No modal is visible — the key belongs to the main UI.
    NoModal,
    /// A modal consumed the key; the caller must stop handling it.
    Consumed,
    /// A modal asked for the current prompt to be submitted as input
    /// (the command palette, whose confirmed entry is a slash command).
    Submit,
}

/// Drive one modal through the shared key pipeline.
///
/// This is the single funnel of the modal layer: [`DialogBehavior::handle_key`]
/// applies `Esc` / `Tab` handling, offers the key to the modal's `on_key`, and
/// swallows what the modal ignores. The caller only reacts to the closing
/// outcomes (`Confirmed` / `Cancelled`).
fn drive<D: DialogBehavior>(dialog: &mut D, key: KeyEvent) -> DialogOutcome {
    dialog.handle_key(key)
}

/// Route a key press to whichever modal owns the keyboard.
///
/// Returns [`ModalKeyOutcome::NoModal`] when nothing is visible, so the caller
/// falls through to the main-UI handler. The chain below is the ONLY place
/// modal key routing lives, and each arm is the same three steps: *is it
/// visible → drive it → apply its confirmation effect*.
pub fn handle_modal_key(app: &mut App, key: KeyEvent) -> ModalKeyOutcome {
    use ModalKeyOutcome::{Consumed, Submit};

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let super_key = key.modifiers.contains(KeyModifiers::SUPER);

    // ---- Provider / command pickers (`DialogSelect`) --------------------
    if app.connect_dialog.is_visible() {
        if drive(&mut app.connect_dialog, key).is_confirmed() {
            if let Some(selected) = app.connect_dialog.take_selected() {
                app.activate_provider_from_picker(selected);
            }
        }
        return Consumed;
    }
    if app.import_config_picker.is_visible() {
        if drive(&mut app.import_config_picker, key).is_confirmed() {
            if let Some(selected) = app.import_config_picker.take_selected() {
                if let Some(selection) = App::import_selection_from_picker(&selected.id) {
                    app.open_import_config_preview(selection);
                }
            }
        }
        return Consumed;
    }
    // The command palette's confirmed entry becomes the prompt's next input,
    // which the caller submits.
    if app.command_palette.is_visible() {
        if drive(&mut app.command_palette, key).is_confirmed() {
            if let Some(selected) = app.command_palette.take_selected() {
                app.prompt_input.replace_text(selected.id.clone());
                return Submit;
            }
        }
        return Consumed;
    }

    // Right-click context menu — a `DialogSelectState` list modal (same form as
    // the `/effort` picker): Enter runs the highlighted entry, Esc closes.
    if app.context_menu.is_visible() {
        let out = drive(&mut app.context_menu, key);
        if out.is_confirmed() {
            app.execute_context_menu_item();
        } else if out.is_cancelled() {
            app.dismiss_context_menu();
        }
        return Consumed;
    }

    // ---- Startup / consent dialogs --------------------------------------
    // Bypass-permissions gate: the user must accept or the session exits.
    // Accepting is remembered in settings.json so the warning is shown once.
    if app.bypass_permissions_dialog.is_visible() {
        let out = drive(&mut app.bypass_permissions_dialog, key);
        if out.is_confirmed() {
            // "Yes, I accept" — dismiss and continue.
            let _ = App::persist_bypass_permissions_accepted();
        } else if out.is_cancelled() {
            // "No, exit" — quit immediately.
            app.should_exit = true;
        }
        return Consumed;
    }
    // File injection dialog: shown when oversized files are detected in @refs.
    if app.file_injection_dialog.is_visible() {
        let out = drive(&mut app.file_injection_dialog, key);
        if out.is_cancelled() {
            // Abort path (Esc, or Enter on directory-only): restore the
            // stashed input to the prompt so the user can edit it.
            if let Some(input) = app.file_injection_dialog.pending_input.clone() {
                app.set_prompt_text(input);
            }
        }
        return Consumed;
    }
    // Onboarding: first launch, dismissed with Enter/→/Esc.
    if app.onboarding_dialog.is_visible() {
        if drive(&mut app.onboarding_dialog, key).is_confirmed() {
            // Reached the final page — persist that onboarding is complete.
            let _ = App::persist_onboarding_complete();
        }
        return Consumed;
    }
    // `/effort` picker — the shared list picker: Enter returns Confirmed and the
    // chosen rung id maps straight onto `EffortLevel`.
    if app.effort_dialog.is_visible() {
        if drive(&mut app.effort_dialog, key).is_confirmed() {
            if let Some(selected) = app.effort_dialog.take_selected() {
                if let Some(level) = claurst_core::effort::EffortLevel::from_str(&selected.id) {
                    // Applying `Ultracode` here is equivalent to typing the
                    // `ultracode` keyword: it sets the effort to the top level.
                    app.effort_level = level;
                    app.status_message = Some(format!(
                        "Effort set to {} {}.",
                        level.symbol(),
                        level.label()
                    ));
                }
            }
        }
        return Consumed;
    }
    // ---- Provider setup dialogs -----------------------------------------
    // Device code / browser auth dialog (GitHub Copilot, Anthropic OAuth).
    // While waiting the dialog swallows every key; after Success any key
    // closes it as `Confirmed` (store the credential), after Error / Esc as
    // `Cancelled`.
    if app.device_auth_dialog.is_visible() {
        let out = drive(&mut app.device_auth_dialog, key);
        if out.is_close() {
            if out.is_confirmed() {
                if let crate::dialogs::device_auth_dialog::DeviceAuthStatus::Success(ref token) =
                    app.device_auth_dialog.status
                {
                    let provider_id = app.device_auth_dialog.provider_id.clone();
                    let provider_name = app.device_auth_dialog.provider_name.clone();
                    let token = token.clone();
                    if provider_id == "anthropic-oauth" {
                        // The claude.ai OAuth flow already persisted the Bearer
                        // tokens via save_and_register; the anthropic provider
                        // reads them directly. Switch to the real "anthropic"
                        // provider without re-storing the token as an API key.
                        app.device_auth_pending = None;
                        app.device_auth_dialog.close();
                        app.activate_provider(
                            "anthropic".to_string(),
                            "Anthropic".to_string(),
                            "Connected to",
                        );
                        // The live client was built at startup with no
                        // credential; the main loop re-resolves the freshly
                        // saved Bearer and swaps in a working client.
                        app.pending_provider_reload = true;
                        return Consumed;
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
                    app.auth_store.set(&provider_id, credential);
                    app.activate_provider(provider_id, provider_name, "Connected to");
                }
            }
            app.device_auth_pending = None;
        }
        return Consumed;
    }
    // Ask-user question dialog (AskUserQuestion tool).
    if app.ask_user_dialog.is_visible() {
        if drive(&mut app.ask_user_dialog, key).is_cancelled() {
            // Esc — send an empty reply so the tool result signals
            // "user dismissed".
            app.ask_user_dialog.dismiss();
        }
        return Consumed;
    }
    // API key input dialog (opened from /connect for key-based providers).
    if app.key_input_dialog.is_visible() {
        // Ctrl/Super+V paste stays app-level (some terminals don't emit
        // Event::Paste).
        if key.code == KeyCode::Char('v') && (ctrl || super_key) {
            if let Some(text) = crate::image_paste::read_clipboard_text() {
                if !text.is_empty() {
                    for ch in text.chars() {
                        app.key_input_dialog.insert_char(ch);
                    }
                }
            }
            return Consumed;
        }
        if drive(&mut app.key_input_dialog, key).is_confirmed() {
            let provider_id = app.key_input_dialog.provider_id.clone();
            let provider_name = app.key_input_dialog.provider_name.clone();
            let api_key = app.key_input_dialog.take_key();
            if !api_key.is_empty() {
                app.auth_store.set(
                    &provider_id,
                    claurst_core::StoredCredential::ApiKey { key: api_key },
                );
                app.activate_provider(provider_id, provider_name, "Connected to");
            }
        }
        return Consumed;
    }
    // "Free" composite-provider setup (any subset of free-tier upstream keys,
    // min 1 to enable; more = better).
    if app.free_mode_dialog.is_visible() {
        if key.code == KeyCode::Char('v') && (ctrl || super_key) {
            // Paste clipboard text into the focused field (terminals that
            // don't emit Event::Paste, e.g. Windows Terminal).
            if let Some(text) = crate::image_paste::read_clipboard_text() {
                if !text.is_empty() {
                    for ch in text.chars() {
                        app.free_mode_dialog.insert_char(ch);
                    }
                }
            }
            return Consumed;
        }
        if drive(&mut app.free_mode_dialog, key).is_confirmed() {
            let values = app.free_mode_dialog.take_values();
            for (provider_id, key) in values {
                app.auth_store.set(
                    provider_id,
                    claurst_core::StoredCredential::ApiKey { key },
                );
            }
            app.activate_provider("free".to_string(), "Free Mode".to_string(), "Connected to");
        }
        return Consumed;
    }
    // Custom provider dialog (URL + API key for OpenAI-compatible providers).
    if app.custom_provider_dialog.is_visible() {
        if key.code == KeyCode::Char('v') && (ctrl || super_key) {
            if let Some(text) = crate::image_paste::read_clipboard_text() {
                if !text.is_empty() {
                    for ch in text.chars() {
                        app.custom_provider_dialog.insert_char(ch);
                    }
                }
            }
            return Consumed;
        }
        if drive(&mut app.custom_provider_dialog, key).is_confirmed() {
            let provider_id = app.custom_provider_dialog.provider_id.clone();
            let provider_name = app.custom_provider_dialog.provider_name.clone();
            let (base_url, api_key) = app.custom_provider_dialog.take_values();
            app.persist_custom_provider_base_url(&base_url);
            app.auth_store.set(
                &provider_id,
                claurst_core::StoredCredential::ApiKey { key: api_key },
            );
            app.activate_provider(provider_id, provider_name, "Connected to");
        }
        return Consumed;
    }
    // Import-config preview + confirmation.
    if app.import_config_dialog.is_visible() {
        if drive(&mut app.import_config_dialog, key).is_confirmed() {
            app.perform_import_config();
        }
        return Consumed;
    }
    // Startup error dialog for malformed settings.json / AGENTS.md.
    if app.invalid_config_dialog.is_visible() {
        let _ = drive(&mut app.invalid_config_dialog, key);
        return Consumed;
    }

    // ---- Model / effort --------------------------------------------------
    if app.model_picker.is_visible() {
        if drive(&mut app.model_picker, key).is_confirmed() {
            if let Some((model_id, effort)) = app.model_picker.confirm() {
                // Picking a model other than the fast-mode model while fast
                // mode was active turns fast mode off.
                if app.fast_mode && !app.model_picker.is_selected_fast_mode_model(&model_id) {
                    app.fast_mode = false;
                }
                if let Some(e) = effort {
                    app.effort_level = e;
                }
                // Store explicit selections in the canonical
                // "provider/model" form for non-Anthropic providers.
                // The "free" composite's picker entries already carry a
                // routing prefix (`free/…`, `zen/…`, `openrouter/…`)
                // so re-prefixing would produce nonsense like `free/free/auto`.
                let provider = app.config.provider.as_deref().unwrap_or("anthropic");
                let full_model = if provider == "anthropic" || provider == "free" {
                    model_id.clone()
                } else {
                    format!("{}/{}", provider, model_id)
                };
                app.set_model(full_model.clone());
                app.persist_provider_and_model();
                let effort_hint = effort
                    .map(|e| format!(" [{}]", e.label()))
                    .unwrap_or_default();
                app.status_message = Some(format!("Model: {}{}", full_model, effort_hint));
            }
        }
        return Consumed;
    }

    // ---- Sessions --------------------------------------------------------
    if app.session_branching.is_visible() {
        if drive(&mut app.session_branching, key).is_confirmed() {
            use crate::dialogs::session_branching::BranchBrowserMode;
            match app.session_branching.mode {
                BranchBrowserMode::Browse => {
                    if let Some(branch) = app.session_branching.selected_branch() {
                        app.status_message = Some(format!("Switched to branch: {}", branch.name));
                    }
                    app.session_branching.close();
                }
                BranchBrowserMode::CreateNew => {
                    if let Some((name, at_msg)) = app.session_branching.confirm_create_new() {
                        app.status_message =
                            Some(format!("Created branch: {} at message {}", name, at_msg));
                        app.session_branching.close();
                    }
                }
                BranchBrowserMode::ConfirmDelete => {
                    if let Some(branch_id) = app.session_branching.confirm_delete() {
                        app.status_message = Some(format!("Deleted branch: {}", branch_id));
                    }
                }
            }
        }
        return Consumed;
    }
    if app.session_browser.is_visible() {
        if drive(&mut app.session_browser, key).is_confirmed() {
            use crate::dialogs::session_browser::SessionBrowserMode;
            match app.session_browser.mode {
                SessionBrowserMode::Rename => {
                    if let Some((_id, name)) = app.session_browser.confirm_rename() {
                        app.session_title = Some(name.clone());
                        app.status_message = Some(format!("Renamed to: {}", name));
                    }
                    app.session_browser.close();
                }
                SessionBrowserMode::Confirm => {
                    app.session_browser.close();
                }
                _ => {}
            }
        }
        return Consumed;
    }
    // Export format picker (/export).
    if app.export_dialog.is_visible() {
        if drive(&mut app.export_dialog, key).is_confirmed() {
            let _ = app.perform_export();
        }
        return Consumed;
    }

    // ---- Approvals / surveys / browsers ---------------------------------
    // MCP approval: Esc → Cancelled (deny); Enter / digit / n → Confirmed
    // with the highlighted choice.
    if app.mcp_approval.is_visible() {
        let out = drive(&mut app.mcp_approval, key);
        if out.is_cancelled() {
            app.handle_mcp_approval_decision(crate::dialogs::McpApprovalChoice::Deny);
        } else if out.is_confirmed() {
            let choice = app.mcp_approval.confirm();
            app.handle_mcp_approval_decision(choice);
        }
        return Consumed;
    }
    if app.feedback_survey.is_visible() {
        let _ = drive(&mut app.feedback_survey, key);
        return Consumed;
    }
    if app.memory_file_selector.is_visible() {
        let _ = drive(&mut app.memory_file_selector, key);
        return Consumed;
    }
    // Hooks config menu: `Esc`/`q` go back one level (closing only from the
    // top-level event list), `Enter` drills in, `↑↓`/`jk` select.
    if app.hooks_config_menu.is_visible() {
        let _ = drive(&mut app.hooks_config_menu, key);
        return Consumed;
    }
    if app.diff_viewer.is_visible() {
        // 'd' toggles the diff scope and needs the project root (on App).
        if key.code == KeyCode::Char('d') && key.modifiers.is_empty() {
            let root = app.project_root();
            app.diff_viewer.toggle_diff_type(&root);
            return Consumed;
        }
        let _ = drive(&mut app.diff_viewer, key);
        return Consumed;
    }
    if app.stats_dialog.is_visible() {
        let _ = drive(&mut app.stats_dialog, key);
        return Consumed;
    }

    // ---- Full-screen screens --------------------------------------------
    // `handle_settings_key` adapts the screen's DialogBehavior pipeline and
    // drains the pending `&mut Config` apply the dialog cannot perform.
    if app.settings_screen.is_visible() {
        crate::dialogs::settings_screen::handle_settings_key(
            &mut app.settings_screen,
            &mut app.config,
            key,
        );
        return Consumed;
    }
    // `handle_theme_key` adapts the picker's pipeline and returns the
    // confirmed theme name.
    if app.theme_screen.is_visible() {
        if let Some(theme_name) =
            crate::dialogs::theme_screen::handle_theme_key(&mut app.theme_screen, key)
        {
            app.apply_theme(&theme_name);
        }
        return Consumed;
    }
    // Help dialog (? / F1 / /help) — a DialogCore modal.
    if app.help_dialog.is_visible() {
        let _ = drive(&mut app.help_dialog, key);
        return Consumed;
    }

    // ---- Permission gate (highest-priority blocking dialog) -------------
    if app.permission_request.is_some() {
        crate::dialogs::permission::handle_permission_key(app, key);
        return Consumed;
    }

    // ---- Startup upsell / MCP elicitation -------------------------------
    // Desktop upsell startup dialog.
    if app.desktop_upsell.is_visible() {
        let _ = drive(&mut app.desktop_upsell, key);
        return Consumed;
    }
    // MCP elicitation form requested by an MCP server.
    if app.elicitation.is_visible() {
        if drive(&mut app.elicitation, key).is_cancelled() {
            // Esc — queue a Cancelled result so the caller can take_result().
            app.elicitation.cancel();
        }
        return Consumed;
    }

    ModalKeyOutcome::NoModal
}
