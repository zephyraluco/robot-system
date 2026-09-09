// feedback_survey.rs — Session quality survey overlay matching TS FeedbackSurvey.tsx
// Built on the shared `DialogCore` + `DialogBehavior` base (crate::dialogs::dialog).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Alignment;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::dialogs::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::overlays::ModalLayout;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackResponse {
    Bad,
    Fine,
    Good,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackSurveyStage {
    /// Ask 1=Bad 2=Fine 3=Good 0=Dismiss
    Rating,
    /// Ask to share transcript: 1=Yes 2=No 3=DontAskAgain
    SharePrompt,
    /// Show thank-you message
    Thanks,
    /// Survey is closed / not active
    Closed,
}

pub struct FeedbackSurveyState {
    /// Embedded generic dialog base (visibility, geometry, title).
    pub core: DialogCore,
    pub stage: FeedbackSurveyStage,
    pub response: Option<FeedbackResponse>,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl FeedbackSurveyState {
    pub fn new() -> Self {
        Self {
            core: DialogCore::new("Session Feedback", 50, 8),
            stage: FeedbackSurveyStage::Closed,
            response: None,
        }
    }

    pub fn open(&mut self) {
        self.core.open();
        self.stage = FeedbackSurveyStage::Rating;
        self.response = None;
    }

    pub fn close(&mut self) {
        self.core.close();
        self.stage = FeedbackSurveyStage::Closed;
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    /// Handle a digit key press. Returns `true` if the survey consumed the key.
    ///
    /// Stage=Rating:      1→Bad, 2→Fine, 3→Good, 0→Dismissed
    ///   Good → transition to SharePrompt, otherwise → Thanks
    /// Stage=SharePrompt: 1/2/3 → show Thanks and close
    /// Stage=Thanks:      any key → close
    pub fn handle_digit(&mut self, digit: u8) -> bool {
        if !self.is_visible() {
            return false;
        }
        match &self.stage {
            FeedbackSurveyStage::Rating => {
                match digit {
                    0 => {
                        self.response = Some(FeedbackResponse::Dismissed);
                        self.stage = FeedbackSurveyStage::Thanks;
                    }
                    1 => {
                        self.response = Some(FeedbackResponse::Bad);
                        self.stage = FeedbackSurveyStage::Thanks;
                    }
                    2 => {
                        self.response = Some(FeedbackResponse::Fine);
                        self.stage = FeedbackSurveyStage::Thanks;
                    }
                    3 => {
                        self.response = Some(FeedbackResponse::Good);
                        self.stage = FeedbackSurveyStage::SharePrompt;
                    }
                    _ => {}
                }
                true
            }
            FeedbackSurveyStage::SharePrompt => {
                if matches!(digit, 1..=3) {
                    self.stage = FeedbackSurveyStage::Thanks;
                }
                true
            }
            FeedbackSurveyStage::Thanks => {
                self.close();
                true
            }
            FeedbackSurveyStage::Closed => false,
        }
    }
}

impl Default for FeedbackSurveyState {
    fn default() -> Self {
        Self::new()
    }
}

impl DialogBehavior for FeedbackSurveyState {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        if let KeyCode::Char(c) = key.code {
            if let Some(d) = c.to_digit(10) {
                self.handle_digit(d as u8);
                return DialogOutcome::Handled;
            }
        }
        DialogOutcome::Ignored
    }

    fn render_content(&self, frame: &mut Frame, layout: &ModalLayout) {
        let dialog_area = layout.dialog_area;

        let (title, body_lines): (&str, Vec<Line>) = match &self.stage {
            FeedbackSurveyStage::Rating => (
                " Session Feedback ",
                vec![
                    Line::from(""),
                    Line::from(vec![Span::styled(
                        "  How is Claurst doing this session? (optional)",
                        Style::default().fg(Color::White),
                    )]),
                    Line::from(""),
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled("1", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                        Span::raw(" Bad   "),
                        Span::styled("2", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                        Span::raw(" Fine   "),
                        Span::styled("3", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::raw(" Good   "),
                        Span::styled("0", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
                        Span::raw(" Dismiss"),
                    ]),
                ],
            ),
            FeedbackSurveyStage::SharePrompt => (
                " Share Transcript? ",
                vec![
                    Line::from(""),
                    Line::from(vec![Span::styled(
                        "  Can Anthropic look at your session transcript?",
                        Style::default().fg(Color::White),
                    )]),
                    Line::from(""),
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled("1", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::raw(" Yes   "),
                        Span::styled("2", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                        Span::raw(" No   "),
                        Span::styled("3", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
                        Span::raw(" Don't ask again"),
                    ]),
                ],
            ),
            FeedbackSurveyStage::Thanks | FeedbackSurveyStage::Closed => (
                " Thank You ",
                vec![
                    Line::from(""),
                    Line::from(vec![Span::styled(
                        "  Thank you for your feedback!",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )]),
                    Line::from(""),
                    Line::from(vec![Span::styled(
                        "  Press any key to close",
                        Style::default().fg(Color::DarkGray),
                    )]),
                ],
            ),
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Rgb(233, 30, 99)));

        let para = Paragraph::new(body_lines)
            .block(block)
            .alignment(Alignment::Left);

        use ratatui::widgets::Widget;
        para.render(dialog_area, frame.buffer_mut());
    }
}
