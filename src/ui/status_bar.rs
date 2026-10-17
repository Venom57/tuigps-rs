use ratatui::prelude::*;
use ratatui::widgets::*;

use super::constellation_counts;
use crate::app::{ActiveTab, App};
use crate::constants::gnss_short;

/// Columns of key hints we try to keep visible before giving space to the
/// optional detail text (constellation breakdown / error message).
const MIN_HINT_WIDTH: usize = 20;

/// Longest error message shown next to the DISCONNECTED badge.
const MAX_DETAIL_WIDTH: usize = 32;

/// Footer: key hints (or a transient status message) on the left, and a
/// right-anchored chunk with REC/HOLD and the connection badge. The right
/// chunk is laid out first so the hints are what gets truncated on narrow
/// terminals.
pub fn render_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let right = Line::from(right_spans(app, area.width as usize));
    let right_w = (right.width() as u16).min(area.width);

    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(right_w)]).areas(area);

    f.render_widget(Paragraph::new(Line::from(left_spans(app))), left_area);
    f.render_widget(Paragraph::new(right), right_area);
}

fn left_spans(app: &App) -> Vec<Span<'static>> {
    // A transient status message temporarily replaces the hints.
    if !app.status_message.is_empty() {
        return vec![Span::styled(
            format!(" {}", app.status_message),
            Style::default().fg(Color::Yellow),
        )];
    }

    let tab_hints: &[(&str, &str)] = match app.active_tab {
        ActiveTab::Nmea => &[("p", "pause"), ("f", "filter"), ("c", "clear")],
        ActiveTab::Timing => &[("a", "arm"), ("c", "clear"), ("k", "clock")],
        ActiveTab::Satellites => &[("\u{2191}\u{2193}", "scroll")],
        ActiveTab::Device => &[
            ("\u{2191}\u{2193}", "select"),
            ("\u{2190}\u{2192}", "adjust"),
            ("\u{23ce}", "apply"),
        ],
        ActiveTab::Dashboard => &[],
    };
    let global_hints: &[(&str, &str)] = &[
        ("q", "quit"),
        ("r", "reconnect"),
        ("s", "settings"),
        ("u", "units"),
        ("m", "maps"),
        ("l", "log"),
        ("h", "hold"),
    ];

    let key_style = Style::default().fg(Color::Yellow).bold();
    let mut spans = vec![Span::raw(" ")];
    // Context-specific keys first; they are the least discoverable.
    for (i, hints) in [tab_hints, global_hints].into_iter().enumerate() {
        if i == 1 && !tab_hints.is_empty() {
            spans.push(Span::styled(
                "\u{2502} ",
                Style::default().fg(Color::DarkGray),
            ));
        }
        for (key, action) in hints {
            spans.push(Span::styled(*key, key_style));
            spans.push(Span::raw(format!(":{} ", action)));
        }
    }
    spans
}

fn right_spans(app: &App, total_width: usize) -> Vec<Span<'static>> {
    let data = &app.gps_data;
    let mut spans = vec![];

    if let Some(logger) = app.logger.as_ref().filter(|l| l.active) {
        spans.push(Span::styled(
            " REC ",
            Style::default().fg(Color::White).bg(Color::Red).bold(),
        ));
        spans.push(Span::raw(format!(" {} pts ", logger.point_count)));
    }
    if app.position_hold.is_some() {
        spans.push(Span::styled(
            " HOLD ",
            Style::default().fg(Color::Black).bg(Color::Cyan).bold(),
        ));
        spans.push(Span::raw(" "));
    }

    let (badge, detail) = if !data.connected {
        (
            Span::styled(
                " DISCONNECTED ",
                Style::default().fg(Color::White).bg(Color::Red).bold(),
            ),
            data.error_message.clone(),
        )
    } else if app.stale {
        (
            Span::styled(
                format!(" STALE ({:.0}s) ", app.stale_seconds),
                Style::default().fg(Color::Black).bg(Color::Yellow).bold(),
            ),
            String::new(),
        )
    } else {
        (
            Span::styled(
                " CONNECTED ",
                Style::default().fg(Color::Black).bg(Color::Green).bold(),
            ),
            used_breakdown(app),
        )
    };
    spans.push(badge);

    // Optional detail text only gets space left over after the badges and
    // a minimum amount of hints.
    let used: usize = spans.iter().map(Span::width).sum();
    let budget = total_width
        .saturating_sub(used + MIN_HINT_WIDTH + 2)
        .min(MAX_DETAIL_WIDTH);
    if !detail.is_empty() && budget > 0 {
        spans.push(Span::raw(format!(" {} ", truncate(&detail, budget))));
    }

    spans
}

/// e.g. "8GP+5GA+4GL" — satellites used per constellation.
fn used_breakdown(app: &App) -> String {
    let counts = constellation_counts(&app.gps_data.satellites);
    let mut out = String::new();
    for (id, &(_, used)) in counts.iter().enumerate() {
        if used > 0 {
            if !out.is_empty() {
                out.push('+');
            }
            out.push_str(&format!("{}{}", used, gnss_short(id as u8)));
        }
    }
    out
}

/// Truncate to at most `max` chars, marking the cut with an ellipsis.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('\u{2026}');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_and_long() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 4), "abc\u{2026}");
        assert_eq!(truncate("abc", 0), "");
    }
}
