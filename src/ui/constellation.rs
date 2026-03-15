use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::{gnss_color, gnss_name};

/// Render constellation summary as a compact single-line bar
pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let counts = app.gps_data.constellation_counts();
    let mut entries: Vec<_> = counts.into_iter().collect();
    entries.sort_by_key(|(id, _)| *id);

    let mut total_visible = 0u32;
    let mut total_used = 0u32;

    let mut spans = vec![
        Span::styled(" Constellations: ", Style::default().fg(Color::White).bold()),
    ];

    for (gnssid, (visible, used)) in &entries {
        let color = gnss_color(*gnssid);
        total_visible += visible;
        total_used += used;
        spans.push(Span::styled(
            gnss_name(*gnssid),
            Style::default().fg(color),
        ));
        spans.push(Span::raw(format!(" {}/{} ", used, visible)));
    }

    spans.push(Span::styled("| ", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled("Total ", Style::default().fg(Color::White).bold()));
    spans.push(Span::styled(
        format!("{}/{}", total_used, total_visible),
        Style::default().bold(),
    ));

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
