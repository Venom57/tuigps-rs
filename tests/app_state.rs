//! App state driven only through public APIs (plan T8): gpsd events in via
//! `handle_tagged_event`, keys in via `handle_event`, time via
//! `update_staleness`. Assertions are on state, not status-message text.
//!
//! Nothing here executes external commands: the clock-sync test stops at
//! the queued `ClockSyncRequest` and runs it through `clock_sync::sync_clock`
//! with a recording mock runner. Logs are written only into tempdirs.

use std::path::Path;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use quick_xml::Reader;
use quick_xml::events::Event as XmlEvent;

use tuigps_rs::app::{ActiveTab, App, STALE_AFTER_SECS};
use tuigps_rs::clock_sync::{self, ClockSyncRequest, Cmd, CmdOutput, CommandRunner};
use tuigps_rs::data_model::FixMode;
use tuigps_rs::gpsd_client::{GpsdEvent, GpsdMessage, classify_line};

/// Arbitrary but realistic receipt epoch (2024-05-12T14:05:00Z).
const T0: f64 = 1_715_522_700.0;

fn new_app() -> App {
    App::new("localhost".into(), 2947)
}

fn key_kind(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers,
        kind,
        state: KeyEventState::NONE,
    })
}

fn key(c: char) -> Event {
    key_kind(KeyCode::Char(c), KeyModifiers::NONE, KeyEventKind::Press)
}

fn ctrl_c() -> Event {
    key_kind(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
        KeyEventKind::Press,
    )
}

/// Feed one gpsd message for the app's current generation.
fn send(app: &mut App, line: &str, receipt: f64) {
    let msg = GpsdMessage::parse(line, receipt).expect("valid gpsd message");
    assert!(app.handle_tagged_event((app.connection_generation, GpsdEvent::Message(msg))));
}

fn tpv_fix(time: &str, lat: f64, lon: f64) -> String {
    format!(
        r#"{{"class":"TPV","device":"/dev/ttyACM0","mode":3,"status":1,"time":"{time}","lat":{lat},"lon":{lon},"altMSL":45.5}}"#
    )
}

