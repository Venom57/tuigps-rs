use ratatui::prelude::*;
use ratatui::widgets::*;

use super::constellation_counts;
use crate::app::{Action, ActiveTab, App, GLOBAL_BINDINGS, KeyBinding, tab_bindings};
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
    // A pending device confirmation stays visible until answered.
    if let Some(confirm) = &app.device_config.confirm {
        let style = if confirm.destructive() {
            Style::default().fg(Color::White).bg(Color::Red).bold()
        } else {
            Style::default().fg(Color::Black).bg(Color::Yellow).bold()
        };
        return vec![
            Span::raw(" "),
            Span::styled(format!(" {} y/n ", confirm.prompt), style),
        ];
    }

    // A transient status message temporarily replaces the hints.
    if !app.status_message.is_empty() {
        return vec![Span::styled(
            format!(" {}", app.status_message),
            Style::default().fg(Color::Yellow),
        )];
    }

    hint_spans(app.active_tab)
}

/// Key hints generated from the key binding table: `?:help` first, then
/// the tab's keys (least discoverable), then the global keys.
fn hint_spans(tab: ActiveTab) -> Vec<Span<'static>> {
    let key_style = Style::default().fg(Color::Yellow).bold();
    let sep = || Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray));
    let hint = |b: &KeyBinding, spans: &mut Vec<Span<'static>>| {
        if let Some(h) = b.hint {
            spans.push(Span::styled(b.label, key_style));
            spans.push(Span::raw(format!(":{} ", h)));
        }
    };

    let mut spans = vec![Span::raw(" ")];
    let (help, globals): (Vec<&KeyBinding>, Vec<&KeyBinding>) = GLOBAL_BINDINGS
        .iter()
        .partition(|b| b.action == Action::ToggleHelp);
    for b in help {
        hint(b, &mut spans);
    }
    spans.push(sep());
    let tab_bindings = tab_bindings(tab);
    if tab_bindings.iter().any(|b| b.hint.is_some()) {
        for b in tab_bindings {
            hint(b, &mut spans);
        }
        spans.push(sep());
    }
    for b in globals {
        hint(b, &mut spans);
    }
    spans
}

fn right_spans(app: &App, total_width: usize) -> Vec<Span<'static>> {
    let data = &app.gps_data;
    let mut spans = vec![];

    if let Some(points) = app.log_points() {
        spans.push(Span::styled(
            " REC ",
            Style::default().fg(Color::White).bg(Color::Red).bold(),
        ));
        let formats: Vec<&str> = app.loggers.iter().map(|l| l.format().label()).collect();
        spans.push(Span::raw(format!(" {} {} pts ", formats.join("+"), points)));
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
