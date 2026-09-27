//! Slash-command catalog and dispatch.

use claurst_core::config::Theme;
use claurst_core::types::Role;
use crate::dialogs::help_dialog::HelpEntry;
use super::App;
use super::run::open_file_externally;
use super::try_copy_to_clipboard;

pub(super) const PROMPT_SLASH_COMMANDS: &[(&str, &str)] = &[
    ("advisor", "Set or unset the server-side advisor model"),
    ("agent", "List available agents or show agent details"),
    ("changes", "Inspect changes from the current session"),
    ("clear", "Clear the conversation transcript"),
    ("compact", "Compact the conversation context"),
    ("config", "Open settings"),
    ("connect", "Connect an AI provider"),
    ("context", "Show context window and rate limit usage"),
    ("copy", "Copy the last assistant response to clipboard"),
    ("cost", "Show cost breakdown"),
    ("diff", "Inspect the current git diff"),
    ("doctor", "Run diagnostics"),
    ("effort", "Set effort level (low/medium/high/max)"),
    ("exit", "Quit Claurst"),
    ("export", "Export conversation"),
    ("fast", "Toggle fast mode"),
    ("fork", "Fork session into a new branch"),
    ("goal", "Set or view the current session goal"),
    ("heapdump", "Show process memory and diagnostic information"),
    ("help", "Show help"),
    ("hooks", "Browse configured hooks (read-only)"),
    ("import-config", "Import CLAUDE.md and settings.json from ~/.claude"),
    ("init", "Initialize AGENTS.md for this project"),
    ("insights", "Generate a session analysis report with conversation statistics"),
    ("keybindings", "Show keybinding configuration"),
    ("links", "Open URLs from this session in your browser"),
    ("login", "Log in to Claurst"),
    ("logout", "Log out of Claurst"),
    ("managed-agents", "Configure manager-executor managed agent system"),
    ("mcp", "Browse configured MCP servers"),
    ("memory", "Browse and open AGENTS.md memory files"),
    ("model", "Change the AI model"),
    ("move", "Re-home this session to another worktree of the same project"),
    ("new", "Start a fresh session (keeps model, provider & directory)"),
    ("output-style", "Show or switch the output style / persona"),
    ("plugin", "Manage plugins (list/info/enable/disable/reload)"),
    ("providers", "List available AI providers and their status"),
    ("caveman", "Caveman persona output style — save big token"),
    ("rocky", "Rocky persona output style — amaze amaze amaze"),
    ("normal", "Reset persona / output style to default"),
    ("quit", "Exit Claurst"),
    ("refresh", "Clear saved provider auth and model caches"),
    ("rename", "Rename this session"),
    ("resume", "Resume a previous session"),
    ("review", "Review changes (git diff)"),
    ("rewind", "Rewind to an earlier turn"),
    ("session", "Browse and manage sessions"),
    ("settings", "Open settings"),
    ("share", "Upload the current session as a secret gist and get a shareable URL"),
    ("stats", "Open token and cost stats"),
    ("survey", "Open session feedback survey"),
    ("theme", "Open the theme picker"),
    ("ultrareview", "Run an exhaustive multi-dimensional code review"),
    ("update", "Check for updates and upgrade to the latest version"),
    ("upgrade", "Check for updates and upgrade to the latest version"),
    ("vim", "Toggle vim keybindings"),
];

pub(super) fn help_command_category(name: &str) -> &'static str {
    match name {
        "connect" | "model" | "providers" | "refresh" | "fast" | "effort" => "Model & Provider",
        "changes" | "diff" | "review" | "rewind" | "export" | "copy" | "share" | "links" => "Review & History",
        "stats" | "cost" | "context" | "insights" | "heapdump" | "doctor" => "Diagnostics",
        "config" | "settings" | "theme" | "keybindings" | "hooks" | "mcp" | "import-config" => {
            "Workspace"
        }
        "agent" | "memory" | "plugin" | "survey" => "Tools",
        "session" | "resume" | "rename" | "fork" | "clear" | "new" | "move" | "compact"
        | "quit" | "exit" => "Session",
        _ => "Commands",
    }
}

