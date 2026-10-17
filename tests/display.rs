//! Dashboard display tests: units, fix age, position hold and layout at
//! common terminal sizes (80x24, 100x30, 120x40).

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use tuigps_rs::app::{App, CoordFormat, UnitSystem};
use tuigps_rs::data_model::{FixMode, FixStatus, SatelliteInfo};
use tuigps_rs::position_hold::PositionHold;
use tuigps_rs::ui;
use tuigps_rs::ui::position::unix_now;

const SIZES: &[(u16, u16)] = &[(80, 24), (100, 30), (120, 40)];
const UNITS: &[UnitSystem] = &[
    UnitSystem::Metric,
    UnitSystem::Imperial,
    UnitSystem::Nautical,
];

fn satellites() -> Vec<SatelliteInfo> {
    (0..20u16)
        .map(|i| SatelliteInfo {
            prn: i + 1,
            gnssid: (i % 4) as u8,
            svid: i + 1,
            elevation: 10.0 + (i as f64 * 4.0),
            azimuth: i as f64 * 18.0,
            snr: 20.0 + i as f64,
            used: i % 2 == 0,
            sigid: 0,
            health: 1,
            freqid: None,
        })
        .collect()
}

fn populated_app(units: UnitSystem) -> App {
    let mut app = App::new("localhost".into(), 2947);
    app.units = units;
    let now = unix_now();
    let d = &mut app.gps_data;
    d.connected = true;
    d.last_seen = now;
    d.last_tpv = now - 0.42;
    d.latitude = 51.507_4;
    d.longitude = -0.127_8;
    d.alt_hae = 100.0;
    d.alt_msl = 53.0;
    d.geoid_sep = 47.0;
    d.speed = 10.0;
    d.track = 90.0;
    d.climb = 1.0;
    d.time = "2026-10-17T12:34:56.000Z".into();
    d.leapseconds = 18;
    d.mode = FixMode::Fix3D;
    d.status = FixStatus::Gps;
    d.dop.hdop = 0.9;
    d.dop.vdop = 1.4;
    d.dop.pdop = 1.7;
    d.dop.gdop = 2.1;
    d.errors.eph = 2.5;
    d.errors.epv = 4.0;
    d.errors.eps = 0.5;
    d.errors.ept = 0.005;
    d.errors.sep = 5.0;
    d.errors.epd = 3.0;
    d.errors.epc = 0.25;
    d.errors.epx = 1.5;
    d.errors.epy = 2.0;
    d.device.path = "/dev/ttyACM0".into();
    d.device.driver = "u-blox".into();
    d.device.subtype = "SW 3.01".into();
    d.device.bps = 115_200;
    d.device.cycle = 1.0;
    d.version.release = "3.25".into();
    d.satellites = satellites();
    d.satellites_used = 10;
    app
}

/// Hold with a spread of a few decimeters around a known mean.
fn with_hold(mut app: App) -> App {
    let mut hold = PositionHold::new();
    for i in 0..100 {
        let k = i as f64;
        let dlat = (k * 0.7).sin() * 3e-6;
        let dlon = (k * 1.3).cos() * 4e-6;
        hold.add_fix(51.5074 + dlat, -0.1278 + dlon, 53.0 + (k * 0.5).sin());
    }
    app.position_hold = Some(hold);
    app
}

fn draw(app: &App, w: u16, h: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| ui::render(f, app)).unwrap();
    terminal.backend().buffer().clone()
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

/// Screen position of the first cell of `needle`.
fn find(buf: &Buffer, needle: &str) -> (u16, u16) {
    for y in 0..buf.area.height {
        let row = row_text(buf, y);
        if let Some(byte) = row.find(needle) {
            return (row[..byte].chars().count() as u16, y);
        }
    }
    panic!("{needle:?} not on screen:\n{}", all_text(buf));
}

/// Foreground color of the first cell of `needle`.
fn color_of(buf: &Buffer, needle: &str) -> Color {
    let (x, y) = find(buf, needle);
    buf[(x, y)].fg
}

/// Foreground color of the cell right after `needle`.
fn color_after(buf: &Buffer, needle: &str) -> Color {
    let (x, y) = find(buf, needle);
    buf[(x + needle.chars().count() as u16, y)].fg
}

