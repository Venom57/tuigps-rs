use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let filter_label = if app.nmea_filter.is_empty() {
        "ALL".to_string()
    } else {
        app.nmea_filter.clone()
    };

    let title = format!(
        " NMEA [{}]{} ({}) ",
        filter_label,
        if app.nmea_paused { " [PAUSED]" } else { "" },
        app.nmea_buffer.len(),
    );
    let block = Block::bordered().title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let height = inner.height as usize;
    if height == 0 {
        return;
    }

    // Filter sentences
    let filtered: Vec<&String> = app
        .nmea_buffer
        .iter()
        .filter(|s| {
            if app.nmea_filter.is_empty() {
                true
            } else {
                nmea_type(s) == app.nmea_filter
            }
        })
        .collect();

    if filtered.is_empty() {
        // Show help when empty
        let help = vec![
            Line::raw(""),
            Line::styled(
                "  Waiting for NMEA sentences...",
                Style::default().fg(Color::DarkGray),
            ),
            Line::raw(""),
            Line::styled(
                "  p: pause/resume  f: cycle filter  c: clear",
                Style::default().fg(Color::DarkGray),
            ),
            Line::styled(
                "  Up/Down: scroll  PageUp/PageDown: fast scroll",
                Style::default().fg(Color::DarkGray),
            ),
        ];
        f.render_widget(Paragraph::new(help), inner);
        return;
    }

    // Apply scroll offset (offset is from the bottom)
    let total = filtered.len();
    let scroll = app.nmea_scroll_offset.min(total.saturating_sub(height));
    let end = total.saturating_sub(scroll);
    let start = end.saturating_sub(height);

    let lines: Vec<Line> = filtered[start..end]
        .iter()
        .map(|s| {
            let color = nmea_color(nmea_type(s));
            Line::styled((*s).clone(), Style::default().fg(color))
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner);

    // Show scroll indicator if not at bottom
    if scroll > 0 {
        let indicator = format!(" +{} ", scroll);
        let indicator_area = Rect::new(
            inner.x + inner.width.saturating_sub(indicator.len() as u16 + 1),
            area.y,
            indicator.len() as u16,
            1,
        );
        f.render_widget(
            Paragraph::new(indicator).style(Style::default().fg(Color::Yellow)),
            indicator_area,
        );
    }
}

fn nmea_type(sentence: &str) -> &str {
    if sentence.len() > 6 && sentence.starts_with('$') {
        &sentence[3..6]
    } else {
        "???"
    }
}

fn nmea_color(sentence_type: &str) -> Color {
    match sentence_type {
        "GGA" => Color::Green,
        "RMC" => Color::Blue,
        "GSA" => Color::Yellow,
        "GSV" => Color::LightYellow,
        "VTG" => Color::Cyan,
        "GLL" => Color::Magenta,
        "ZDA" => Color::LightCyan,
        "TXT" => Color::DarkGray,
        _ => Color::White,
    }
}
