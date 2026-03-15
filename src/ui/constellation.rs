use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::{gnss_color, gnss_name};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Constellations ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let counts = app.gps_data.constellation_counts();
    let mut entries: Vec<_> = counts.into_iter().collect();
    entries.sort_by_key(|(id, _)| *id);

    let mut lines = vec![];
    let mut total_visible = 0u32;
    let mut total_used = 0u32;

    for (gnssid, (visible, used)) in &entries {
        let color = gnss_color(*gnssid);
        total_visible += visible;
        total_used += used;
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<10}", gnss_name(*gnssid)),
                Style::default().fg(color),
            ),
            Span::raw(format!("{}/{}", used, visible)),
        ]));
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{:<10}", "Total"),
            Style::default().fg(Color::White).bold(),
        ),
        Span::styled(
            format!("{}/{}", total_used, total_visible),
            Style::default().bold(),
        ),
    ]));

    f.render_widget(Paragraph::new(lines), inner);
}