#[test]
fn dashboard_renders_all_sizes_units_and_hold_states() {
    for &units in UNITS {
        for hold in [false, true] {
            let app = populated_app(units);
            let app = if hold { with_hold(app) } else { app };
            for &(w, h) in SIZES {
                let text = all_text(&draw(&app, w, h));
                assert!(text.contains("Lat:"), "{units:?} {w}x{h}\n{text}");
                assert!(text.contains("Age:"), "{units:?} {w}x{h}\n{text}");
                assert!(text.contains("EPH:"), "{units:?} {w}x{h}\n{text}");
                assert!(!text.contains("NaN"), "{units:?} {w}x{h}\n{text}");
                assert!(!text.contains("\u{00b1}---"), "{w}x{h}\n{text}");
                for dop in ["HDOP", "VDOP", "PDOP", "GDOP"] {
                    assert!(text.contains(dop), "{dop} {units:?} {w}x{h}\n{text}");
                }
                if hold {
                    assert!(text.contains(" Hold "), "{w}x{h}\n{text}");
                    assert!(text.contains("Fixes: 100"), "{w}x{h}\n{text}");
                    assert!(text.contains("CEP50:"), "{w}x{h}\n{text}");
                    assert!(text.contains("CEP95:"), "{w}x{h}\n{text}");
                    assert!(!text.contains(" Velocity "), "{w}x{h}\n{text}");
                } else {
                    assert!(text.contains("Speed:"), "{w}x{h}\n{text}");
                    assert!(text.contains("Climb:"), "{w}x{h}\n{text}");
                }
            }
        }
    }
}

#[test]
fn metric_units() {
    let app = populated_app(UnitSystem::Metric);
    let text = all_text(&draw(&app, 120, 40));
    for s in [
        "Alt HAE: 100.0 m",
        "Alt MSL: 53.0 m",
        "Geoid:   47.0 m",
        "Speed: 36.0 km/h",
        "Climb: 1.00 m/s",
        "EPH:  \u{00b1}2.50 m",
        "EPV:  \u{00b1}4.00 m",
        "EPS:  \u{00b1}1.80 km/h",
        "EPT:  \u{00b1}0.005000 s",
        "SEP:  \u{00b1}5.00 m",
        "EPD:  \u{00b1}3.0°",
        "EPC:  \u{00b1}0.25 m/s",
        "EPX:  \u{00b1}1.50 m",
        "EPY:  \u{00b1}2.00 m",
    ] {
        assert!(text.contains(s), "{s:?} missing:\n{text}");
    }
    assert!(!text.contains(" ft"), "{text}");
}

#[test]
fn imperial_units() {
    let app = populated_app(UnitSystem::Imperial);
    let text = all_text(&draw(&app, 120, 40));
    for s in [
        "Alt HAE: 328.1 ft",
        "Alt MSL: 173.9 ft",
        "Geoid:   154.2 ft",
        "Speed: 22.4 mph",
        "Climb: 197 ft/min",
        "EPH:  \u{00b1}8.20 ft",
        "EPS:  \u{00b1}1.12 mph",
        "SEP:  \u{00b1}16.40 ft",
        "EPC:  \u{00b1}49 ft/min",
        "EPY:  \u{00b1}6.56 ft",
    ] {
        assert!(text.contains(s), "{s:?} missing:\n{text}");
    }
    assert!(!text.contains(" m/s"), "{text}");
    assert!(!text.contains(" km/h"), "{text}");
}

#[test]
fn nautical_units() {
    let app = populated_app(UnitSystem::Nautical);
    let text = all_text(&draw(&app, 120, 40));
    for s in [
        "Alt HAE: 328.1 ft",
        "Speed: 19.4 kn",
        "Climb: 197 ft/min",
        "EPH:  \u{00b1}8.20 ft",
        "EPS:  \u{00b1}0.97 kn",
    ] {
        assert!(text.contains(s), "{s:?} missing:\n{text}");
    }
}

#[test]
fn missing_errors_show_plain_placeholder() {
    let mut app = populated_app(UnitSystem::Imperial);
    app.gps_data.errors = Default::default();
    let text = all_text(&draw(&app, 120, 40));
    assert!(text.contains("EPH:  ---"), "{text}");
    assert!(text.contains("EPT:  ---"), "{text}");
    assert!(!text.contains("\u{00b1}"), "{text}");
}

