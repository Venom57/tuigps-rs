use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::formatting::fmt;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Errors ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let e = &app.gps_data.errors;

    let lines = vec![
        Line::from(format!("EPH:  ±{}", fmt(e.eph, 2, " m"))),
        Line::from(format!("EPV:  ±{}", fmt(e.epv, 2, " m"))),
        Line::from(format!("EPT:  ±{}", fmt(e.ept, 6, " s"))),
        Line::from(format!("EPS:  ±{}", fmt(e.eps, 2, " m/s"))),
        Line::from(format!("EPD:  ±{}", fmt(e.epd, 1, "°"))),
        Line::from(format!("EPC:  ±{}", fmt(e.epc, 2, " m/s"))),
        Line::from(format!("SEP:   {}", fmt(e.sep, 2, " m"))),
    ];

    f.render_widget(Paragraph::new(lines), inner);
}
