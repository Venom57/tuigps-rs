use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;

pub fn render_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let data = &app.gps_data;

    let mut spans = vec![];

    for (key, action) in &[
        ("q", "quit"),
        ("s", "settings"),
        ("u", "units"),
        ("m", "maps"),
        ("l", "log"),
        ("h", "hold"),
    ] {
        spans.push(Span::styled(
            format!(" {} ", key),
            Style::default().fg(Color::White).bg(Color::DarkGray),
        ));
        spans.push(Span::raw(format!("{} ", action)));
    }

    // Activity badges
    if app.logger.as_ref().is_some_and(|l| l.active) {
        spans.push(Span::styled(
            " REC ",
            Style::default().fg(Color::White).bg(Color::Red),
        ));
        spans.push(Span::raw(format!(
            " {} pts ",
            app.logger.as_ref().unwrap().point_count
        )));
    }
    if app.position_hold.is_some() {
        spans.push(Span::styled(
            " HOLD ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ));
    }

    // Connection status
    let status_span = if !data.connected {
        Span::styled(
            " DISCONNECTED ",
            Style::default().fg(Color::White).bg(Color::Red),
        )
    } else if data.error_message.is_empty() {
        Span::styled(
            " CONNECTED ",
            Style::default().fg(Color::Black).bg(Color::Green),
        )
    } else {
        Span::styled(
            format!(" {} ", data.error_message),
            Style::default().fg(Color::White).bg(Color::Red),
        )
    };
    spans.push(status_span);

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
