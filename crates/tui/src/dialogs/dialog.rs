// dialog.rs — Generic dialog base component (`DialogCore`) + `DialogBehavior`.
//
// Every dialog in this TUI shares the same skeleton: a centred modal frame on
// a dark overlay, a title bar, a body, an optional footer, and a strict rule
// for capturing keyboard / mouse events so input never "leaks" through to the
// transcript underneath.
//
// `DialogCore` implements that skeleton once. Concrete dialogs embed it and
// fill in only their own behaviour by implementing `DialogBehavior`:
//
// ```ignore
// struct MyDialog {
//     core: DialogCore,
//     // ... own state ...
// }
//
// impl DialogBehavior for MyDialog {
//     dialog_core_accessors!();
//
//     fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
//         // extra keys; Esc / Tab are already handled by the dispatch pipeline
//         DialogOutcome::Ignored
//     }
//     fn on_mouse(&mut self, mouse: MouseEvent) -> DialogOutcome {
//         DialogOutcome::Ignored
//     }
//     fn render_content(&mut self, frame: &mut Frame, layout: &ModalLayout) {
//         // draw body (and footer) inside layout.body_area / layout.footer_area
//     }
// }
//
// // In the app event loop — call BEFORE any other key / mouse handling:
// let out = self.my_dialog.handle_key(key);
// if out.is_close() { /* act on Confirmed / Cancelled */ }
// ```
//
// Event-capture contract
// ----------------------
// * Only visible dialogs handle events; invisible ones always return `Ignored`.
// * Built-in keys: `Esc` closes the dialog (returns `Cancelled`; override
//   [`DialogBehavior::on_escape`] when `Esc` needs to do something first),
//   `Tab` / `Shift+Tab` cycle focus zones.
// * Everything else is first offered to `DialogBehavior::on_key`.
// * Dialogs are always modal: while visible they capture EVERY key and mouse
//   event — even ones their behaviour ignores — so the UI underneath never
//   reacts. The caller only ever sees `Confirmed` / `Cancelled` (plus
//   `Ignored` while the dialog is not visible).
// * A left click outside a dialog with `dismiss_on_outside_click()` closes it
//   (returns `Cancelled`); every other outside event is swallowed.

use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::cell::Cell;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::overlays::{begin_modal_frame, ModalLayout, CLAURST_ACCENT};

// ---------------------------------------------------------------------------
// Boilerplate remover
// ---------------------------------------------------------------------------

/// Generate the two `DialogBehavior` accessors for a dialog whose embedded
/// core is the field `core`.
///
/// Every dialog implements [`DialogBehavior::core`] and
/// [`DialogBehavior::core_shared`] the same three-line way, so a dialog can
/// replace that boilerplate with one macro call and keep the rest of its
/// `impl` block as ordinary Rust:
///
/// ```ignore
/// impl DialogBehavior for MyDialog {
///     dialog_core_accessors!();
///
///     fn on_key(&mut self, key: KeyEvent) -> DialogOutcome {
///         // …
///     }
/// }
/// ```
macro_rules! dialog_core_accessors {
    () => {
        fn core(&mut self) -> &mut $crate::dialogs::dialog::DialogCore {
            &mut self.core
        }

        fn core_shared(&self) -> &$crate::dialogs::dialog::DialogCore {
            &self.core
        }
    };
}
pub(crate) use dialog_core_accessors;

// ---------------------------------------------------------------------------
// Outcome
// ---------------------------------------------------------------------------

/// What happened after a dialog handled an input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogOutcome {
    /// Event consumed; the dialog stays open.
    Handled,
    /// The dialog is not interested in the event. Dialogs are modal, so the
    /// event is still swallowed; the one observable effect is that an ignored
    /// `Tab` / `Shift+Tab` falls back to focus-zone cycling.
    Ignored,
    /// The dialog closed with a positive/affirmative semantic (Enter, OK…).
    Confirmed,
    /// The dialog closed without action (Esc, outside click…).
    Cancelled,
}

impl DialogOutcome {
    /// `true` when the event was consumed by the dialog (not `Ignored`).
    pub fn is_handled(self) -> bool {
        !matches!(self, DialogOutcome::Ignored)
    }

