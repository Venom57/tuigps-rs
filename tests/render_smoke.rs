//! Render smoke tests: every tab, with and without the settings overlay, at
//! a range of terminal sizes, with empty and fully populated GPS data. The
//! main goal is "never panics"; a few content assertions at 80x24 guard the
//! most important UX behaviors.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use tuigps_rs::app::{ActiveTab, App};
use tuigps_rs::data_model::{FixMode, FixStatus, SatelliteInfo};
use tuigps_rs::position_hold::PositionHold;
use tuigps_rs::ui;

const SIZES: &[(u16, u16)] = &[
    (0, 0),
    (1, 1),
    (10, 3),
    (20, 5),
    (40, 12),
    (59, 15),
    (60, 16),
    (80, 24),
    (120, 40),
    (200, 60),
];

fn new_app() -> App {
    App::new("localhost".into(), 2947)
}

/// 64 satellites across all constellations, including edge cases: SNR above
/// the chart maximum, elevation 0 and 90, negative elevation, NaN fields,
/// every health value.
fn satellites() -> Vec<SatelliteInfo> {
    (0..64u16)
        .map(|i| {
            let gnssid = (i % 9) as u8; // 8 is an unknown constellation
            let elevation = match i % 7 {
                0 => 0.0,
                1 => 90.0,
                2 => -5.0,
                3 => f64::NAN,
                _ => (i as f64 * 7.0) % 90.0,
            };
            SatelliteInfo {
                prn: i + 1,
                gnssid,
                svid: i * 3 + 1,
                elevation,
                azimuth: if i % 11 == 0 {
                    f64::NAN
                } else {
                    (i as f64 * 37.0) % 360.0
                },
                snr: match i % 5 {
                    0 => 60.0,
                    1 => f64::NAN,
                    2 => 0.0,
                    _ => 15.0 + i as f64 % 40.0,
                },
                used: i % 3 == 0,
                sigid: (i % 4) as u8,
                health: (i % 4) as u8,
                freqid: None,
            }
        })
        .collect()
}

fn populated_app() -> App {
    let mut app = new_app();
    let d = &mut app.gps_data;
    d.connected = true;
    d.last_seen = 1.0;
    d.latitude = 51.507_4;
    d.longitude = -0.127_8;
    d.alt_hae = 45.2;
    d.alt_msl = f64::NAN; // deliberately missing
    d.geoid_sep = 47.0;
    d.speed = 1.5;
    d.track = 271.0;
    d.climb = f64::NAN;
    d.time = "2026-10-17T12:34:56.000Z".into();
    d.leapseconds = 18;
    d.mode = FixMode::Fix3D;
    d.status = FixStatus::Dgps;
    d.magtrack = 270.0;
    d.magvar = f64::NAN;
    d.dop.hdop = 0.9;
    d.dop.vdop = 1.4;
    d.dop.pdop = 1.7;
    d.dop.gdop = 2.1;
    d.dop.tdop = 0.8;
    d.errors.eph = 2.5;
    d.errors.epv = f64::NAN;
    d.errors.ept = 0.005;
    d.errors.sep = 3.0;
    d.pps.real_sec = 1_700_000_000;
    d.pps.real_nsec = 0;
    d.pps.clock_sec = 1_700_000_000;
    d.pps.clock_nsec = 12_345;
    d.toff.real_sec = 1_700_000_000;
    d.toff.clock_sec = 1_700_000_000;
    d.toff.clock_nsec = 98_765;
    d.device.path = "/dev/ttyACM0-with-a-really-long-device-path-name".into();
    d.device.driver = "u-blox".into();
    d.device.subtype = "SW ROM CORE 3.01 (107888),HW 00080000".into();
    d.device.bps = 115_200;
    d.device.cycle = 1.0;
    d.version.release = "3.25".into();
    app.timing.toff_samples = vec![1e-4, -2e-4, 3e-5, f64::NAN];
    app.timing.toff_armed_offset = 1.5e-4;
    app.timing.toff_armed_gps_time = "2026-10-17T12:34:56Z".into();
    d.satellites = satellites();
    d.satellites_used = 22;

    for i in 0..1000 {
        let s = match i % 6 {
            0 => format!("$GPGGA,{i},4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47"),
            1 => "$PUBX,00,081350.00,4717.113210,N,00833.915187,E,546.589,G3,2.1,2.0".into(),
            2 => "$GN\u{00e9}\u{1f600}".into(),
            3 => "$".into(),
            4 => "x".repeat(300),
            _ => "$GPGSV,3,1,11,03,03,111,00,04,15,270,00,06,01,010,00,13,06,292,00*74".into(),
        };
        app.nmea_buffer.push_back(s);
    }
    app.nmea_scroll_offset = 5000; // past the end; must clamp
    app.sat_scroll_offset = 5000;

    let mut hold = PositionHold::new();
    hold.add_fix(51.5074, -0.1278, 45.0);
    hold.add_fix(51.50741, -0.12781, f64::NAN);
    app.position_hold = Some(hold);
    app.status_message = "Status message that is fairly long to force truncation".into();
    app
}

