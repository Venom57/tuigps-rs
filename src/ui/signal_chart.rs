use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::{gnss_color, gnss_short};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Signal ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let data = &app.gps_data;
    let mut sats: Vec<_> = data.satellites.iter().filter(|s| s.snr.is_finite()).collect();

    sats.sort_by(|a, b| {
        b.used
            .cmp(&a.used)
            .then(b.snr.partial_cmp(&a.snr).unwrap_or(std::cmp::Ordering::Equal))
    });

    let max_snr = 55.0;
    let bar_width = inner.width.saturating_sub(8) as f64;

    for (i, sat) in sats.iter().enumerate().take(inner.height as usize) {
        let label = format!("{:>2}{:>3}", gnss_short(sat.gnssid), sat.svid);
        let filled = (sat.snr / max_snr * bar_width).round() as usize;
        let empty = bar_width as usize - filled.min(bar_width as usize);

        let color = gnss_color(sat.gnssid);
        let used_marker = if sat.used { "+" } else { " " };

        let line = Line::from(vec![
            Span::styled(used_marker, Style::default().fg(Color::White)),
            Span::styled(label, Style::default().fg(color)),
            Span::raw(" "),
            Span::styled("\u{2588}".repeat(filled), Style::default().fg(color)),
            Span::styled("\u{2591}".repeat(empty), Style::default().fg(Color::DarkGray)),
        ]);

        f.render_widget(
            Paragraph::new(vec![line]),
            Rect::new(inner.x, inner.y + i as u16, inner.width, 1),
        );
    }
}