fn tpv_nofix(time: &str) -> String {
    format!(r#"{{"class":"TPV","device":"/dev/ttyACM0","mode":1,"time":"{time}"}}"#)
}

const PPS: &str = r#"{"class":"PPS","device":"/dev/pps0","real_sec":1715522702,"real_nsec":0,"clock_sec":1715522701,"clock_nsec":999987654,"precision":-20}"#;
const SKY: &str = r#"{"class":"SKY","device":"/dev/ttyACM0","hdop":0.9,"satellites":[{"PRN":2,"el":42.0,"az":290.0,"ss":38.0,"used":true,"gnssid":0,"svid":2}]}"#;

fn hold_count(app: &App) -> u64 {
    app.position_hold
        .as_ref()
        .and_then(|h| h.result())
        .map_or(0, |r| r.count)
}

// === Full session ===

#[test]
fn fixture_session_through_app() {
    let path = format!(
        "{}/tests/fixtures/session.jsonl",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(path).unwrap();
    let mut app = new_app();
    assert!(app.handle_tagged_event((0, GpsdEvent::Connected)));
    for (i, line) in text.lines().enumerate() {
        if let Some(event) = classify_line(line, T0 + i as f64 * 0.1) {
            assert!(app.handle_tagged_event((0, event)));
        }
    }
    let d = &app.gps_data;
    assert!(d.connected);
    assert_eq!(d.mode, FixMode::Fix3D);
    assert_eq!(d.time, "2024-05-12T14:05:02.000Z");
    assert_eq!(d.satellites.len(), 3);
    assert_eq!(d.device.path, "/dev/ttyACM0");
    assert_eq!(d.pps.precision, -20);
    // One TOFF sample per new TPV time
    assert_eq!(d.toff_samples.len(), 2);
    assert!(d.toff_samples.iter().all(|s| s.is_finite()));
    assert_eq!(app.nmea_buffer.len(), 3);
    assert!(app.nmea_buffer[0].starts_with("$GNRMC,140501.00"));
    assert!(!app.stale);
}

// === Staleness ===

#[test]
fn stale_after_threshold_without_tpv_even_with_pps() {
    let mut app = new_app();
    app.handle_tagged_event((0, GpsdEvent::Connected));

    // Nothing received yet: never stale
    app.update_staleness(T0 + 1000.0);
    assert!(!app.stale);

    send(&mut app, &tpv_fix("2024-05-12T14:05:00Z", 51.5, -0.1), T0);
    let mut first_stale = None;
    for s in 1..=15 {
        let t = T0 + s as f64;
        send(&mut app, PPS, t); // PPS keeps flowing every second
        app.update_staleness(t + 0.5);
        if app.stale && first_stale.is_none() {
            first_stale = Some(s);
            assert!((app.stale_seconds - (s as f64 + 0.5)).abs() < 1e-9);
        }
        if !app.stale {
            assert_eq!(app.stale_seconds, 0.0);
        }
    }
    assert_eq!(app.gps_data.last_seen, T0 + 15.0);
    let expected = (STALE_AFTER_SECS - 0.5).floor() as i32 + 1;
    assert_eq!(
        first_stale,
        Some(expected),
        "stale once TPV age > threshold"
    );

    // A new TPV clears staleness immediately, before the next tick
    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:16Z", 51.5, -0.1),
        T0 + 16.0,
    );
    assert!(!app.stale);
    assert_eq!(app.stale_seconds, 0.0);
    app.update_staleness(T0 + 16.0 + STALE_AFTER_SECS);
    assert!(!app.stale, "exactly at the threshold is not stale");
}

#[test]
fn stale_before_first_tpv_uses_any_message() {
    let mut app = new_app();
    app.handle_tagged_event((0, GpsdEvent::Connected));
    send(&mut app, SKY, T0);
    app.update_staleness(T0 + STALE_AFTER_SECS + 1.0);
    assert!(app.stale);
    // Reconnecting clears the flag until data arrives again
    app.begin_reconnect();
    assert!(!app.stale);
}

// === Position hold ===

#[test]
fn hold_accumulates_only_on_new_tpv_time() {
    let mut app = new_app();
    app.handle_tagged_event((0, GpsdEvent::Connected));
    app.handle_event(key('h'));
    assert!(app.position_hold.is_some());

    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:01Z", 10.0, 20.0),
        T0 + 1.0,
    );
    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:01Z", 99.0, 99.0),
        T0 + 1.2,
    ); // same epoch
    send(&mut app, SKY, T0 + 1.3);
    send(&mut app, PPS, T0 + 1.4);
    assert_eq!(hold_count(&app), 0, "result needs 2 fixes");
    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:02Z", 10.0002, 20.0002),
        T0 + 2.0,
    );
    assert_eq!(hold_count(&app), 2);
    send(&mut app, &tpv_nofix("2024-05-12T14:05:03Z"), T0 + 3.0); // no fix
    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:04Z", 10.0001, 20.0001),
        T0 + 4.0,
    );
    send(&mut app, PPS, T0 + 4.5);
    assert_eq!(hold_count(&app), 3);

    let r = app.position_hold.as_ref().unwrap().result().unwrap();
    assert!((r.mean_lat - 10.0001).abs() < 1e-9, "{}", r.mean_lat);
    assert!((r.mean_lon - 20.0001).abs() < 1e-9, "{}", r.mean_lon);

    app.handle_event(key('h'));
    assert!(app.position_hold.is_none());
}

// === Connection generations ===

#[test]
fn old_generation_events_ignored_after_reconnect() {
    let mut app = new_app();
    app.handle_tagged_event((0, GpsdEvent::Connected));
    send(&mut app, &tpv_fix("2024-05-12T14:05:01Z", 10.0, 20.0), T0);

    let generation = app.begin_reconnect();
    assert_eq!(generation, 1);
    assert!(!app.gps_data.connected);

    let old_tpv =
        GpsdMessage::parse(&tpv_fix("2024-05-12T14:05:02Z", 50.0, 60.0), T0 + 1.0).unwrap();
    let stale_events = [
        GpsdEvent::Connected,
        GpsdEvent::Message(old_tpv),
        GpsdEvent::Nmea("$GPGGA,old*00".into()),
        GpsdEvent::Error("old task error".into()),
    ];
    let error_before = app.gps_data.error_message.clone();
    for event in stale_events {
        assert!(!app.handle_tagged_event((0, event)));
    }
    assert!(!app.gps_data.connected);
    assert_eq!(app.gps_data.latitude, 10.0);
    assert_eq!(app.gps_data.time, "2024-05-12T14:05:01Z");
    assert!(app.nmea_buffer.is_empty());
    assert_eq!(app.gps_data.error_message, error_before);

    // The new generation is applied and starts from fresh data
    assert!(app.handle_tagged_event((1, GpsdEvent::Connected)));
    assert!(app.gps_data.connected);
    assert!(app.gps_data.latitude.is_nan());
    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:03Z", 1.0, 2.0),
        T0 + 3.0,
    );
    assert_eq!(app.gps_data.latitude, 1.0);

    // A future generation is not accepted either
    assert!(!app.handle_tagged_event((2, GpsdEvent::Error("x".into()))));
    assert!(app.gps_data.connected);
}

