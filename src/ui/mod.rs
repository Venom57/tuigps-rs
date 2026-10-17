use std::sync::OnceLock;

use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{ActiveTab, App};
use crate::data_model::SatelliteInfo;

pub mod constellation;
pub mod dashboard;
pub mod device_config;
pub mod device_panel;
pub mod error_panel;
pub mod fix;
pub mod nmea_viewer;
pub mod position;
pub mod satellite_table;
pub mod settings;
pub mod signal_chart;
pub mod sky_plot;
pub mod status_bar;
pub mod time_panel;
pub mod velocity;

/// Below this size the layout cannot show anything useful; a notice is
/// rendered instead of garbled panels.
pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 16;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(f, area);
    } else {
        render_main(f, app);
    }

    if no_color() {
        strip_colors(f.buffer_mut());
    }
}

fn render_main(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // tab bar
        Constraint::Min(0),    // main content
        Constraint::Length(1), // status bar
    ])
    .split(f.area());

    render_tab_bar(f, chunks[0], app);

    match app.active_tab {
        ActiveTab::Dashboard => dashboard::render_dashboard(f, chunks[1], app),
        ActiveTab::Satellites => render_satellites(f, chunks[1], app),
        ActiveTab::Timing => render_timing(f, chunks[1], app),
        ActiveTab::Device => device_config::render(f, chunks[1], app),
        ActiveTab::Nmea => nmea_viewer::render(f, chunks[1], app),
    }

    status_bar::render_status_bar(f, chunks[2], app);

    if app.show_settings {
        settings::render_settings(f, f.area(), app);
    }
}

fn render_too_small(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::styled("Terminal too small", Style::default().bold()),
        Line::raw(format!("need {}x{}", MIN_WIDTH, MIN_HEIGHT)),
        Line::styled(
            format!("have {}x{}", area.width, area.height),
            Style::default().fg(Color::DarkGray),
        ),
    ];
    let top = area.y + area.height.saturating_sub(lines.len() as u16) / 2;
    let msg_area = Rect::new(
        area.x,
        top,
        area.width,
        (lines.len() as u16).min(area.height),
    );
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), msg_area);
}

// ---------------------------------------------------------------------------
// Data freshness (stale / disconnected)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataState {
    Live,
    Stale,
    NoData,
}

pub fn data_state(app: &App) -> DataState {
    if !app.gps_data.connected {
        DataState::NoData
    } else if app.stale {
        DataState::Stale
    } else {
        DataState::Live
    }
}

/// Call after rendering a data panel: when data is not live, grey out the
/// whole panel and stamp a "NO DATA"/"STALE" marker on its top border so
/// old values are not mistaken for current ones.
pub fn mark_data_state(f: &mut Frame, area: Rect, app: &App) {
    let (label, style) = match data_state(app) {
        DataState::Live => return,
        DataState::Stale => (
            " STALE ",
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ),
        DataState::NoData => (
            " NO DATA ",
            Style::default().fg(Color::White).bg(Color::Red),
        ),
    };

    let buf = f.buffer_mut();
    dim_area(buf, area);

    // Leave room for the panel's own title on the left (longest is ~10 cols).
    let w = label.len() as u16;
    if area.height > 0 && area.width >= w + 13 {
        buf.set_string(area.right() - w - 1, area.y, label, style.bold());
    }
}

/// Recolor every cell in `area` to DarkGray and drop bold, keeping symbols
/// and backgrounds.
pub fn dim_area(buf: &mut Buffer, area: Rect) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_fg(Color::DarkGray);
                cell.modifier.remove(Modifier::BOLD);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// NO_COLOR support (https://no-color.org)
// ---------------------------------------------------------------------------

/// True when the `NO_COLOR` environment variable is set to a non-empty
/// value. Evaluated once.
pub fn no_color() -> bool {
    static NO_COLOR: OnceLock<bool> = OnceLock::new();
    *NO_COLOR.get_or_init(|| std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()))
}