pub(super) fn help_dialog_entries() -> Vec<HelpEntry> {
    PROMPT_SLASH_COMMANDS
        .iter()
        .map(|(name, description)| HelpEntry {
            name: (*name).to_string(),
            aliases: String::new(),
            description: (*description).to_string(),
            category: help_command_category(name).to_string(),
        })
        .collect()
}

impl App {
    /// Open the `/effort` picker.
    ///
    /// The ladder is model-adaptive: it comes from
    /// `claurst_api::supported_efforts(provider, model, registry)`, which returns
    /// the model's supported levels ascending with `Ultracode` always last. The
    /// dialog is the shared list picker (`DialogSelectState`) — a plain vertical
    /// list with the current level badged, no bespoke visuals.
    pub fn open_effort_picker(&mut self) {
        use crate::dialogs::model_picker::EffortLevel;
        use crate::dialogs::dialog_select::SelectItem;

        let provider = self.config.provider.as_deref().unwrap_or("anthropic");
        let model_id = self
            .model_name
            .strip_prefix(&format!("{}/", provider))
            .unwrap_or(&self.model_name);
        let mut levels = claurst_api::supported_efforts(
            provider,
            model_id,
            Some(&self.model_registry),
        );
        if levels.is_empty() {
            // Non-reasoning / unknown model: still offer the common ladder so the
            // picker is never empty.
            levels = vec![
                EffortLevel::Low,
                EffortLevel::Medium,
                EffortLevel::High,
                EffortLevel::Ultracode,
            ];
        }

        let current = self.effort_level;
        self.effort_dialog.items = levels
            .iter()
            .map(|level| SelectItem {
                id: level.as_str().to_string(),
                title: level.label().to_string(),
                description: String::new(),
                // No section header: keep the list flat, like the variant picker.
                category: String::new(),
                badge: (*level == current).then(|| "current".to_string()),
            })
            .collect();

        self.effort_dialog.open();
        // Land on the current level (exact match, else the nearest level below).
        if let Some(idx) = levels
            .iter()
            .position(|level| *level == current)
            .or_else(|| levels.iter().rposition(|level| *level <= current))
        {
            self.effort_dialog.selected_index = idx;
        }
    }

    /// Handle slash commands that should open UI screens rather than execute
    /// as normal commands. Returns `true` if the command was intercepted.
    pub fn intercept_slash_command_with_args(&mut self, cmd: &str, args: &str) -> bool {
        if cmd == "mcp" && !args.trim().is_empty() {
            return false;
        }
        self.intercept_slash_command(cmd)
    }