// === NMEA buffer ===

#[test]
fn nmea_buffer_is_capped() {
    let mut app = new_app();
    for i in 0..1500 {
        app.handle_tagged_event((0, GpsdEvent::Nmea(format!("$GPGGA,{i}*00"))));
    }
    assert_eq!(app.nmea_buffer.len(), 1000);
    assert_eq!(app.nmea_buffer.front().unwrap(), "$GPGGA,500*00");
    assert_eq!(app.nmea_buffer.back().unwrap(), "$GPGGA,1499*00");

    // While paused, new sentences go to the (also capped) pause buffer
    app.active_tab = ActiveTab::Nmea;
    app.handle_event(key('p'));
    assert!(app.nmea_paused);
    for i in 0..1200 {
        app.handle_tagged_event((0, GpsdEvent::Nmea(format!("$GNRMC,{i}*00"))));
    }
    assert_eq!(app.nmea_buffer.back().unwrap(), "$GPGGA,1499*00");
    assert_eq!(app.nmea_pause_buffer.len(), 1000);
    assert_eq!(app.nmea_pause_buffer.front().unwrap(), "$GNRMC,200*00");

    // Unpausing merges, keeping only the newest 1000
    app.handle_event(key('p'));
    assert!(!app.nmea_paused);
    assert!(app.nmea_pause_buffer.is_empty());
    assert_eq!(app.nmea_buffer.len(), 1000);
    assert_eq!(app.nmea_buffer.front().unwrap(), "$GNRMC,200*00");
    assert_eq!(app.nmea_buffer.back().unwrap(), "$GNRMC,1199*00");
}

// === Logging ===

/// Check the GPX is well-formed and closed; returns the number of points.
fn gpx_points(path: &Path) -> usize {
    let text = std::fs::read_to_string(path).unwrap();
    let mut reader = Reader::from_str(&text);
    let mut depth = 0usize;
    let mut points = 0;
    loop {
        match reader.read_event().expect("well-formed GPX") {
            XmlEvent::Start(e) => {
                depth += 1;
                if e.name().as_ref() == "trkpt" {
                    points += 1;
                }
            }
            XmlEvent::End(_) => depth -= 1,
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    assert_eq!(depth, 0, "unclosed elements in {}", path.display());
    assert!(text.trim_end().ends_with("</gpx>"));
    points
}

fn log_files(dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect()
}

#[test]
fn logging_start_stop_writes_valid_gpx() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = new_app();
    app.log_dir = dir.path().to_path_buf();
    app.handle_tagged_event((0, GpsdEvent::Connected));

    app.handle_event(key('l'));
    assert!(app.logger.as_ref().is_some_and(|l| l.active));
    let files = log_files(dir.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].extension().unwrap(), "gpx");

    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:01Z", 51.5, -0.1),
        T0 + 1.0,
    );
    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:01Z", 51.5, -0.1),
        T0 + 1.1,
    ); // dup
    send(&mut app, SKY, T0 + 1.2);
    send(&mut app, &tpv_nofix("2024-05-12T14:05:02Z"), T0 + 2.0);
    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:03Z", 51.6, -0.2),
        T0 + 3.0,
    );
    assert_eq!(app.logger.as_ref().unwrap().point_count, 2);

    app.handle_event(key('l'));
    assert!(app.logger.is_none());
    assert_eq!(gpx_points(&files[0]), 2);

    // Quit path: main calls stop_logging() after the loop ends
    app.handle_event(key('l'));
    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:04Z", 51.7, -0.3),
        T0 + 4.0,
    );
    app.handle_event(ctrl_c());
    assert!(app.should_quit);
    app.stop_logging();
    assert!(app.logger.is_none());
    let mut files = log_files(dir.path());
    files.sort();
    assert_eq!(files.len(), 2, "{files:?}");
    let total: usize = files.iter().map(|f| gpx_points(f)).sum();
    assert_eq!(total, 3);
}

