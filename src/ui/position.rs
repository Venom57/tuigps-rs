use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::{cep_color, fix_age_color};
use crate::formatting::{
    fix_age, fmt_age, fmt_altitude, fmt_coord, fmt_distance, fmt_distance_pair, fmt_duration,
    panel_title,
};

/// Current time as unix seconds (0.0 if the clock is before 1970).
pub fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    render_at(f, area, app, unix_now());
}

/// [`render`] with an explicit "now" (unix seconds) for the fix age, so
/// tests are deterministic.
pub fn render_at(f: &mut Frame, area: Rect, app: &App, now: f64) {
    let data = &app.gps_data;
    let cf = app.coord_format.as_str();
    let unit = app.units.as_str();
    let age = fix_age(data.last_tpv, now);

    let lines = vec![
        Line::from(vec![
            Span::raw("Lat: "),
            Span::styled(fmt_coord(data.latitude, "lat", cf), Style::default().bold()),
        ]),
        Line::from(vec![
            Span::raw("Lon: "),
            Span::styled(
                fmt_coord(data.longitude, "lon", cf),
                Style::default().bold(),
            ),
        ]),
        Line::from(format!("Alt HAE: {}", fmt_altitude(data.alt_hae, unit))),
        Line::from(format!("Alt MSL: {}", fmt_altitude(data.alt_msl, unit))),
        Line::from(format!(
            "Geoid:   {}",
            fmt_distance(data.geoid_sep, unit, 1)
        )),
        Line::from(vec![
            Span::raw("Age:     "),
            Span::styled(fmt_age(age), Style::default().fg(fix_age_color(age))),
        ]),
    ];

    let block = Block::bordered().title(panel_title("Position", lines.len(), area.height));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
    super::mark_data_state(f, area, app);
}

/// Position hold statistics: sample count and duration, mean position (in
/// the selected coordinate format), mean MSL altitude, per-axis standard
/// deviation and CEP50/CEP95, all lengths in the selected unit. Renders
/// nothing when hold is off.
pub fn render_hold(f: &mut Frame, area: Rect, app: &App) {
    let Some(hold) = &app.position_hold else {
        return;
    };
    let cf = app.coord_format.as_str();
    let unit = app.units.as_str();
    let dim = Style::default().fg(Color::DarkGray);

    let lines = match hold.result() {
        None => vec![
            Line::styled("Collecting fixes\u{2026}", Style::default().bold()),
            Line::styled(format!("{} of 2 fixes for stats", hold.count()), dim),
        ],
        Some(r) => {
            let cep = |label: &'static str, v: f64| {
                Line::from(vec![
                    Span::raw(label),
                    Span::styled(
                        fmt_distance(v, unit, 2),
                        Style::default().fg(cep_color(v)).bold(),
                    ),
                ])
            };
            vec![
                Line::from(vec![
                    Span::raw("Fixes: "),
                    Span::styled(r.count.to_string(), Style::default().bold()),
                    Span::styled(format!("  {}", fmt_duration(r.duration)), dim),
                ]),
                Line::from(vec![
                    Span::raw("Lat: "),
                    Span::styled(fmt_coord(r.mean_lat, "lat", cf), Style::default().bold()),
                ]),
                Line::from(vec![
                    Span::raw("Lon: "),
                    Span::styled(fmt_coord(r.mean_lon, "lon", cf), Style::default().bold()),
                ]),
                Line::from(format!("Alt MSL: {}", fmt_altitude(r.mean_alt, unit))),
                cep("CEP50: ", r.cep50),
                cep("CEP95: ", r.cep95),
                Line::from(format!(
                    "\u{03c3}N/E: {}",
                    fmt_distance_pair(r.std_north_m, r.std_east_m, unit, 2)
                )),
            ]
        }
    };

    let block = Block::bordered().title(panel_title("Hold", lines.len(), area.height));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
    super::mark_data_state(f, area, app);
}