    /// `true` when handling the event closed the dialog.
    pub fn is_close(self) -> bool {
        matches!(self, DialogOutcome::Confirmed | DialogOutcome::Cancelled)
    }

    /// `true` when handling the event closed the dialog affirmatively.
    pub fn is_confirmed(self) -> bool {
        matches!(self, DialogOutcome::Confirmed)
    }

    /// `true` when handling the event closed the dialog without action.
    pub fn is_cancelled(self) -> bool {
        matches!(self, DialogOutcome::Cancelled)
    }
}

// ---------------------------------------------------------------------------
// Behaviour hook trait
// ---------------------------------------------------------------------------

/// Per-dialog behaviour implemented by concrete dialogs built on `DialogCore`.
///
/// A concrete dialog embeds a `DialogCore` and exposes it via the required
/// [`DialogBehavior::core`] accessor; the dispatch pipeline (`handle_key`,
/// `handle_mouse`, `render`) is provided by the trait, so the app event loop
/// simply calls `my_dialog.handle_key(key)`.
///
/// All hook methods have defaults, so a dialog only overrides what it needs.
/// Hooks mutate their own `core` field directly (no extra `&mut DialogCore`
/// parameter — that would alias with `&mut self`).
pub trait DialogBehavior {
    /// Access the embedded `DialogCore` (usually `&mut self.core`).
    fn core(&mut self) -> &mut DialogCore;

    /// Immutable access to the embedded `DialogCore` (usually `&self.core`).
    /// Required because rendering happens on shared paths (`render_app` only
    /// has `&App`), where `core()` is not callable.
    fn core_shared(&self) -> &DialogCore;

    /// Number of focus zones `Tab` cycles through (default 1).
    fn focus_zones(&self) -> usize {
        1
    }

    /// Handle a key event not consumed by the built-in defaults
    /// (`Esc` → [`DialogBehavior::on_escape`], `Tab`/`Shift+Tab` cycle focus).
    /// Return `Ignored` if the key is not relevant — it is still swallowed
    /// (dialogs are modal), but an ignored `Tab` falls back to focus cycling.
    fn on_key(&mut self, _key: KeyEvent) -> DialogOutcome {
        DialogOutcome::Ignored
    }

    /// Handle `Esc`. The default closes the dialog and returns `Cancelled`.
    ///
    /// Override when `Esc` has dialog-specific precedence — e.g. a screen that
    /// first cancels an in-progress edit, or a drill-down browser that goes
    /// back one level before closing. Return `Handled` to keep the dialog open.
    fn on_escape(&mut self) -> DialogOutcome {
        self.core().close();
        DialogOutcome::Cancelled
    }

    /// Handle a mouse event whose cursor is inside the dialog area.
    fn on_mouse(&mut self, _mouse: MouseEvent) -> DialogOutcome {
        DialogOutcome::Ignored
    }

    /// Render the dialog content. The modal frame, overlay and title bar are
    /// already drawn by [`DialogBehavior::render`]; draw the body inside
    /// `layout.body_area` and (optionally) hints inside `layout.footer_area`.
    /// Takes `&self`: render paths are shared (`render_app` holds `&App`), so
    /// any render products (hit rects, row maps) must use interior mutability.
    fn render_content(&self, _frame: &mut Frame, _layout: &ModalLayout) {}

    // -- Dispatch pipeline (provided) ------------------------------------

    /// Feed a key event through the capture pipeline. Call this BEFORE the
    /// app-level key handler while the dialog is open.
    fn handle_key(&mut self, key: KeyEvent) -> DialogOutcome {
        if !self.core().is_visible() {
            return DialogOutcome::Ignored;
        }

        // Built-in defaults first.
        match key.code {
            KeyCode::Esc => {
                return self.on_escape();
            }
            KeyCode::BackTab => {
                let zones = self.focus_zones();
                self.core().cycle_focus(-1, zones);
                return DialogOutcome::Handled;
            }
            KeyCode::Tab => {
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                let zones = self.focus_zones();
                self.core().cycle_focus(if shift { -1 } else { 1 }, zones);
                return DialogOutcome::Handled;
            }
            _ => {}
        }

        let out = self.on_key(key);
        // Dialogs are modal: every key is swallowed so nothing reaches the UI
        // below. Only the closing outcomes are reported back to the caller.
        if out.is_close() {
            out
        } else {
            DialogOutcome::Handled
        }
    }