#[test]
fn logging_into_missing_dir_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = new_app();
    app.log_dir = dir.path().join("does/not/exist");
    app.handle_event(key('l'));
    assert!(app.logger.is_none());
    assert!(!app.status_message.is_empty(), "failure is reported");
}

// === Clock sync arming ===

/// Records commands and always succeeds. Never executes anything.
#[derive(Default)]
struct RecordingRunner {
    calls: Vec<Cmd>,
}

impl CommandRunner for RecordingRunner {
    fn run(&mut self, cmd: &Cmd) -> std::io::Result<CmdOutput> {
        self.calls.push(cmd.clone());
        Ok(CmdOutput {
            success: true,
            stderr: String::new(),
        })
    }
}

#[test]
fn clock_sync_fires_only_on_fresh_tpv_with_fix() {
    let mut app = new_app();
    app.handle_tagged_event((0, GpsdEvent::Connected));
    app.active_tab = ActiveTab::Timing;
    app.handle_event(key('k'));
    assert!(app.armed_clock_set);

    // Non-TPV traffic and a no-fix TPV do not fire
    send(&mut app, PPS, T0);
    send(&mut app, SKY, T0);
    send(&mut app, &tpv_nofix("2024-05-12T14:05:01Z"), T0 + 1.0);
    assert!(app.armed_clock_set && app.pending_clock_sync.is_none());

    // A fix whose time equals the previous TPV time is not fresh
    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:01Z", 1.0, 2.0),
        T0 + 1.5,
    );
    assert!(app.armed_clock_set && app.pending_clock_sync.is_none());

    // An old-generation TPV cannot fire it either
    app.begin_reconnect();
    let old = GpsdMessage::parse(&tpv_fix("2024-05-12T14:05:02Z", 1.0, 2.0), T0 + 2.0).unwrap();
    assert!(!app.handle_tagged_event((0, GpsdEvent::Message(old))));
    assert!(app.armed_clock_set && app.pending_clock_sync.is_none());
    app.handle_tagged_event((1, GpsdEvent::Connected));

    // Fresh TPV with a fix: queued exactly once, single-shot
    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:03.250Z", 1.0, 2.0),
        T0 + 3.3,
    );
    assert!(!app.armed_clock_set);
    let expected = ClockSyncRequest {
        gps_time: "2024-05-12T14:05:03.250Z".into(),
        tpv_receipt: T0 + 3.3,
    };
    assert_eq!(app.pending_clock_sync.as_ref(), Some(&expected));
    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:04Z", 1.0, 2.0),
        T0 + 4.3,
    );
    assert_eq!(app.pending_clock_sync.as_ref(), Some(&expected));

    // Execute the queued request with the mock runner only
    let req = app.pending_clock_sync.take().unwrap();
    let mut runner = RecordingRunner::default();
    let msg = clock_sync::sync_clock(&mut runner, &req, || req.tpv_receipt + 0.5).unwrap();
    assert!(msg.contains("14:05:03.750"), "{msg}");
    assert_eq!(runner.calls.len(), 2);
    assert_eq!(runner.calls[0], clock_sync::ntp_cmd(false));
    assert_eq!(runner.calls[1].program, "timedatectl");
    assert_eq!(runner.calls[1].args[1], "set-time");

    // Disarm before a fix arrives: nothing is queued
    app.handle_event(key('k'));
    assert!(app.armed_clock_set);
    app.handle_event(key('k'));
    assert!(!app.armed_clock_set);
    send(
        &mut app,
        &tpv_fix("2024-05-12T14:05:05Z", 1.0, 2.0),
        T0 + 5.0,
    );
    assert!(app.pending_clock_sync.is_none());
}

// === Quit ===

#[test]
fn ctrl_c_quits_everywhere() {
    for &tab in ActiveTab::ALL {
        for settings in [false, true] {
            let mut app = new_app();
            app.active_tab = tab;
            app.show_settings = settings;
            app.nmea_buffer.push_back("$GPGGA,keep*00".into());
            // A key release is not a press
            app.handle_event(key_kind(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
                KeyEventKind::Release,
            ));
            assert!(!app.should_quit, "{tab:?} settings={settings}");
            app.handle_event(ctrl_c());
            assert!(app.should_quit, "{tab:?} settings={settings}");
            assert_eq!(app.nmea_buffer.len(), 1, "Ctrl+C must not act as 'c'");
        }
    }
}
