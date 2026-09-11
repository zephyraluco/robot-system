//! dialogs — All TUI dialog components.
//!
//! Every dialog in the TUI is built on the shared `DialogCore` +
//! `DialogBehavior` base (`crate::dialogs::dialog`):
//!
//! - `dialog` — the generic base component (`DialogCore` + `DialogBehavior`).
//! - `dialog_select` — reusable fuzzy-search selection dialog widget.
//! - `permission` — permission dialogs and confirmation dialogs
//!   (`PermissionRequest`, `ToolPermissionDialog`, `McpApprovalDialogState`).
//! - One module per concrete dialog (export, stats, diff viewer, elicitation,
//!   invalid config, bypass permissions, onboarding, key input, custom
//!   provider, free mode, device auth, import config, ask user, file
//!   injection, desktop upsell, feedback survey, memory file selector,
//!   session browser, session branching, model picker, settings screen,
//!   theme picker, hooks browser).

pub mod ask_user_dialog;
pub mod bypass_permissions_dialog;
pub mod custom_provider_dialog;
pub mod desktop_upsell_startup;
pub mod device_auth_dialog;
pub mod dialog;
pub mod dialog_select;
pub mod diff_viewer;
pub mod elicitation_dialog;
pub mod export_dialog;
pub mod feedback_survey;
pub mod file_injection_dialog;
pub mod free_mode_dialog;
pub mod hooks_config_menu;
pub mod import_config_dialog;
pub mod invalid_config_dialog;
pub mod key_input_dialog;
pub mod memory_file_selector;
pub mod model_picker;
pub mod onboarding_dialog;
pub mod permission;
pub mod session_branching;
pub mod session_browser;
pub mod settings_screen;
pub mod stats_dialog;
pub mod theme_screen;

// Re-export the permission dialog API at the `dialogs` root so existing
// callers (`crate::dialogs::PermissionRequest`, …) keep working. All three
// dialogs implement `DialogBehavior` and are driven through `handle_key` /
// `handle_mouse` / `render`.
pub use permission::{
    ElicitationField, ElicitationFieldType, McpApprovalChoice, McpApprovalDialogState,
    PermissionDialogKind, PermissionOption, PermissionRequest, ToolPermissionDialog,
    ToolPermissionKind, render_mcp_approval_dialog, render_mcp_approval_dialog_frame,
    render_permission_dialog, render_tool_permission_dialog,
};