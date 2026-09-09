// ask_user_dialog.rs — TUI overlay for model-initiated questions.
//
// Rendered when the model calls the `AskUserQuestion` tool.  The dialog
// shows the question text, an optional list of predefined choices that the
// user can navigate with arrow keys or number shortcuts, and a free-text
// input line for a custom answer.
// Built on the shared `DialogCore` + `DialogBehavior` base (crate::dialogs::dialog).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use crate::dialogs::dialog::{DialogBehavior, DialogCore, DialogOutcome};
use crate::overlays::{ModalLayout, CLAURST_PANEL_BG};

const TITLE_FG: Color = Color::Rgb(200, 160, 255);
const QUESTION_FG: Color = Color::Rgb(230, 230, 230);
const OPTION_FG: Color = Color::Rgb(190, 190, 210);
const SELECTED_FG: Color = Color::Rgb(255, 255, 255);
const SELECTED_BG: Color = Color::Rgb(55, 55, 90);
const HINT_FG: Color = Color::Rgb(100, 100, 130);
const INPUT_FG: Color = Color::Rgb(200, 255, 200);
const NUMBER_FG: Color = Color::Rgb(150, 150, 200);

/// State for the ask-user question dialog overlay.
pub struct AskUserDialogState {
    /// Embedded generic dialog base (visibility, geometry, title).
    pub core: DialogCore,
    /// The question text from the model.
    pub question: String,
    /// Optional predefined choices.
    pub options: Option<Vec<String>>,
    /// Index of the currently highlighted option (0 = custom-text row when
    /// options is None, or indices into options vec, with the custom row last).
    pub selected_idx: usize,
    /// Custom text the user is typing (if they choose not to pick an option).
    pub custom_text: String,
    /// Whether cursor is in the custom-text input row.
    pub in_custom_input: bool,
    /// Pending reply channel sender — set when the dialog opens, consumed on submit.
    pub(crate) reply_tx: Option<tokio::sync::oneshot::Sender<String>>,
}

impl Default for AskUserDialogState {
    fn default() -> Self {
        Self {
            core: DialogCore::new("Question", 58, 10),
            question: String::new(),
            options: None,
            selected_idx: 0,
            custom_text: String::new(),
            in_custom_input: false,
            reply_tx: None,
        }
    }
}

