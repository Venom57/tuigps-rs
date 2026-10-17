use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::formatting::{fmt, fmt_climb, fmt_distance, fmt_speed_dec, panel_title, plus_minus};

/// Number of lines the Errors panel wants (for dashboard layout).
pub const LINES: u16 = 9;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let e = &app.gps_data.errors;
    let unit = app.units.as_str();
    let dist = |v: f64| plus_minus(fmt_distance(v, unit, 2));

    // Most important first: lower lines are dropped when the panel is short.
    let rows: [(&str, String); LINES as usize] = [
        ("EPH", dist(e.eph)),
        ("EPV", dist(e.epv)),
        ("EPS", plus_minus(fmt_speed_dec(e.eps, unit, 2))),
        ("EPT", plus_minus(fmt(e.ept, 6, " s"))),
        ("SEP", dist(e.sep)),
        ("EPD", plus_minus(fmt(e.epd, 1, "°"))),
        ("EPC", plus_minus(fmt_climb(e.epc, unit))),
        ("EPX", dist(e.epx)),
        ("EPY", dist(e.epy)),
    ];
    let lines: Vec<Line> = rows
        .into_iter()
        .map(|(label, value)| Line::from(format!("{label}:  {value}")))
        .collect();

    let block = Block::bordered().title(panel_title("Errors", lines.len(), area.height));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
    super::mark_data_state(f, area, app);
}