fn draw(app: &App, w: u16, h: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| ui::render(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
}

fn all_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_matrix(app: &mut App) {
    for &tab in ActiveTab::ALL {
        app.active_tab = tab;
        for settings in [false, true] {
            app.show_settings = settings;
            for &(w, h) in SIZES {
                draw(app, w, h);
            }
        }
    }
}

#[test]
fn smoke_default_data() {
    render_matrix(&mut new_app());
}

#[test]
fn smoke_populated_data() {
    render_matrix(&mut populated_app());
}

#[test]
fn smoke_populated_stale_and_filtered() {
    let mut app = populated_app();
    app.stale = true;
    app.stale_seconds = 12.0;
    app.nmea_filter = "GGA".into();
    app.nmea_paused = true;
    app.settings_editing = true;
    render_matrix(&mut app);

    app.nmea_filter = "PUBX".into();
    app.nmea_scroll_offset = 3;
    app.gps_data.connected = false;
    app.gps_data.error_message = "Connection refused (os error 111) and more".repeat(3);
    render_matrix(&mut app);
}

/// Individual panels at tiny/odd rects, bypassing the "too small" guard.
#[test]
fn smoke_panels_direct() {
    let app = populated_app();
    let empty = new_app();
    let rects = [
        Rect::new(0, 0, 0, 0),
        Rect::new(0, 0, 1, 1),
        Rect::new(0, 0, 2, 2),
        Rect::new(0, 0, 10, 3),
        Rect::new(0, 0, 9, 5),
        Rect::new(0, 0, 21, 11),
        Rect::new(0, 0, 40, 12),
        Rect::new(3, 2, 37, 10),
    ];
    for a in [&app, &empty] {
        for &r in &rects {
            let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
            terminal
                .draw(|f| {
                    let r = r.intersection(f.area());
                    ui::position::render(f, r, a);
                    ui::fix::render(f, r, a);
                    ui::velocity::render(f, r, a);
                    ui::sky_plot::render(f, r, a);
                    ui::signal_chart::render(f, r, a);
                    ui::error_panel::render(f, r, a);
                    ui::device_panel::render(f, r, a);
                    ui::time_panel::render(f, r, a, true);
                    ui::time_panel::render(f, r, a, false);
                    ui::constellation::render(f, r, a);
                    ui::satellite_table::render(f, r, a);
                    ui::nmea_viewer::render(f, r, a);
                    ui::device_config::render(f, r, a);
                    ui::status_bar::render_status_bar(f, r, a);
                    ui::settings::render_settings(f, r, a);
                })
                .unwrap();
        }
    }
}

// ---------------------------------------------------------------------------
// Targeted content assertions at 80x24
// ---------------------------------------------------------------------------

#[test]
fn connection_badge_always_visible_in_footer() {
    let mut app = populated_app();
    for &tab in ActiveTab::ALL {
        app.active_tab = tab;
        let buf = draw(&app, 80, 24);
        let footer = row_text(&buf, 23);
        assert!(footer.contains("CONNECTED"), "{tab:?}: {footer:?}");
        assert!(footer.contains("HOLD"), "{tab:?}: {footer:?}");
    }

    app.gps_data.connected = false;
    app.active_tab = ActiveTab::Device;
    let footer = row_text(&draw(&app, 80, 24), 23);
    assert!(footer.contains("DISCONNECTED"), "{footer:?}");

    app.gps_data.connected = true;
    app.stale = true;
    app.stale_seconds = 7.0;
    let buf = draw(&app, 80, 24);
    let footer = row_text(&buf, 23);
    assert!(footer.contains("STALE (7s)"), "{footer:?}");
    // STALE badge is black on yellow for legibility.
    let x = footer.find("STALE").unwrap() as u16;
    assert_eq!(buf[(x, 23)].fg, Color::Black);
    assert_eq!(buf[(x, 23)].bg, Color::Yellow);
}

#[test]
fn hints_shown_when_no_status_message() {
    let mut app = populated_app();
    app.status_message.clear();
    app.active_tab = ActiveTab::Nmea;
    let footer = row_text(&draw(&app, 80, 24), 23);
    assert!(footer.contains("p:pause"), "{footer:?}");
    assert!(footer.contains("CONNECTED"), "{footer:?}");
}

#[test]
fn panels_marked_when_disconnected() {
    let app = new_app(); // default: not connected
    let buf = draw(&app, 80, 24);
    let text = all_text(&buf);
    assert!(text.contains("NO DATA"), "{text}");
    // Panel contents are greyed out.
    let lat_y = (0..24)
        .find(|&y| row_text(&buf, y).contains("Lat:"))
        .unwrap();
    let x = row_text(&buf, lat_y).find("Lat:").unwrap() as u16;
    assert_eq!(buf[(x, lat_y)].fg, Color::DarkGray);
}

#[test]
fn panels_marked_when_stale_and_clean_when_live() {
    let mut app = populated_app();
    let live = all_text(&draw(&app, 80, 24));
    assert!(
        !live.contains("NO DATA") && !live.contains(" STALE "),
        "{live}"
    );

    app.stale = true;
    let stale = draw(&app, 80, 24);
    // Marker on a panel border (row 1 is the top border of the first row).
    assert!(row_text(&stale, 1).contains(" STALE "));
}

#[test]
fn fix_panel_shows_all_dops_at_80x24() {
    let mut app = populated_app();
    app.position_hold = None;
    let text = all_text(&draw(&app, 80, 24));
    for dop in ["HDOP", "VDOP", "PDOP", "GDOP"] {
        assert!(text.contains(dop), "{dop} missing:\n{text}");
    }
}

#[test]
fn too_small_notice() {
    let app = populated_app();
    let text = all_text(&draw(&app, 40, 12));
    assert!(text.contains("Terminal too small"), "{text}");
    assert!(text.contains("need 60x16"), "{text}");
}

#[test]
fn timing_tab_status_message_once_and_pps_placeholder() {
    let mut app = populated_app();
    app.active_tab = ActiveTab::Timing;
    app.status_message = "UNIQUE-STATUS".into();
    let text = all_text(&draw(&app, 120, 40));
    assert_eq!(text.matches("UNIQUE-STATUS").count(), 1, "{text}");
    assert!(text.contains("Leap: 18"));

    let mut empty = new_app();
    empty.active_tab = ActiveTab::Timing;
    let text = all_text(&draw(&empty, 120, 40));
    assert!(text.contains("PPS Offset: ---"), "{text}");
    assert!(text.contains("Leap: ---"), "{text}");
    assert!(text.contains("Cycle:   ---"), "{text}");
    assert!(!text.contains("NaN"), "{text}");
}

#[test]
fn sky_plot_labels_have_constellation_letter() {
    let mut app = populated_app();
    app.gps_data.satellites = vec![SatelliteInfo {
        prn: 12,
        gnssid: 2,
        svid: 12,
        elevation: 45.0,
        azimuth: 90.0,
        snr: 40.0,
        used: true,
        sigid: 0,
        health: 1,
        freqid: None,
    }];
    let text = all_text(&draw(&app, 120, 40));
    assert!(text.contains("#E12"), "{text}");
    assert!(text.contains("E GAL"), "legend missing:\n{text}");
}

#[test]
fn satellite_table_uses_all_rows() {
    let mut app = populated_app();
    app.active_tab = ActiveTab::Satellites;
    app.sat_scroll_offset = 0;
    let buf = draw(&app, 80, 24);
    // Rows: tab bar, summary, table (top border, header, N rows, bottom
    // border), status bar. The last data row sits just above the border.
    let table_h = 24 - 3;
    let rows = table_h - 3;
    let title = row_text(&buf, 2);
    assert!(title.contains(&format!("[1-{rows}/64]")), "{title:?}");
    // First row is GPS PRN 1 with health 0 (unknown): neutral, not red.
    let health_x = row_text(&buf, 3).find("Health").unwrap() as u16;
    let first_row = row_text(&buf, 4);
    assert!(first_row.contains("GPS"), "{first_row:?}");
    assert_eq!(buf[(health_x, 4)].symbol(), "-", "{first_row:?}");
    assert_eq!(buf[(health_x, 4)].fg, Color::DarkGray);
}

#[test]
fn settings_text_is_not_on_dark_gray() {
    let mut app = new_app();
    app.show_settings = true;
    let buf = draw(&app, 80, 24);
    for cell in buf.content() {
        if cell.symbol() != " " && cell.fg == Color::DarkGray {
            // Only the divider line may use DarkGray.
            assert_eq!(cell.symbol(), "\u{2500}");
        }
        assert_ne!(cell.bg, Color::DarkGray);
    }
}

#[test]
fn strip_colors_removes_all_color() {
    let app = populated_app();
    let mut buf = draw(&app, 80, 24);
    ui::strip_colors(&mut buf);
    assert!(
        buf.content()
            .iter()
            .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
    );
    // Badges survive as reverse video.
    let footer_y = 23;
    let x = row_text(&buf, footer_y).find("CONNECTED").unwrap() as u16;
    assert!(buf[(x, footer_y)].modifier.contains(Modifier::REVERSED));
}