    pub fn intercept_slash_command(&mut self, cmd: &str) -> bool {
        // Running a command means the user moved on: clear a stale failure from
        // the prompt corner (a command that fails pushes its own notification
        // after this point).
        self.notifications.dismiss_errors();
        self.close_secondary_views();
        match cmd {
            "config" | "settings" => {
                self.settings_screen.open();
                true
            }
            "theme" => {
                let current = match &self.config.theme {
                    Theme::Dark => "dark",
                    Theme::Light => "light",
                    Theme::Default => "default",
                    Theme::Deuteranopia => "deuteranopia",
                    Theme::Custom(s) => s.as_str(),
                };
                self.theme_screen.open(current);
                true
            }
            "stats" => {
                self.stats_dialog.open();
                true
            }
            "diff" | "review" => {
                let root = self.project_root();
                self.diff_viewer.open(&root);
                true
            }
            "changes" => {
                let root = self.project_root();
                self.refresh_turn_diff_from_history();
                self.diff_viewer.open_turn(&root);
                true
            }
            "survey" => {
                self.feedback_survey.open();
                true
            }
            "memory" => {
                let root = self.project_root();
                self.memory_file_selector.open(&root);
                true
            }
            "hooks" => {
                self.hooks_config_menu.open();
                true
            }
            "import-config" => {
                self.open_import_config_picker();
                true
            }
            "connect" => {
                self.connect_dialog.open();
                true
            }
            "model" => {
                if !self.has_credentials {
                    self.connect_dialog.open();
                    self.status_message = Some("Connect a provider to choose a model.".to_string());
                    return true;
                }
                let provider = self
                    .config
                    .provider
                    .clone()
                    .unwrap_or_else(|| "anthropic".to_string());
                self.open_model_picker_for_provider(&provider, None);
                true
            }
            "session" | "resume" => {
                self.session_browser.open(vec![]);
                self.session_list_pending = true;
                true
            }
            // `/new` (opencode's lazy-home) resets the same visible transcript
            // state as `/clear`; the CLI layer then swaps in a brand-new session
            // and overrides the status line to "Started a new session.".
            "clear" | "new" => {
                self.messages.clear();
                self.system_annotations.clear();
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.tool_use_blocks.clear();
                self.turn_metadata.clear();
                self.cost_usd = 0.0;
                self.invalidate_transcript();
                self.status_message = Some("Conversation cleared.".to_string());
                true
            }
            "exit" | "quit" => {
                self.should_exit = true;
                true
            }
            "vim" => {
                self.prompt_input.vim_enabled = !self.prompt_input.vim_enabled;
                let status = if self.prompt_input.vim_enabled { "enabled" } else { "disabled" };
                self.status_message = Some(format!("Vim mode {}.", status));
                self.refresh_prompt_input();
                true
            }
            "fast" => {
                self.fast_mode = !self.fast_mode;
                let status = if self.fast_mode { "enabled" } else { "disabled" };
                self.status_message = Some(format!("Fast mode {}.", status));
                true
            }
            "plan" => {
                use claurst_core::config::PermissionMode;
                self.plan_mode = !self.plan_mode;
                self.config.permission_mode = if self.plan_mode {
                    PermissionMode::Plan
                } else {
                    PermissionMode::Default
                };
                self.status_message = Some(if self.plan_mode {
                    "Plan mode ON — Claurst will plan before acting.".to_string()
                } else {
                    "Plan mode OFF.".to_string()
                });
                // Allow CLI path to also run (sends UserMessage to Claurst).
                false
            }
            "compact" => {
                // Handled by execute_command in the CLI loop (real LLM compaction).
                false
            }
            "copy" => {
                // Copy the last assistant message to the clipboard
                // (xclip/xsel/pbcopy/clip.exe under the hood).
                if let Some(text) = self
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::Assistant)
                    .map(|m| m.get_all_text())
                {
                    let _ = try_copy_to_clipboard(&text);
                }
                true
            }
            "output-style" => {
                self.output_style = match self.output_style.as_str() {
                    "auto" => "stream".to_string(),
                    "stream" => "verbose".to_string(),
                    _ => "auto".to_string(),
                };
                self.status_message = Some(format!("Output style: {}.", self.output_style));
                true
            }
            "effort" => {
                self.open_effort_picker();
                true
            }
            "doctor" => {
                // Handled by execute_command (DoctorCommand).
                false
            }
            "cost" => {
                self.stats_dialog.open();
                true
            }
            "export" => {
                self.export_dialog.open();
                true
            }
            "rename" => {
                self.session_browser.open(vec![]);
                self.session_list_pending = true;
                self.session_browser.start_rename();
                true
            }
            "init" | "login" | "logout" => {
                // Handled by execute_command (CLI-level operations).
                false
            }
            "keybindings" => {
                // Open the keybindings.json file in the external editor
                let keybindings_path = claurst_core::config::Settings::config_dir().join("keybindings.json");

                if let Err(e) = open_file_externally(&keybindings_path) {
                    eprintln!("Failed to open keybindings file: {}", e);
                }
                true
            }
            "help" => {
                // Open the help overlay (same as pressing `?` or F1).
                if !self.help_dialog.is_visible() {
                    self.help_dialog.toggle();
                }
                true
            }
            _ => false,
        }
    }

}
