use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::bearing_to_compass;
use crate::formatting::{fmt, fmt_speed};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Velocity ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let data = &app.gps_data;
    let unit = app.units.as_str();

    let lines = vec![
        Line::from(vec![
            Span::raw("Speed: "),
            Span::styled(
                fmt_speed(data.speed, unit),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(format!(
            "Track: {} ({})",
            fmt(data.track, 1, "°"),
            bearing_to_compass(data.track)
        )),
        Line::from(format!("Mag Track: {}", fmt(data.magtrack, 1, "°"))),
        Line::from(format!("Climb: {}", fmt(data.climb, 2, " m/s"))),
        Line::from(format!("Mag Var: {}", fmt(data.magvar, 1, "°"))),
    ];

    f.render_widget(Paragraph::new(lines), inner);
}
