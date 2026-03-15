use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::formatting::{fmt, fmt_offset, fmt_time_iso};

pub fn render(f: &mut Frame, area: Rect, app: &App, show_pps: bool) {
    let title = if show_pps { " Timing " } else { " Time " };
    let block = Block::bordered().title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let data = &app.gps_data;
    let (date, time) = fmt_time_iso(&data.time);

    let mut lines = vec![
        Line::from(vec![
            Span::raw("Date: "),
            Span::styled(&date, Style::default().fg(Color::White).bold()),
        ]),
        Line::from(vec![
            Span::raw("Time: "),
            Span::styled(&time, Style::default().fg(Color::White).bold()),
        ]),
        Line::from(format!("EPT:  ±{}", fmt(data.errors.ept, 6, " s"))),
        Line::from(format!("Leap: {}", data.leapseconds)),
    ];

    if show_pps {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::raw("PPS Offset: "),
            Span::styled(
                fmt_offset(data.pps_offset_us() / 1e6),
                Style::default().fg(Color::Cyan),
            ),
        ]));

        // TOFF stats
        if !data.toff_samples.is_empty() {
            if let Some((mean, std, min, max)) = toff_stats(&data.toff_samples) {
                lines.push(Line::raw(""));
                lines.push(Line::from(format!("TOFF samples: {}", data.toff_samples.len())));
                lines.push(Line::from(format!("  Mean: {}", fmt_offset(mean))));
                lines.push(Line::from(format!("  Std:  {}", fmt_offset(std))));
                lines.push(Line::from(format!("  Min:  {}", fmt_offset(min))));
                lines.push(Line::from(format!("  Max:  {}", fmt_offset(max))));
            }
        }

        // Armed TOFF
        if data.toff_armed_offset.is_finite() {
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled("Armed TOFF: ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    fmt_offset(data.toff_armed_offset),
                    Style::default().fg(Color::Yellow).bold(),
                ),
            ]));
        }

        lines.push(Line::raw(""));
        lines.push(Line::from(format!("TDOP: {}", fmt(data.dop.tdop, 1, ""))));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn toff_stats(samples: &[f64]) -> Option<(f64, f64, f64, f64)> {
    if samples.is_empty() {
        return None;
    }
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    let variance = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    let std = variance.sqrt();
    let min = samples.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    Some((mean, std, min, max))
}
