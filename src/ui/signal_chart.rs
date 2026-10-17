use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::{gnss_color, gnss_short};

/// SNR that fills the whole bar; stronger signals are clamped.
const MAX_SNR: f64 = 55.0;

/// Width of the "+GP 12 " prefix before each bar.
const LABEL_WIDTH: u16 = 7;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Signal ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let data = &app.gps_data;
    let mut sats: Vec<_> = data
        .satellites
        .iter()
        .filter(|s| s.snr.is_finite())
        .collect();

    sats.sort_unstable_by(|a, b| {
        b.used.cmp(&a.used).then(
            b.snr
                .partial_cmp(&a.snr)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });

    let bar_width = inner.width.saturating_sub(LABEL_WIDTH + 1) as usize;
    let buf = f.buffer_mut();

    for (i, sat) in sats.iter().enumerate().take(inner.height as usize) {
        let label = format!("{:>2}{:>3}", gnss_short(sat.gnssid), sat.svid);
        let filled = ((sat.snr / MAX_SNR).clamp(0.0, 1.0) * bar_width as f64).round() as usize;
        let filled = filled.min(bar_width);
        let empty = bar_width - filled;

        let color = gnss_color(sat.gnssid);
        let used_marker = if sat.used { "+" } else { " " };

        let line = Line::from(vec![
            Span::styled(used_marker, Style::default().bold()),
            Span::styled(label, Style::default().fg(color)),
            Span::raw(" "),
            Span::styled("\u{2588}".repeat(filled), Style::default().fg(color)),
            Span::styled(
                "\u{2591}".repeat(empty),
                Style::default().fg(Color::DarkGray),
            ),
        ]);

        buf.set_line(inner.x, inner.y + i as u16, &line, inner.width);
    }

    super::mark_data_state(f, area, app);
}