#[test]
fn hold_view_units_and_coord_format() {
    for (units, label) in [
        (UnitSystem::Metric, " m"),
        (UnitSystem::Imperial, " ft"),
        (UnitSystem::Nautical, " ft"),
    ] {
        let mut app = with_hold(populated_app(units));
        app.coord_format = CoordFormat::DMS;
        let buf = draw(&app, 80, 24);
        let text = all_text(&buf);
        let line = |prefix: &str| {
            text.lines()
                .find_map(|l| l.find(prefix).map(|i| l[i..].to_string()))
                .unwrap_or_else(|| panic!("{prefix} missing:\n{text}"))
        };
        assert!(line("CEP50: ").contains(label), "{units:?}\n{text}");
        assert!(line("CEP95: ").contains(label), "{units:?}\n{text}");
        assert!(line("\u{03c3}N/E: ").contains(label), "{units:?}\n{text}");
        assert!(line("Alt MSL: ").contains(label), "{units:?}\n{text}");
        // Mean position in DMS (both live Position and Hold use it).
        assert!(text.matches("51° 30' 26.").count() >= 2, "{text}");
        assert!(text.contains("Fixes: 100  00:00"), "{text}");
        // Sub-meter spread is green (thresholds are in meters whatever the
        // display unit).
        assert_eq!(color_after(&buf, "CEP50: "), Color::Green);
        assert_eq!(color_after(&buf, "CEP95: "), Color::Green);
    }
}

#[test]
fn hold_collecting_before_two_fixes() {
    let mut app = populated_app(UnitSystem::Metric);
    let mut hold = PositionHold::new();
    hold.add_fix(51.5, -0.1, 10.0);
    app.position_hold = Some(hold);
    let text = all_text(&draw(&app, 80, 24));
    assert!(text.contains("Collecting fixes"), "{text}");
}

#[test]
fn fix_age_shown_and_colored() {
    let app = populated_app(UnitSystem::Metric);
    let buf = draw(&app, 80, 24);
    let text = all_text(&buf);
    assert!(text.contains("Age:     0.4s"), "{text}");
    assert_eq!(color_of(&buf, "0.4s"), Color::Green);

    // Deterministic "now" through the panel's injectable entry point.
    let mut app = populated_app(UnitSystem::Metric);
    app.gps_data.last_tpv = 1000.0;
    for (now, text, color) in [
        (1000.4, "0.4s", Color::Green),
        (1005.0, "5.0s", Color::Yellow),
        (1030.0, "30s", Color::Red),
        (1303.0, "5m03s", Color::Red),
    ] {
        let mut terminal = Terminal::new(TestBackend::new(30, 10)).unwrap();
        terminal
            .draw(|f| ui::position::render_at(f, Rect::new(0, 0, 30, 10), &app, now))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        assert!(all_text(&buf).contains(&format!("Age:     {text}")));
        assert_eq!(color_of(&buf, text), color, "{text}");
    }

    // Before the first TPV.
    let mut app = populated_app(UnitSystem::Metric);
    app.gps_data.last_tpv = 0.0;
    let text = all_text(&draw(&app, 80, 24));
    assert!(text.contains("Age:     ---"), "{text}");
}

#[test]
fn short_bottom_row_marks_truncation() {
    let app = populated_app(UnitSystem::Metric);
    let text = all_text(&draw(&app, 80, 24));
    // Errors shows its most important lines and flags the rest as cut.
    assert!(text.contains(" Errors \u{2026} "), "{text}");
    assert!(text.contains(" Device \u{2026} "), "{text}");
    assert!(text.contains(" Time \u{2026} "), "{text}");
    for s in [
        "EPH:  \u{00b1}2.50 m",
        "EPV:",
        "EPS:",
        "Path:",
        "Driver:",
        "Baud:",
        "Date:",
        "Time:",
        "EPT:  \u{00b1}0.005000 s",
    ] {
        assert!(text.contains(s), "{s} missing:\n{text}");
    }
    assert!(!text.contains("EPY:"), "{text}");
    // The top row is never truncated at 80x24.
    for title in [" Position ", " Fix ", " Velocity "] {
        assert!(text.contains(title), "{title}\n{text}");
    }
    let held = all_text(&draw(&with_hold(populated_app(UnitSystem::Metric)), 80, 24));
    assert!(held.contains(" Hold "), "{held}");
    assert!(held.contains("\u{03c3}N/E:"), "{held}");

    let text = all_text(&draw(&app, 120, 40));
    assert!(!text.contains('\u{2026}'), "{text}");
    assert!(text.contains("EPY:"), "{text}");
    assert!(text.contains("gpsd:    3.25"), "{text}");
}

#[test]
#[ignore = "prints screens for manual inspection"]
fn dump_screens() {
    for &(w, h) in SIZES {
        for app in [
            populated_app(UnitSystem::Metric),
            with_hold(populated_app(UnitSystem::Imperial)),
        ] {
            println!("{}\n", all_text(&draw(&app, w, h)));
        }
    }
}
