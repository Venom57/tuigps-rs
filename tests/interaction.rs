//! Interaction-level tests through the public API: key sequences plus
//! rendering of the help overlay, settings dialog, device confirmation and
//! NMEA follow indicator. Pure: no gpsd, no ubxtool/gpsctl, config files
//! only in temp dirs.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

use tuigps_rs::app::{ActiveTab, App, GLOBAL_BINDINGS, UnitSystem, tab_bindings};
use tuigps_rs::config;
use tuigps_rs::gpsd_client::GpsdEvent;
use tuigps_rs::ui;

fn key(code: KeyCode) -> Event {
    key_mod(code, KeyModifiers::NONE)
}

fn key_mod(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })
}

fn new_app() -> App {
    App::new("localhost".into(), 2947)
}

fn draw(app: &App, w: u16, h: u16) -> Buffer {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ui::render(f, app)).unwrap();
    term.backend().buffer().clone()
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

fn all_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

// === Help overlay ===

#[test]
fn help_overlay_80x24_lists_global_and_tab_keys() {
    for tab in ActiveTab::ALL {
        let mut app = new_app();
        app.active_tab = *tab;
        app.handle_event(key(KeyCode::Char('?')));
        let text = all_text(&draw(&app, 80, 24));
        assert!(text.contains("Help"), "{text}");
        assert!(text.contains("Global"), "{text}");
        assert!(text.contains(&format!("{} tab", tab.title())), "{text}");
        // At 80x24 every description fits untruncated.
        for b in GLOBAL_BINDINGS.iter().chain(tab_bindings(*tab)) {
            assert!(
                text.contains(b.desc),
                "{tab:?}: {:?} missing\n{text}",
                b.desc
            );
        }
        assert!(text.contains("to close"), "{text}");
    }
}

#[test]
fn help_overlay_40x12_renders_over_too_small_notice() {
    let mut app = new_app();
    app.active_tab = ActiveTab::Nmea;
    app.handle_event(key(KeyCode::Char('?')));
    let text = all_text(&draw(&app, 40, 12));
    assert!(text.contains("Help"), "{text}");
    // Single column, tab keys first.
    assert!(text.contains("NMEA tab"), "{text}");
    assert!(text.contains("Pause"), "{text}");
}

#[test]
fn help_overlay_never_panics_at_any_size() {
    for tab in ActiveTab::ALL {
        let mut app = new_app();
        app.active_tab = *tab;
        app.show_help = true;
        for (w, h) in [
            (0, 0),
            (1, 1),
            (5, 3),
            (20, 5),
            (40, 12),
            (60, 16),
            (200, 60),
        ] {
            draw(&app, w, h);
        }
    }
}

#[test]
fn help_closes_with_esc() {
    let mut app = new_app();
    app.handle_event(key(KeyCode::Char('?')));
    app.handle_event(key(KeyCode::Esc));
    let text = all_text(&draw(&app, 80, 24));
    assert!(!text.contains("to close"), "{text}");
}

// === Status bar ===

#[test]
fn status_bar_shows_help_hint_on_every_tab() {
    for tab in ActiveTab::ALL {
        let mut app = new_app();
        app.active_tab = *tab;
        let footer = row_text(&draw(&app, 80, 24), 23);
        assert!(footer.contains("?:help"), "{tab:?}: {footer:?}");
        assert!(footer.contains("DISCONNECTED"), "{tab:?}: {footer:?}");
    }
}

// === Device confirmation ===

#[test]
fn device_confirmation_rendered_and_destructive_warning_red() {
    let mut app = new_app();
    app.handle_event(key(KeyCode::Char('4'))); // Device tab
    assert_eq!(app.active_tab, ActiveTab::Device);
    app.handle_event(key(KeyCode::Down));
    app.handle_event(key(KeyCode::Down)); // serial speed
    app.handle_event(key(KeyCode::Right));
    app.handle_event(key(KeyCode::Enter));

    let buf = draw(&app, 80, 24);
    let text = all_text(&buf);
    assert!(text.contains("WARNING"), "{text}");
    assert!(text.contains("19200 baud"), "{text}");
    assert!(text.contains("any other key"), "{text}");
    // The footer keeps the question visible after the status times out.
    for _ in 0..10 {
        app.tick();
    }
    let footer = row_text(&draw(&app, 80, 24), 23);
    assert!(footer.contains("y/n"), "{footer:?}");
    let x = footer.find("Change serial").unwrap() as u16;
    assert_eq!(buf[(x, 23)].bg, Color::Red);

    app.handle_event(key(KeyCode::Char('n')));
    assert!(app.pending_device_cmds.is_empty());
    let text = all_text(&draw(&app, 80, 24));
    assert!(!text.contains("any other key"), "{text}");
}

