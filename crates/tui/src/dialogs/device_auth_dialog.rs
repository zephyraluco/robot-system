// device_auth_dialog.rs — Device code / browser-based auth overlay.
//
// Provides a modal dialog that shows the device code flow status for GitHub
// Copilot (RFC 8628) and browser-based OAuth for Anthropic.  The actual
// network requests run in a background tokio task; this module only owns the
// display state.
//
// Built on the shared `DialogCore` + `DialogBehavior` base: the modal frame,
// overlay, title bar and the key/mouse capture pipeline come from the base.
// The dialog is display-only while waiting (all keys are swallowed by the
// modal capture); any key after `Success` closes it affirmatively
// (`DialogOutcome::Confirmed`) and any key after `Error` closes it as
// `Cancelled` — the app event loop acts on those outcomes.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::Stylize;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::dialogs::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::overlays::{CLAURST_PANEL_BG, ModalLayout};

// ---------------------------------------------------------------------------
// Status enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceAuthStatus {
    /// Dialog is idle / not started.
    Idle,
    /// Requesting the device code from the authorization server.
    WaitingForCode,
    /// User code is displayed; waiting for the user to authorize in-browser.
    ShowingCode,
    /// Actively polling the token endpoint.
    Polling,
    /// Browser-based auth (Anthropic OAuth) — browser was opened.
    BrowserAuth,
    /// Successfully obtained a token.
    Success(String),
    /// An error occurred.
    Error(String),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct DeviceAuthDialogState {
    pub core: DialogCore,
    pub provider_id: String,
    pub provider_name: String,
    pub status: DeviceAuthStatus,
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,
    /// OAuth URL for browser-based flows (Codex). Shown in the dialog so the
    /// user can copy-paste it when automatic browser launch fails.
    pub auth_url: String,
}

impl Default for DeviceAuthDialogState {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceAuthDialogState {
    pub fn new() -> Self {
        Self {
            // header/footer heights are 0: the title row is part of the body
            // paragraph, matching the original hand-rolled layout.
            core: DialogCore::new("Connect", 64, 14)
                .header_height(0)
                .footer_height(0),
            provider_id: String::new(),
            provider_name: String::new(),
            status: DeviceAuthStatus::Idle,
            user_code: String::new(),
            verification_uri: String::new(),
            device_code: String::new(),
            interval: 5,
            auth_url: String::new(),
        }
    }

    /// Open the dialog for a specific provider and begin the auth flow.
    pub fn open(&mut self, provider_id: String, provider_name: String) {
        self.provider_id = provider_id;
        self.provider_name = provider_name;
        self.status = DeviceAuthStatus::WaitingForCode;
        self.user_code.clear();
        self.verification_uri.clear();
        self.device_code.clear();
        self.core.set_title(format!("Connect {}", self.provider_name));
        self.core.set_size(64, 14);
        self.core.open();
    }

    /// Close and reset the dialog.
    pub fn close(&mut self) {
        self.core.close();
        self.status = DeviceAuthStatus::Idle;
        self.auth_url.clear();
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    /// Switch to BrowserAuth status and store the URL so the dialog can
    /// display it as a copy-paste fallback. The dialog grows to fit the
    /// wrapped URL lines.
    pub fn set_browser_url(&mut self, url: String) {
        self.auth_url = url;
        self.status = DeviceAuthStatus::BrowserAuth;
        let width = 64u16;
        let body_width = width.saturating_sub(4).max(1);
        let url_lines =
            (self.auth_url.len() as u16).saturating_add(body_width - 1) / body_width;
        let height = (14 + url_lines + 2).min(60);
        self.core.set_size(width, height);
    }

    /// Set the device code information received from the authorization server.
    pub fn set_code(
        &mut self,
        user_code: String,
        verification_uri: String,
        device_code: String,
        interval: u64,
    ) {
        self.user_code = user_code;
        self.verification_uri = verification_uri;
        self.device_code = device_code;
        self.interval = interval;
        self.status = DeviceAuthStatus::ShowingCode;
    }

    /// Transition to the polling state (code has been shown, now waiting for
    /// the user to complete authorization).
    pub fn set_polling(&mut self) {
        self.status = DeviceAuthStatus::Polling;
    }

    /// Mark the flow as successful with the obtained token.
    pub fn set_success(&mut self, token: String) {
        self.status = DeviceAuthStatus::Success(token);
    }

    /// Mark the flow as failed.
    pub fn set_error(&mut self, msg: String) {
        self.status = DeviceAuthStatus::Error(msg);
    }
}

// ---------------------------------------------------------------------------
// Events sent from the background task to the main loop
// ---------------------------------------------------------------------------

/// Messages sent from the background device-code / OAuth task back to the
/// main event loop so it can update the dialog state.
pub enum DeviceAuthEvent {
    /// Device code received — show the user code and verification URI.
    GotCode {
        user_code: String,
        verification_uri: String,
        device_code: String,
        interval: u64,
    },
    /// Browser-based OAuth URL is ready — display it so the user can open it
    /// manually if the automatic browser launch failed.
    GotBrowserUrl {
        url: String,
    },
    /// Access token obtained — auth succeeded.
    TokenReceived(String),
    /// Something went wrong.
    Error(String),
}

// ---------------------------------------------------------------------------
// DialogBehavior — shared modal frame + key/mouse capture pipeline
// ---------------------------------------------------------------------------

impl DialogBehavior for DeviceAuthDialogState {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        match &self.status {
            // Any key after success closes affirmatively; the app event loop
            // then stores the credential and activates the provider.
            DeviceAuthStatus::Success(_) => DialogOutcome::Confirmed,
            // Any key after an error closes without action.
            DeviceAuthStatus::Error(_) => DialogOutcome::Cancelled,
            // While waiting, the modal capture swallows everything.
            _ if key.code == KeyCode::Esc => DialogOutcome::Cancelled,
            _ => DialogOutcome::Ignored,
        }
    }

