//! Overlay/dialog orchestration.

use std::sync::Arc;

use crate::dialogs::export_dialog::ExportFormat;
use super::App;

impl App {
    pub(super) fn close_secondary_views(&mut self) {
        self.stats_dialog.close();
        self.diff_viewer.close();
        self.feedback_survey.close();
        self.memory_file_selector.close();
        self.hooks_config_menu.close();
        self.model_picker.close();
        self.session_browser.close();
        self.session_branching.close();
        self.export_dialog.dismiss();
        self.connect_dialog.close();
        self.import_config_picker.close();
        self.import_config_dialog.close();
        self.command_palette.close();
        self.key_input_dialog.close();
        self.custom_provider_dialog.close();
        self.free_mode_dialog.close();
        self.device_auth_dialog.close();
        self.settings_screen.close();
        self.theme_screen.close();
    }

    pub fn any_modal_open(&self) -> bool {
        self.permission_request.is_some()
            || self.help_overlay.visible
            || self.show_help
            || self.settings_screen.is_visible()
            || self.theme_screen.is_visible()
            || self.stats_dialog.is_visible()
            || self.diff_viewer.is_visible()
            || self.feedback_survey.is_visible()
            || self.memory_file_selector.is_visible()
            || self.hooks_config_menu.is_visible()
            || self.desktop_upsell.is_visible()
            || self.import_config_dialog.is_visible()
            || self.invalid_config_dialog.is_visible()
            || self.bypass_permissions_dialog.is_visible()
            || self.ask_user_dialog.is_visible()
            || self.onboarding_dialog.is_visible()
            || self.import_config_picker.is_visible()
            || self.connect_dialog.is_visible()
            || self.key_input_dialog.is_visible()
            || self.custom_provider_dialog.is_visible()
            || self.free_mode_dialog.is_visible()
            || self.device_auth_dialog.is_visible()
            || self.command_palette.is_visible()
            || self.elicitation.is_visible()
            || self.model_picker.is_visible()
            || self.effort_picker.visible
            || self.session_browser.is_visible()
            || self.session_branching.is_visible()
            || self.export_dialog.is_visible()
            || self.mcp_approval.is_visible()
            || self.file_injection_dialog.is_visible()
            || self.context_menu_state.is_some()
    }

    /// Perform the export based on the selected format. Returns the path written.
    pub fn perform_export(&mut self) -> Option<String> {
        use crate::dialogs::export_dialog::{export_as_json, export_as_markdown};
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let (filename, content) = match self.export_dialog.selected {
            ExportFormat::Json => {
                let json = export_as_json(&self.messages, self.session_title.as_deref());
                let s = serde_json::to_string_pretty(&json).unwrap_or_default();
                (format!("claude-export-{}.json", ts), s)
            }
            ExportFormat::Markdown => {
                let md = export_as_markdown(&self.messages, self.session_title.as_deref());
                (format!("claude-export-{}.md", ts), md)
            }
        };
        if std::fs::write(&filename, &content).is_ok() {
            self.export_dialog.dismiss();
            Some(filename)
        } else {
            None
        }
    }

    pub(super) fn project_root(&self) -> std::path::PathBuf {
        self.config
            .project_dir
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    }
    pub fn attach_mcp_manager(&mut self, mcp_manager: Arc<claurst_mcp::McpManager>) {
        self.mcp_manager = Some(mcp_manager);
    }
    pub fn take_pending_mcp_panel_auth(&mut self) -> Option<String> {
        self.pending_mcp_panel_auth.take()
    }

    pub fn take_pending_mcp_reconnect(&mut self) -> bool {
        let pending = self.pending_mcp_reconnect;
        self.pending_mcp_reconnect = false;
        pending
    }

    pub fn take_pending_provider_reload(&mut self) -> bool {
        let pending = self.pending_provider_reload;
        self.pending_provider_reload = false;
        pending
    }

    /// If a project MCP server is waiting for approval and no approval dialog
    /// is currently open, pop the next one and show the approval dialog for it.
    ///
    /// Called from the main loop. Returns `true` when a dialog was shown.
    pub fn maybe_prompt_next_mcp_server(&mut self) -> bool {
        if self.mcp_approval.is_visible() || self.mcp_prompting.is_some() {
            return false;
        }
        if let Some(server) = self.mcp_pending_project.pop_front() {
            self.mcp_approval.show(
                &server.name,
                server.url.as_deref(),
                server.command.as_deref(),
                // Tools are unknown until the server is launched; the dialog
                // shows the command/url so the user can judge before running it.
                Vec::new(),
            );
            self.mcp_prompting = Some(server);
            true
        } else {
            false
        }
    }

    /// Apply the user's decision for the project MCP server currently shown in
    /// the approval dialog. Persists "always allow" choices to the on-disk
    /// trust store and requests an MCP reconnect when a server is approved.
    pub fn handle_mcp_approval_decision(&mut self, choice: crate::dialogs::McpApprovalChoice) {
        use crate::dialogs::McpApprovalChoice;
        let server = match self.mcp_prompting.take() {
            Some(s) => s,
            None => return,
        };
        match choice {
            McpApprovalChoice::AllowSession => {
                self.mcp_session_trusted
                    .insert(claurst_core::mcp_trust::server_fingerprint(&server));
                self.pending_mcp_reconnect = true;
                self.status_message = Some(format!(
                    "Approved MCP server '{}' for this session.",
                    server.name
                ));
            }
            McpApprovalChoice::AllowAlways => {
                self.mcp_session_trusted
                    .insert(claurst_core::mcp_trust::server_fingerprint(&server));
                if let Some(root) = self.mcp_project_root.clone() {
                    let mut store = claurst_core::mcp_trust::McpTrustStore::load();
                    store.approve(&root, &server);
                    if let Err(e) = store.save() {
                        self.status_message = Some(format!(
                            "Approved '{}', but failed to persist trust: {}",
                            server.name, e
                        ));
                    } else {
                        self.status_message = Some(format!(
                            "Always allowing MCP server '{}' for this project.",
                            server.name
                        ));
                    }
                } else {
                    self.status_message = Some(format!(
                        "Approved MCP server '{}' (no project root to persist to).",
                        server.name
                    ));
                }
                self.pending_mcp_reconnect = true;
            }
            McpApprovalChoice::Deny => {
                self.status_message = Some(format!(
                    "Skipped project MCP server '{}'.",
                    server.name
                ));
            }
        }
    }

    /// Persist `has_completed_onboarding = true` to the settings file.
    /// Best-effort: failures are silently ignored to not disrupt the session.
    pub(super) fn persist_onboarding_complete() -> anyhow::Result<()> {
        let mut settings = claurst_core::config::Settings::load_sync()?;
        settings.has_completed_onboarding = true;
        settings.save_sync()
    }

    /// Public wrapper so the main loop can mark onboarding complete without
    /// going through the dialog flow.
    pub fn persist_onboarding_complete_pub() -> anyhow::Result<()> {
        Self::persist_onboarding_complete()
    }

    /// Persist `skip_dangerous_mode_permission_prompt = true` to the settings
    /// file after the user accepts the Bypass Permissions warning, so the
    /// dialog is a one-time gate rather than shown on every launch.
    /// Best-effort: failures are silently ignored to not disrupt the session.
    pub(super) fn persist_bypass_permissions_accepted() -> anyhow::Result<()> {
        let mut settings = claurst_core::config::Settings::load_sync()?;
        settings.skip_dangerous_mode_permission_prompt = true;
        settings.save_sync()
    }

}
