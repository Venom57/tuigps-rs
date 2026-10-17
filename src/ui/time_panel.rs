use ratatui::prelude::*;
use ratatui::widgets::*;
use std::sync::atomic::Ordering;

use crate::app::App;
use crate::data_model::GPSData;
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
            Span::styled(&date, Style::default().bold()),
        ]),
        Line::from(vec![
            Span::raw("Time: "),
            Span::styled(&time, Style::default().bold()),
        ]),
        Line::from(format!("EPT:  \u{00b1}{}", fmt(data.errors.ept, 6, " s"))),
        // The data model uses 0 for "not reported"; the real GPS-UTC leap
        // second count has been 18 since 2017 and will never be 0 again.
        Line::from(if data.leapseconds == 0 {
            "Leap: ---".to_string()
        } else {
            format!("Leap: {}", data.leapseconds)
        }),
    ];

    if show_pps {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::raw("PPS Offset: "),
            Span::styled(
                if has_pps(data) {
                    fmt_offset(data.pps_offset_us() / 1e6)
                } else {
                    "---".to_string()
                },
                Style::default().fg(Color::Cyan),
            ),
        ]));

        // TOFF stats
        if !data.toff_samples.is_empty() {
            if let Some((mean, std, min, max)) = toff_stats(&data.toff_samples) {
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("TOFF ", Style::default().bold()),
                    Span::raw(format!("({} samples)", data.toff_samples.len())),
                ]));
                lines.push(Line::from(format!("  Mean: {}", fmt_offset(mean))));
                lines.push(Line::from(format!("  Std:  {}", fmt_offset(std))));
                lines.push(Line::from(format!("  Min:  {}", fmt_offset(min))));
                lines.push(Line::from(format!("  Max:  {}", fmt_offset(max))));
            }
        } else {
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "TOFF: no samples yet",
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Armed TOFF state
        let is_armed = app.armed_toff.load(Ordering::Relaxed);
        if is_armed {
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled(
                    " TOFF ARMED ",
                    Style::default().fg(Color::Black).bg(Color::Yellow),
                ),
                Span::styled(
                    " waiting for next fix...",
                    Style::default().fg(Color::Yellow),
                ),
            ]));
        } else if data.toff_armed_offset.is_finite() {
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled("Armed TOFF: ", Style::default().fg(Color::Green).bold()),
                Span::styled(
                    fmt_offset(data.toff_armed_offset),
                    Style::default().fg(Color::Green).bold(),
                ),
            ]));
            if !data.toff_armed_gps_time.is_empty() {
                lines.push(Line::from(vec![
                    Span::raw("  GPS time: "),
                    Span::raw(&data.toff_armed_gps_time),
                ]));
            }
        }

        lines.push(Line::raw(""));
        lines.push(Line::from(format!("TDOP: {}", fmt(data.dop.tdop, 1, ""))));
    }

    f.render_widget(Paragraph::new(lines), inner);
    super::mark_data_state(f, area, app);
}

/// No PPS message has been received while all timestamps are still zero.
fn has_pps(data: &GPSData) -> bool {
    let p = &data.pps;
    p.real_sec != 0 || p.real_nsec != 0 || p.clock_sec != 0 || p.clock_nsec != 0
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
