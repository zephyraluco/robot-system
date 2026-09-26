//! Background loads pumped by the main loop (`App::tick_background_loads`).
//!
//! The session browser list and the welcome screen's "Recent activity" list are
//! read from disk on a spawned task and delivered over a channel; the frame loop
//! only drains them. These tests cover the drain side and that a pending request
//! is handed to a task exactly once.

use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_tui::app::RecentSession;
use claurst_tui::dialogs::session_browser::SessionEntry;
use claurst_tui::App;

fn app() -> App {
    // `App::new` starts with `recent_sessions_pending = true`; tests drive the
    // flags explicitly so nothing is spawned unless the test wants that.
    let mut app = App::new(Config::default(), CostTracker::new());
    app.session_list_pending = false;
    app.recent_sessions_pending = false;
    app
}

fn entry(id: &str) -> SessionEntry {
    SessionEntry {
        id: id.to_string(),
        title: format!("session {id}"),
        last_updated: "just now".to_string(),
        message_count: 3,
        cost_usd: 0.0,
    }
}

#[test]
fn delivered_session_list_reaches_the_browser() {
    let mut app = app();
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    app.session_list_rx = Some(rx);
    tx.try_send(vec![entry("a"), entry("b")]).unwrap();

    app.tick_background_loads();

    assert_eq!(app.session_browser.sessions.len(), 2);
    assert_eq!(app.session_browser.selected_idx, 0);
    assert!(app.session_list_rx.is_none(), "receiver released after delivery");
}

#[test]
fn delivered_recent_sessions_reach_the_welcome_screen() {
    let mut app = app();
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    app.recent_sessions_rx = Some(rx);
    tx.try_send(vec![RecentSession {
        label: "Fix the parser bug".to_string(),
        mtime: std::time::SystemTime::now(),
    }])
    .unwrap();

    app.tick_background_loads();

    assert_eq!(app.recent_sessions.len(), 1);
    assert_eq!(app.recent_sessions[0].label, "Fix the parser bug");
    assert!(app.recent_sessions_rx.is_none(), "receiver released after delivery");
}

#[test]
fn empty_receiver_is_kept_until_a_batch_arrives() {
    let mut app = app();
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    app.session_list_rx = Some(rx);

    app.tick_background_loads();

    assert!(app.session_list_rx.is_some(), "still waiting");
    assert!(app.session_browser.sessions.is_empty());
}

#[test]
fn dropped_sender_releases_the_receiver() {
    let mut app = app();
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    app.session_list_rx = Some(rx);
    drop(tx);

    app.tick_background_loads();

    assert!(app.session_list_rx.is_none(), "no point polling a dead channel");
}

/// A pending request must be dispatched to a spawned task exactly once — the
/// flag is what keeps the loop from re-listing the whole store every frame.
#[tokio::test]
async fn pending_request_is_dispatched_once() {
    let mut app = app();
    app.session_list_pending = true;

    app.tick_background_loads();

    assert!(!app.session_list_pending, "flag cleared so it runs once");
    assert!(app.session_list_rx.is_some(), "load in flight");
}
