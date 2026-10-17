use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::bearing_to_compass;
use crate::formatting::{fmt, fmt_climb, fmt_speed, panel_title};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let data = &app.gps_data;
    let unit = app.units.as_str();

    let lines = vec![
        Line::from(vec![
            Span::raw("Speed: "),
            Span::styled(fmt_speed(data.speed, unit), Style::default().bold()),
        ]),
        Line::from(format!(
            "Track: {} ({})",
            fmt(data.track, 1, "°"),
            bearing_to_compass(data.track)
        )),
        Line::from(format!("Climb: {}", fmt_climb(data.climb, unit))),
        Line::from(format!("Mag Track: {}", fmt(data.magtrack, 1, "°"))),
        Line::from(format!("Mag Var: {}", fmt(data.magvar, 1, "°"))),
    ];

    let block = Block::bordered().title(panel_title("Velocity", lines.len(), area.height));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
    super::mark_data_state(f, area, app);
}
