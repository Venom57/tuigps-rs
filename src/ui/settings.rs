use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;

pub fn render_settings(f: &mut Frame, area: Rect, app: &App) {
    // Clear background
    f.render_widget(Clear, area);

    // Centered dialog
    let dialog_area = centered_rect(60, 14, area);
    let block = Block::bordered()
        .title(" Settings ")
        .style(Style::default().bg(Color::DarkGray));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::raw("  Host:   "),
            Span::styled(&app.host, Style::default().fg(Color::White).bold()),
        ]),
        Line::from(vec![
            Span::raw("  Port:   "),
            Span::styled(
                format!("{}", app.port),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::raw("  Units:  "),
            Span::styled(
                app.units.as_str(),
                Style::default().fg(Color::Yellow).bold(),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Coords: "),
            Span::styled(
                app.coord_format.as_str(),
                Style::default().fg(Color::Yellow).bold(),
            ),
        ]),
        Line::raw(""),
        Line::styled(
            "  Press Esc to close",
            Style::default().fg(Color::DarkGray),
        ),
    ];

    f.render_widget(Paragraph::new(lines), inner);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