impl AskUserDialogState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the dialog with a question and optional choices.
    pub fn open(
        &mut self,
        question: String,
        options: Option<Vec<String>>,
        reply_tx: tokio::sync::oneshot::Sender<String>,
    ) {
        self.question = question;
        self.options = options;
        self.selected_idx = 0;
        self.custom_text.clear();
        self.in_custom_input = self.options.is_none();
        self.reply_tx = Some(reply_tx);
        // Height adapts to the question + options content.
        let question_lines = word_wrap(&self.question, 52).len() as u16;
        let options_lines = self.options.as_ref().map(|v| v.len() as u16 + 1).unwrap_or(0);
        let height = (5 + question_lines + options_lines + 3).max(8);
        self.core.set_size(58, height);
        self.core.open();
    }

    pub fn is_visible(&self) -> bool {
        self.core.is_visible()
    }

    /// Navigate selection up.
    pub fn select_prev(&mut self) {
        let n = self.option_count();
        if n == 0 {
            return;
        }
        if self.selected_idx == 0 {
            self.selected_idx = n; // wrap to custom row
            self.in_custom_input = true;
        } else {
            self.selected_idx -= 1;
            self.in_custom_input = self.selected_idx >= self.options_len();
        }
    }

    /// Navigate selection down.
    pub fn select_next(&mut self) {
        let n = self.option_count();
        if n == 0 {
            return;
        }
        if self.selected_idx >= n {
            self.selected_idx = 0;
            self.in_custom_input = false;
        } else {
            self.selected_idx += 1;
            self.in_custom_input = self.selected_idx >= self.options_len();
        }
    }

    /// Select an option directly by 1-based number key.
    pub fn select_by_number(&mut self, n: usize) {
        if let Some(ref opts) = self.options {
            if n >= 1 && n <= opts.len() {
                self.selected_idx = n - 1;
                self.in_custom_input = false;
            }
        }
    }

    /// Append a character to the custom-text input.
    ///
    /// Any printable character auto-switches to the custom row regardless of
    /// where the selection currently is — so the user can just start typing
    /// without having to navigate down with Tab/↓ first.
    pub fn push_char(&mut self, c: char) {
        self.custom_text.push(c);
        self.in_custom_input = true;
        self.selected_idx = self.options_len();
    }

    /// Backspace in the custom-text input.
    pub fn pop_char(&mut self) {
        if self.in_custom_input || self.options.is_none() {
            self.custom_text.pop();
        }
    }

    /// Confirm the current selection and send the answer.
    ///
    /// Returns `true` if the dialog was successfully submitted (i.e. a reply
    /// channel was present).
    pub fn confirm(&mut self) -> bool {
        let answer = if self.in_custom_input || self.options.is_none() {
            self.custom_text.clone()
        } else if let Some(ref opts) = self.options {
            opts.get(self.selected_idx).cloned().unwrap_or_default()
        } else {
            self.custom_text.clone()
        };

        self.send_reply(answer)
    }

    /// Dismiss without answering (sends an empty string so the tool result
    /// signals "user dismissed").
    pub fn dismiss(&mut self) -> bool {
        self.send_reply(String::new())
    }

    fn send_reply(&mut self, answer: String) -> bool {
        self.core.close();
        if let Some(tx) = self.reply_tx.take() {
            let _ = tx.send(answer);
            true
        } else {
            false
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn options_len(&self) -> usize {
        self.options.as_ref().map(|v| v.len()).unwrap_or(0)
    }

    /// Total number of selectable rows: options + custom-text row.
    fn option_count(&self) -> usize {
        self.options_len() + 1
    }
}

impl DialogBehavior for AskUserDialogState {
    fn core(&mut self) -> &mut DialogCore {
        &mut self.core
    }

    fn core_shared(&self) -> &DialogCore {
        &self.core
    }

    fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
        match key.code {
            KeyCode::Enter => {
                self.confirm();
                DialogOutcome::Confirmed
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.select_prev();
                DialogOutcome::Handled
            }
            KeyCode::Down | KeyCode::Tab => {
                self.select_next();
                DialogOutcome::Handled
            }
            KeyCode::Char(c)
                if c.is_ascii_digit()
                    && self.options.is_some()
                    && !self.in_custom_input =>
            {
                // Digit keys select an option by number ONLY when the user
                // is not already typing a custom answer.  Once in custom
                // mode, digits flow through to push_char like any other char.
                let n = (c as u8 - b'0') as usize;
                if n >= 1 {
                    self.select_by_number(n);
                }
                DialogOutcome::Handled
            }
            KeyCode::Char(c) => {
                self.push_char(c);
                DialogOutcome::Handled
            }
            KeyCode::Backspace => {
                self.pop_char();
                DialogOutcome::Handled
            }
            _ => DialogOutcome::Ignored,
        }
    }

    fn render_content(&self, frame: &mut Frame, layout: &ModalLayout) {
        let inner = Rect {
            x: layout.body_area.x,
            y: layout.body_area.y,
            width: layout.body_area.width,
            height: layout.body_area.height,
        };
        let inner_w = inner.width as usize;

        let mut lines: Vec<Line<'static>> = Vec::new();

        // Title row
        lines.push(Line::from(vec![Span::styled(
            " Question ",
            Style::default().fg(TITLE_FG).bg(CLAURST_PANEL_BG).add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));

        // Question text
        for wrap_line in word_wrap(&self.question, inner_w) {
            lines.push(Line::from(Span::styled(
                wrap_line,
                Style::default().fg(QUESTION_FG).bg(CLAURST_PANEL_BG),
            )));
        }
        lines.push(Line::from(""));

        // Option rows
        if let Some(ref opts) = self.options {
            for (i, opt) in opts.iter().enumerate() {
                let is_sel = !self.in_custom_input && self.selected_idx == i;
                let prefix = if is_sel { "▶ " } else { "  " };
                let num_str = format!("{}", i + 1);
                let label = format!(" {}", opt);
                let style_bg = if is_sel { SELECTED_BG } else { CLAURST_PANEL_BG };
                lines.push(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(if is_sel { SELECTED_FG } else { HINT_FG }).bg(style_bg)),
                    Span::styled(num_str, Style::default().fg(NUMBER_FG).bg(style_bg)),
                    Span::styled(label, Style::default().fg(if is_sel { SELECTED_FG } else { OPTION_FG }).bg(style_bg).add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() })),
                ]));
            }
            lines.push(Line::from("")); // spacer before custom row
        }

        // Custom input row
        let is_sel = self.in_custom_input || self.options.is_none();
        let prefix = if is_sel { "❯ " } else { "  " };
        let cursor = if is_sel { "█" } else { "" };
        let style_bg = if is_sel { SELECTED_BG } else { CLAURST_PANEL_BG };
        let mut spans = vec![
            Span::styled(prefix, Style::default().fg(if is_sel { SELECTED_FG } else { HINT_FG }).bg(style_bg)),
        ];
        if self.custom_text.is_empty() && !is_sel && self.options.is_some() {
            // Not yet active: show a subtle prompt so user knows they can type
            spans.push(Span::styled(
                "type to fill custom answer…",
                Style::default().fg(HINT_FG).bg(style_bg),
            ));
        } else {
            let display_text = format!("{}{}", self.custom_text, cursor);
            spans.push(Span::styled(display_text, Style::default().fg(INPUT_FG).bg(style_bg)));
        }
        lines.push(Line::from(spans));
        lines.push(Line::from(""));

        // Hint row
        let hint = if self.options.is_some() {
            "  type: custom   ↑↓/Tab: options   Enter: confirm   Esc: skip"
        } else {
            "  Type answer, then Enter to confirm   Esc: skip"
        };
        lines.push(Line::from(Span::styled(
            hint,
            Style::default().fg(HINT_FG).bg(CLAURST_PANEL_BG),
        )));

        Paragraph::new(lines).render(inner, frame.buffer_mut());
    }
}

// ---------------------------------------------------------------------------
// Word-wrap helper
// ---------------------------------------------------------------------------

fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.len() + 1 + word.len() <= max_width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current.clone());
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}

// ---------------------------------------------------------------------------
// (Rendering is provided by `DialogBehavior::render` + `render_content` above.)
// ---------------------------------------------------------------------------
