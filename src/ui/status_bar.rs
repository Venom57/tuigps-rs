use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::gnss_short;

pub fn render_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let data = &app.gps_data;

    let mut spans = vec![];

    for (key, action) in &[
        ("q", "quit"),
        ("r", "reconnect"),
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
        spans.push(Span::raw(" "));
    }

    // Connection status with staleness and constellation breakdown
    if !data.connected {
        spans.push(Span::styled(
            " DISCONNECTED ",
            Style::default().fg(Color::White).bg(Color::Red),
        ));
        if !data.error_message.is_empty() {
            spans.push(Span::raw(format!(" {} ", data.error_message)));
        }
    } else if app.stale {
        spans.push(Span::styled(
            format!(" STALE ({:.0}s) ", app.stale_seconds),
            Style::default().fg(Color::White).bg(Color::Yellow),
        ));
    } else {
        spans.push(Span::styled(
            " CONNECTED ",
            Style::default().fg(Color::Black).bg(Color::Green),
        ));

        // Constellation breakdown
        let counts = data.constellation_counts();
        if !counts.is_empty() {
            let mut parts: Vec<(u8, u32, u32)> = counts.into_iter().map(|(id, (v, u))| (id, v, u)).collect();
            parts.sort_by_key(|(id, _, _)| *id);
            let breakdown: Vec<String> = parts
                .iter()
                .filter(|(_, _, u)| *u > 0)
                .map(|(id, _, u)| format!("{}{}", u, gnss_short(*id)))
                .collect();
            if !breakdown.is_empty() {
                spans.push(Span::raw(format!(" {} ", breakdown.join("+"))));
            }
        }
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
