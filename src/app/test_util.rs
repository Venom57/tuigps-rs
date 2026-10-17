//! Helpers shared by the `app` unit tests.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{ActiveTab, App};
use crate::gpsd_client::{GpsdEvent, GpsdMessage};

pub(crate) fn key_event(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })
}

pub(crate) fn key_event_mod(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })
}

pub(crate) fn new_app() -> App {
    App::new("localhost".to_string(), 2947)
}

pub(crate) fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

pub(crate) fn json_event(line: &str, receipt: f64) -> GpsdEvent {
    GpsdEvent::Message(GpsdMessage::parse(line, receipt).expect("valid gpsd message"))
}

/// A 3D-fix TPV at the given time, received at `receipt`.
pub(crate) fn tpv(time: &str, receipt: f64) -> GpsdEvent {
    json_event(
        &format!(
            r#"{{"class":"TPV","mode":3,"lat":51.5,"lon":-0.1,"altMSL":10.0,"time":"{time}"}}"#
        ),
        receipt,
    )
}

pub(crate) fn hold_count(app: &App) -> u64 {
    app.position_hold
        .as_ref()
        .and_then(|h| h.result())
        .map_or(0, |r| r.count)
}

pub(crate) fn device_app(control: usize) -> App {
    let mut app = new_app();
    app.active_tab = ActiveTab::Device;
    app.device_config.selected_control = control;
    app
}

/// Open settings, select the port field and replace its text.
pub(crate) fn type_port(app: &mut App, text: &str) {
    app.handle_event(key_event(KeyCode::Char('s')));
    app.handle_event(key_event(KeyCode::Down)); // Port
    app.handle_event(key_event(KeyCode::Enter)); // edit
    for _ in 0..10 {
        app.handle_event(key_event(KeyCode::Backspace));
    }
    for c in text.chars() {
        app.handle_event(key_event(KeyCode::Char(c)));
    }
}