    /// Feed a mouse event through the capture pipeline. Call this BEFORE the
    /// app-level mouse handler while the dialog is open.
    fn handle_mouse(&mut self, mouse: MouseEvent) -> DialogOutcome {
        if !self.core().is_visible() {
            return DialogOutcome::Ignored;
        }

        let dismiss_outside = self.core().dismisses_on_outside_click();
        let inside = self.core().contains(mouse.column, mouse.row);

        if !inside {
            // A left click outside a dismissible dialog closes it; every other
            // outside event is swallowed so nothing reaches the UI below.
            if dismiss_outside && mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                self.core().close();
                return DialogOutcome::Cancelled;
            }
            return DialogOutcome::Handled;
        }

        let out = self.on_mouse(mouse);
        if out.is_close() {
            out
        } else {
            DialogOutcome::Handled
        }
    }

    /// Render the dialog chrome (overlay, frame, background, title) and then
    /// delegate the content to `render_content`.
    fn render(&self, frame: &mut Frame, screen_area: Rect) {
        if !self.core_shared().is_visible() {
            return;
        }

        let (width, height, header_h, footer_h) = {
            let core = self.core_shared();
            (core.width, core.height, core.header_height, core.footer_height)
        };
        let layout =
            begin_modal_frame(frame, screen_area, width, height, header_h, footer_h);

        // Title bar (accent-coloured, truncated to the available width).
        let title_max = layout.header_area.width.saturating_sub(2) as usize;
        let title = truncate_to_width(self.core_shared().title(), title_max);
        if !title.is_empty() && layout.header_area.height > 0 {
            let title_spans = vec![
                Span::styled(" ", Style::default()),
                Span::styled(
                    title,
                    Style::default()
                        .fg(CLAURST_ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            frame.render_widget(Paragraph::new(Line::from(title_spans)), layout.header_area);
        }

        self.core_shared().set_layout(layout);
        self.render_content(frame, &layout);
    }
}

// ---------------------------------------------------------------------------
// DialogCore — the embeddable base state every dialog shares
// ---------------------------------------------------------------------------

/// Generic dialog base: geometry, visibility, focus tracking, hit-testing and
/// the key/mouse event-capture pipeline. Embed one in every dialog `struct`.
#[derive(Debug, Clone)]
pub struct DialogCore {
    title: String,
    /// Requested size; clamped to the screen by `begin_modal_frame`.
    width: u16,
    height: u16,
    header_height: u16,
    footer_height: u16,
    /// Close on left click outside the dialog (default `false`).
    dismiss_on_outside_click: bool,
    visible: bool,
    /// Current focus zone in `0..focus_zones` (used by concrete dialogs).
    focus_zone: usize,
    /// Layout from the last render; used for mouse hit-testing. Stored in a
    /// `Cell` so render paths that only have `&self` (e.g. free render
    /// functions taking `&DialogSelectState`) can still record it.
    last_layout: Cell<ModalLayout>,
}

impl DialogCore {
    /// Create a dialog base with the given title and requested size.
    pub fn new(title: impl Into<String>, width: u16, height: u16) -> Self {
        Self {
            title: title.into(),
            width,
            height,
            header_height: 1,
            footer_height: 1,
            dismiss_on_outside_click: false,
            visible: false,
            focus_zone: 0,
            last_layout: Cell::new(ModalLayout {
                dialog_area: Rect::default(),
                inner_area: Rect::default(),
                header_area: Rect::default(),
                body_area: Rect::default(),
                footer_area: Rect::default(),
            }),
        }
    }

    // -- Builder-style configuration ------------------------------------

    /// Set the header (title bar) height in rows.
    pub fn header_height(mut self, rows: u16) -> Self {
        self.header_height = rows;
        self
    }

    /// Set the footer (hint bar) height in rows.
    pub fn footer_height(mut self, rows: u16) -> Self {
        self.footer_height = rows;
        self
    }

    /// Close the dialog when a left click lands outside it.
    pub fn dismiss_on_outside_click(mut self) -> Self {
        self.dismiss_on_outside_click = true;
        self
    }

    // -- Visibility / geometry -------------------------------------------

    pub fn open(&mut self) {
        self.visible = true;
        self.focus_zone = 0;
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn toggle(&mut self) {
        if self.visible {
            self.close();
        } else {
            self.open();
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    /// Update the requested dialog size (clamped to the screen at render time).
    /// Used by dialogs whose height adapts to their content (e.g. item lists).
    pub fn set_size(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    /// Layout computed during the last render. All zeros before the first
    /// render (or while the dialog has never been drawn).
    pub fn layout(&self) -> ModalLayout {
        self.last_layout.get()
    }

    /// The last rendered dialog area.
    pub fn area(&self) -> Rect {
        self.last_layout.get().dialog_area
    }

    /// Whether an absolute screen position is inside the last rendered area.
    pub fn contains(&self, column: u16, row: u16) -> bool {
        rect_contains(self.last_layout.get().dialog_area, column, row)
    }

    // -- Focus -----------------------------------------------------------

    /// Current focus zone (meaningful to the concrete dialog).
    pub fn focus_zone(&self) -> usize {
        self.focus_zone
    }

    pub fn set_focus_zone(&mut self, zone: usize) {
        self.focus_zone = zone;
    }

    /// Cycle the focus zone by `delta` (wrapping) across `zones` slots.
    fn cycle_focus(&mut self, delta: isize, zones: usize) {
        if zones == 0 {
            self.focus_zone = 0;
            return;
        }
        let current = self.focus_zone as isize;
        let next = (current + delta).rem_euclid(zones as isize);
        self.focus_zone = next as usize;
    }

    /// Whether a left click outside the dialog closes it.
    pub fn dismisses_on_outside_click(&self) -> bool {
        self.dismiss_on_outside_click
    }

    /// Record the layout produced by the last render (for hit-testing).
    /// Takes `&self` so render paths without `&mut` can record too.
    pub fn set_layout(&self, layout: ModalLayout) {
        self.last_layout.set(layout);
    }
}

// ---------------------------------------------------------------------------
// Small shared helpers
// ---------------------------------------------------------------------------

/// Whether an absolute screen position falls inside `rect`.
pub fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    rect.width > 0
        && rect.height > 0
        && column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

/// Truncate a string (by display width) so it fits `max_width` columns.
pub fn truncate_to_width(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }
    if max_width <= 1 {
        return text.chars().take(max_width).collect();
    }
    let mut out = String::new();
    let budget = max_width.saturating_sub(1); // room for the ellipsis
    let mut w = 0;
    for ch in text.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > budget {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    struct NoopDialog {
        core: DialogCore,
    }

    impl NoopDialog {
        fn new() -> Self {
            Self {
                core: DialogCore::new("test", 40, 10),
            }
        }
    }

    impl DialogBehavior for NoopDialog {
        fn core(&mut self) -> &mut DialogCore {
            &mut self.core
        }

        fn core_shared(&self) -> &DialogCore {
            &self.core
        }
    }

    #[test]
    fn invisible_dialog_ignores_everything() {
        let mut dlg = NoopDialog::new();
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(dlg.handle_key(key), DialogOutcome::Ignored);
    }

    #[test]
    fn esc_closes_and_returns_cancelled() {
        let mut dlg = NoopDialog::new();
        dlg.core.open();
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let out = dlg.handle_key(key);
        assert_eq!(out, DialogOutcome::Cancelled);
        assert!(!dlg.core.is_visible());
    }

    #[test]
    fn on_escape_override_can_keep_the_dialog_open() {
        struct EscOverride {
            core: DialogCore,
            handled: bool,
        }
        impl DialogBehavior for EscOverride {
            fn core(&mut self) -> &mut DialogCore {
                &mut self.core
            }
            fn core_shared(&self) -> &DialogCore {
                &self.core
            }
            fn on_escape(&mut self) -> DialogOutcome {
                self.handled = true;
                DialogOutcome::Handled
            }
        }
        let mut dlg = EscOverride {
            core: DialogCore::new("e", 40, 10),
            handled: false,
        };
        dlg.core.open();
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(dlg.handle_key(key), DialogOutcome::Handled);
        assert!(dlg.handled, "the override must be consulted");
        assert!(dlg.core.is_visible(), "Handled keeps the dialog open");
    }

    #[test]
    fn modal_captures_ignored_keys() {
        let mut dlg = NoopDialog::new();
        dlg.core.open();
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(dlg.handle_key(key), DialogOutcome::Handled);
    }

    #[test]
    fn tab_cycles_focus_zones() {
        struct Zoned {
            core: DialogCore,
        }
        impl DialogBehavior for Zoned {
            fn core(&mut self) -> &mut DialogCore {
                &mut self.core
            }

            fn core_shared(&self) -> &DialogCore {
                &self.core
            }

            fn focus_zones(&self) -> usize {
                3
            }
        }
        let mut dlg = Zoned {
            core: DialogCore::new("z", 40, 10),
        };
        dlg.core.open();
        let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        dlg.handle_key(tab);
        assert_eq!(dlg.core.focus_zone(), 1);
        dlg.handle_key(tab);
        dlg.handle_key(tab);
        assert_eq!(dlg.core.focus_zone(), 0);
        let shift_tab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE);
        dlg.handle_key(shift_tab);
        assert_eq!(dlg.core.focus_zone(), 2);
    }

    #[test]
    fn mouse_capture_and_dismiss() {
        // Seed a layout manually so hit-testing has a real area.
        let mut dlg = NoopDialog::new();
        dlg.core.open();
        dlg.core.set_layout(ModalLayout {
            dialog_area: Rect::new(10, 5, 20, 8),
            inner_area: Rect::new(11, 6, 18, 6),
            header_area: Rect::new(11, 6, 18, 1),
            body_area: Rect::new(11, 7, 18, 5),
            footer_area: Rect::new(11, 12, 18, 1),
        });

        let outside = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::NONE,
        };
        // Modal without dismiss: captured, stays open.
        assert_eq!(dlg.handle_mouse(outside), DialogOutcome::Handled);
        assert!(dlg.core.is_visible());

        // Inside click reaches the behavior (default → swallowed as Handled).
        let inside = MouseEvent {
            kind: MouseEventKind::Moved,
            column: 12,
            row: 7,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(dlg.handle_mouse(inside), DialogOutcome::Handled);
    }

    #[test]
    fn dismiss_on_outside_click() {
        // Dismiss is a DialogCore concern — exercise it with a minimal
        // test dialog instead of a concrete widget.
        struct Dismissible {
            core: DialogCore,
        }
        impl DialogBehavior for Dismissible {
            fn core(&mut self) -> &mut DialogCore {
                &mut self.core
            }

            fn core_shared(&self) -> &DialogCore {
                &self.core
            }
        }
        let mut dlg = Dismissible {
            core: DialogCore::new("t", 40, 10),
        };
        dlg.core.open();
        dlg.core.set_layout(ModalLayout {
            dialog_area: Rect::new(10, 5, 20, 8),
            inner_area: Rect::new(11, 6, 18, 6),
            header_area: Rect::new(11, 6, 18, 1),
            body_area: Rect::new(11, 7, 18, 5),
            footer_area: Rect::new(11, 12, 18, 1),
        });
        let outside = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(dlg.handle_mouse(outside), DialogOutcome::Handled);
        assert!(dlg.core.is_visible());

        dlg.core = dlg.core.dismiss_on_outside_click();
        assert_eq!(dlg.handle_mouse(outside), DialogOutcome::Cancelled);
        assert!(!dlg.core.is_visible());
    }

    #[test]
    fn rect_hit_testing() {
        let rect = Rect::new(5, 5, 4, 2);
        assert!(rect_contains(rect, 5, 5));
        assert!(rect_contains(rect, 8, 6));
        assert!(!rect_contains(rect, 9, 6)); // one past the right edge
        assert!(!rect_contains(rect, 6, 7)); // one past the bottom
        assert!(!rect_contains(rect, 4, 5));
    }

    #[test]
    fn truncate_to_width_shortens_with_ellipsis() {
        assert_eq!(truncate_to_width("hello", 10), "hello");
        assert_eq!(truncate_to_width("hello", 4).width(), 4);
        assert!(truncate_to_width("hello", 4).ends_with('…'));
    }
}
