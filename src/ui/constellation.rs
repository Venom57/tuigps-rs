use ratatui::prelude::*;
use ratatui::widgets::*;

use super::constellation_counts;
use crate::app::App;
use crate::constants::{gnss_color, gnss_name};

/// Render constellation summary as a compact single-line bar
pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let counts = constellation_counts(&app.gps_data.satellites);

    let mut total_visible = 0u32;
    let mut total_used = 0u32;

    let mut spans = vec![Span::styled(" Constellations: ", Style::default().bold())];

    for (gnssid, &(visible, used)) in counts.iter().enumerate() {
        if visible == 0 {
            continue;
        }
        let gnssid = gnssid as u8;
        total_visible += visible;
        total_used += used;
        spans.push(Span::styled(
            gnss_name(gnssid),
            Style::default().fg(gnss_color(gnssid)),
        ));
        spans.push(Span::raw(format!(" {}/{} ", used, visible)));
    }

    spans.push(Span::styled("| ", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled("Total ", Style::default().bold()));
    spans.push(Span::styled(
        format!("{}/{}", total_used, total_visible),
        Style::default().bold(),
    ));

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