    fn render_content(&self, frame: &mut Frame, layout: &ModalLayout) {
        let pink = Color::Rgb(233, 30, 99);
        let dim = Color::Rgb(90, 90, 90);
        let green = Color::Rgb(80, 200, 120);

        let mut lines: Vec<Line<'static>> = Vec::new();

        // Title row: "Connect {provider}" on the left, right-aligned "esc"
        // hint on the same line (original hand-rolled layout).
        let title_text = self.core.title().to_string();
        let title_pad = layout
            .body_area
            .width
            .saturating_sub(title_text.len() as u16 + 5) as usize;
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {}", title_text),
                Style::default().fg(pink).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>width$}", "esc ", width = title_pad),
                Style::default().fg(dim),
            ),
        ]));

        // Status-dependent content
        match &self.status {
            DeviceAuthStatus::Idle | DeviceAuthStatus::WaitingForCode => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " Requesting device code...",
                    Style::default().fg(Color::Yellow),
                )));
            }
            DeviceAuthStatus::ShowingCode | DeviceAuthStatus::Polling => {
                lines.push(Line::from(""));
                let status_text = if self.status == DeviceAuthStatus::Polling {
                    " Checking for authorization..."
                } else {
                    " Waiting for authorization..."
                };
                lines.push(Line::from(Span::styled(
                    " Enter this code:",
                    Style::default().fg(Color::Rgb(180, 180, 180)),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("    {}", self.user_code),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled(" at ", Style::default().fg(dim)),
                    Span::styled(
                        self.verification_uri.clone(),
                        Style::default()
                            .fg(pink)
                            .add_modifier(Modifier::UNDERLINED),
                    ),
                ]));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    status_text,
                    Style::default().fg(Color::Yellow),
                )));
            }
            DeviceAuthStatus::BrowserAuth => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " Opening browser for authentication...",
                    Style::default().fg(Color::Yellow),
                )));
                if !self.auth_url.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        " If browser didn't open, visit:",
                        Style::default().fg(Color::Rgb(180, 180, 180)),
                    )));
                    lines.push(Line::from(""));
                    // Wrap URL to dialog width
                    let max_w = layout.body_area.width.saturating_sub(2) as usize;
                    for chunk in self.auth_url.as_bytes().chunks(max_w.max(1)) {
                        let s = String::from_utf8_lossy(chunk).into_owned();
                        lines.push(Line::from(Span::styled(
                            format!(" {}", s),
                            Style::default()
                                .fg(pink)
                                .add_modifier(Modifier::UNDERLINED),
                        )));
                    }
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        " (URL copied to clipboard)",
                        Style::default().fg(dim),
                    )));
                } else {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        " Complete the login in your browser.",
                        Style::default().fg(dim),
                    )));
                    lines.push(Line::from(Span::styled(
                        " This dialog will update when done.",
                        Style::default().fg(dim),
                    )));
                }
            }
            DeviceAuthStatus::Success(_) => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " \u{2714} Connected successfully!",
                    Style::default()
                        .fg(green)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " Press any key to continue.",
                    Style::default().fg(dim),
                )));
            }
            DeviceAuthStatus::Error(msg) => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(" Error: {}", msg),
                    Style::default().fg(Color::Red),
                )));
            }
        };

        let para = Paragraph::new(lines).bg(CLAURST_PANEL_BG);
        frame.render_widget(para, layout.body_area);
    }
}
