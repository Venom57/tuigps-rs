use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::{dop_rating, mode_color, mode_name, status_color, status_name};
use crate::formatting::fmt;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Fix ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let data = &app.gps_data;
    let mut lines = vec![];

    lines.push(Line::from(vec![
        Span::raw("Mode:   "),
        Span::styled(
            mode_name(data.mode),
            Style::default().fg(mode_color(data.mode)).bold(),
        ),
    ]));

    lines.push(Line::from(vec![
        Span::raw("Status: "),
        Span::styled(
            status_name(data.status),
            Style::default().fg(status_color(data.status)),
        ),
    ]));

    lines.push(Line::from(format!(
        "Sats:   {}/{}",
        data.satellites_used,
        data.satellites.len()
    )));

    lines.push(Line::raw(""));

    for (label, value) in [
        ("HDOP", data.dop.hdop),
        ("VDOP", data.dop.vdop),
        ("PDOP", data.dop.pdop),
        ("GDOP", data.dop.gdop),
    ] {
        let (rating, color) = dop_rating(value);
        lines.push(Line::from(vec![
            Span::raw(format!("{}: ", label)),
            Span::styled(fmt(value, 1, ""), Style::default().fg(color)),
            Span::styled(
                format!(" ({})", rating),
                Style::default().fg(color).dim(),
            ),
        ]));
    }

    f.render_widget(Paragraph::new(lines), inner);
}
