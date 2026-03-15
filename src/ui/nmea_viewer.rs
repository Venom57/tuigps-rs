use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(format!(
        " NMEA {} {} ",
        if app.nmea_paused { "[PAUSED]" } else { "" },
        if app.nmea_filter.is_empty() {
            String::new()
        } else {
            format!("[{}]", app.nmea_filter)
        },
    ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines: Vec<Line> = app
        .nmea_buffer
        .iter()
        .rev()
        .filter(|s| {
            if app.nmea_filter.is_empty() {
                true
            } else {
                nmea_type(s) == app.nmea_filter
            }
        })
        .take(inner.height as usize)
        .map(|s| {
            let color = nmea_color(nmea_type(s));
            Line::styled(s.clone(), Style::default().fg(color))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
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