/// Remove all colors from a rendered buffer while keeping text modifiers.
/// Cells that relied on a background color (badges, key hints) become
/// reverse video and DarkGray "secondary" text becomes DIM so the visual
/// hierarchy survives without color.
pub fn strip_colors(buf: &mut Buffer) {
    for cell in buf.content.iter_mut() {
        if cell.bg != Color::Reset {
            cell.modifier.insert(Modifier::REVERSED);
        }
        if cell.fg == Color::DarkGray {
            cell.modifier.insert(Modifier::DIM);
        }
        cell.fg = Color::Reset;
        cell.bg = Color::Reset;
        cell.underline_color = Color::Reset;
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// (visible, used) satellite counts indexed by gnssid. Allocation-free and
/// naturally ordered by gnssid, unlike `GPSData::constellation_counts`.
pub(crate) fn constellation_counts(sats: &[SatelliteInfo]) -> [(u32, u32); 256] {
    let mut counts = [(0u32, 0u32); 256];
    for sat in sats {
        let entry = &mut counts[sat.gnssid as usize];
        entry.0 += 1;
        if sat.used {
            entry.1 += 1;
        }
    }
    counts
}

fn render_tab_bar(f: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = ActiveTab::ALL
        .iter()
        .map(|t| {
            let style = if *t == app.active_tab {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::styled(t.title(), style)
        })
        .collect();

    let tabs = Tabs::new(titles)
        .select(
            ActiveTab::ALL
                .iter()
                .position(|t| *t == app.active_tab)
                .unwrap_or(0),
        )
        .divider("|");

    f.render_widget(tabs, area);
}

fn render_satellites(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // constellation summary (compact bar)
        Constraint::Min(0),    // satellite table
    ])
    .split(area);

    constellation::render(f, chunks[0], app);
    satellite_table::render(f, chunks[1], app);
}

fn render_timing(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3), // TOFF controls
        Constraint::Length(8), // device panel
    ])
    .split(area);

    time_panel::render(f, chunks[0], app, true);
    render_toff_controls(f, chunks[1], app);
    device_panel::render(f, chunks[2], app);
}

fn render_toff_controls(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered();
    let inner = block.inner(area);
    f.render_widget(block, area);

    use std::sync::atomic::Ordering;
    let is_armed = app.armed_toff.load(Ordering::Relaxed);

    let key_style = Style::default().fg(Color::White).bg(Color::DarkGray);

    // The status message is shown once, in the status bar.
    let spans = vec![
        Span::styled(" a ", key_style),
        if is_armed {
            Span::styled(
                " ARMED ",
                Style::default().fg(Color::Black).bg(Color::Yellow),
            )
        } else {
            Span::raw(" arm TOFF  ")
        },
        Span::styled(" c ", key_style),
        Span::raw(" clear TOFF  "),
        Span::styled(" k ", key_style),
        if app.armed_clock_set {
            Span::styled(" ARMED ", Style::default().fg(Color::White).bg(Color::Red))
        } else {
            Span::raw(" clock sync  ")
        },
    ];

    f.render_widget(Paragraph::new(vec![Line::from(spans)]), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_colors_keeps_modifiers_and_reverses_badges() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        buf[(0, 0)].set_fg(Color::Green).modifier = Modifier::BOLD;
        buf[(1, 0)].set_fg(Color::Black).set_bg(Color::Yellow);
        buf[(2, 0)].set_fg(Color::DarkGray);
        strip_colors(&mut buf);
        for x in 0..3 {
            assert_eq!(buf[(x, 0)].fg, Color::Reset);
            assert_eq!(buf[(x, 0)].bg, Color::Reset);
        }
        assert!(buf[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(buf[(1, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(2, 0)].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn dim_area_clips_to_buffer() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 2));
        buf[(1, 1)].set_fg(Color::Green).modifier = Modifier::BOLD;
        dim_area(&mut buf, Rect::new(1, 1, 50, 50));
        assert_eq!(buf[(1, 1)].fg, Color::DarkGray);
        assert!(!buf[(1, 1)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(0, 0)].fg, Color::Reset);
    }

    #[test]
    fn constellation_counts_by_gnssid() {
        let sat = |gnssid, used| SatelliteInfo {
            prn: 1,
            gnssid,
            svid: 1,
            elevation: 45.0,
            azimuth: 90.0,
            snr: 30.0,
            used,
            sigid: 0,
            health: 1,
            freqid: None,
        };
        let sats = vec![sat(0, true), sat(0, false), sat(6, true)];
        let c = constellation_counts(&sats);
        assert_eq!(c[0], (2, 1));
        assert_eq!(c[6], (1, 1));
        assert_eq!(c[2], (0, 0));
    }
}