#[test]
fn device_pending_value_marked() {
    let mut app = new_app();
    app.active_tab = ActiveTab::Device;
    let before = all_text(&draw(&app, 80, 24));
    assert!(before.contains("1 Hz"), "{before}");
    assert!(!before.contains("1 Hz*") && !before.contains("2 Hz*"));

    app.handle_event(key(KeyCode::Right)); // nav rate 2 Hz, pending
    let text = all_text(&draw(&app, 80, 24));
    assert!(text.contains("2 Hz*"), "{text}");

    app.handle_event(key(KeyCode::Enter));
    app.handle_event(key(KeyCode::Char('y')));
    let text = all_text(&draw(&app, 80, 24));
    assert!(text.contains("2 Hz \u{2713}"), "{text}");
    assert!(text.contains("Setting nav rate"), "{text}");
}

#[test]
fn device_tab_shows_all_controls_at_80x24() {
    let mut app = new_app();
    app.active_tab = ActiveTab::Device;
    let text = all_text(&draw(&app, 80, 24));
    for label in ["Nav Rate", "PPS Frequency", "QZSS", "SBAS", "[Save Config]"] {
        assert!(text.contains(label), "{label} missing\n{text}");
    }
}

// === Settings ===

#[test]
fn settings_shows_port_error_inside_dialog() {
    let mut app = new_app();
    app.handle_event(key(KeyCode::Char('s')));
    app.handle_event(key(KeyCode::Down));
    app.handle_event(key(KeyCode::Enter));
    app.handle_event(key(KeyCode::Char('z')));
    let text = all_text(&draw(&app, 80, 24));
    assert!(text.contains("Port accepts digits only"), "{text}");
    assert!(text.contains("[ Save ]"), "{text}");
    assert!(text.contains("Ctrl+S"), "{text}");
}

#[test]
fn settings_apply_persists_and_reloads_in_tempdir() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tuigps-rs/config.toml");

    let startup = config::startup(Some(path.clone()), None, None);
    assert!(
        startup
            .message
            .as_deref()
            .unwrap()
            .contains("No config file")
    );
    let mut app = App::from_startup(startup);
    app.handle_event(key(KeyCode::Char('s')));
    app.handle_event(key(KeyCode::Down));
    app.handle_event(key(KeyCode::Down)); // Units
    app.handle_event(key(KeyCode::Left)); // nautical
    app.handle_event(key_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(!app.reconnect_requested);

    let reloaded = App::from_startup(config::startup(Some(path.clone()), None, None));
    assert_eq!(reloaded.units, UnitSystem::Nautical);
    assert!(reloaded.status_message.is_empty());

    // CLI host overrides the saved host, the rest still comes from the file.
    let cli = App::from_startup(config::startup(Some(path), Some("10.1.1.1".into()), None));
    assert_eq!(cli.host, "10.1.1.1");
    assert_eq!(cli.port, 2947);
    assert_eq!(cli.units, UnitSystem::Nautical);
}

#[test]
fn corrupt_config_falls_back_with_status() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "port = \"not a number\"\n").unwrap();
    let app = App::from_startup(config::startup(Some(path), None, None));
    assert_eq!(app.port, 2947);
    assert!(
        app.status_message.contains("invalid"),
        "{}",
        app.status_message
    );
}

// === NMEA follow indicator ===

#[test]
fn nmea_indicator_following_scrolled_paused() {
    let mut app = new_app();
    app.active_tab = ActiveTab::Nmea;
    for i in 0..100 {
        app.handle_gpsd_event(GpsdEvent::Nmea(format!("$GPGGA,{i}")));
    }
    let title = row_text(&draw(&app, 80, 24), 1);
    assert!(title.contains("FOLLOWING"), "{title:?}");

    app.handle_event(key(KeyCode::PageUp));
    let buf = draw(&app, 80, 24);
    let title = row_text(&buf, 1);
    assert!(title.contains("SCROLLED +19"), "{title:?}");
    // Newest visible line is 19 back from the newest (99).
    assert!(all_text(&buf).contains("$GPGGA,80"));
    assert!(!all_text(&buf).contains("$GPGGA,81"));

    // New sentences do not move the view.
    app.handle_gpsd_event(GpsdEvent::Nmea("$GPGGA,new".into()));
    let buf = draw(&app, 80, 24);
    assert!(row_text(&buf, 1).contains("SCROLLED +20"));
    assert!(all_text(&buf).contains("$GPGGA,80"));
    assert!(!all_text(&buf).contains("$GPGGA,new"));

    app.handle_event(key(KeyCode::End));
    let buf = draw(&app, 80, 24);
    assert!(row_text(&buf, 1).contains("FOLLOWING"));
    assert!(all_text(&buf).contains("$GPGGA,new"));

    app.handle_event(key(KeyCode::Char('p')));
    assert!(row_text(&draw(&app, 80, 24), 1).contains("PAUSED"));
}

#[test]
fn nmea_empty_help_from_binding_table() {
    let mut app = new_app();
    app.active_tab = ActiveTab::Nmea;
    let text = all_text(&draw(&app, 80, 24));
    assert!(text.contains("Waiting for NMEA"), "{text}");
    for b in tab_bindings(ActiveTab::Nmea) {
        assert!(text.contains(b.desc), "{:?} missing\n{text}", b.desc);
    }
}
