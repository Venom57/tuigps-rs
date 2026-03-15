use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::formatting::{fmt, fmt_altitude, fmt_coord};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Position ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let data = &app.gps_data;
    let cf = app.coord_format.as_str();
    let unit = app.units.as_str();

    let mut lines = vec![
        Line::from(vec![
            Span::raw("Lat: "),
            Span::styled(
                fmt_coord(data.latitude, "lat", cf),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(vec![
            Span::raw("Lon: "),
            Span::styled(
                fmt_coord(data.longitude, "lon", cf),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(format!("Alt HAE: {}", fmt_altitude(data.alt_hae, unit))),
        Line::from(format!("Alt MSL: {}", fmt_altitude(data.alt_msl, unit))),
        Line::from(format!("Geoid:   {}", fmt(data.geoid_sep, 1, " m"))),
    ];

    if let Some(hold) = &app.position_hold
        && let Some(result) = hold.result() {
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::raw("CEP50: "),
                Span::styled(
                    format!("{:.2} m", result.cep50),
                    Style::default().fg(cep_color(result.cep50)),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::raw("CEP95: "),
                Span::styled(
                    format!("{:.2} m", result.cep95),
                    Style::default().fg(cep_color(result.cep95)),
                ),
            ]));
        }

    f.render_widget(Paragraph::new(lines), inner);
}

fn cep_color(cep: f64) -> Color {
    if cep < 1.0 {
        Color::Green
    } else if cep < 5.0 {
        Color::Yellow
    } else {
        Color::Red
    }
}
