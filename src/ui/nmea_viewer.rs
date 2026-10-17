use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{ActiveTab, App, tab_bindings};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let filter_label = if app.nmea_filter.is_empty() {
        "ALL"
    } else {
        app.nmea_filter.as_str()
    };

    let height = area.height.saturating_sub(2) as usize;
    // Record the view height so key handling can clamp scroll offsets.
    app.nmea_view_rows.set(height);

    let matches = |s: &&String| app.nmea_filter.is_empty() || nmea_type(s) == app.nmea_filter;

    // Count matches without collecting them; only the visible window is
    // gathered below.
    let total = app.nmea_visible_count();
    // Offset is from the bottom (newest line).
    let scroll = app.nmea_scroll_offset.min(total.saturating_sub(height));

    let (mode, mode_style) = if app.nmea_paused {
        (
            " PAUSED (p resumes) ".to_string(),
            Style::default().fg(Color::Black).bg(Color::Yellow).bold(),
        )
    } else if scroll > 0 {
        (
            format!(" SCROLLED +{} (End follows) ", scroll),
            Style::default().fg(Color::Black).bg(Color::Cyan).bold(),
        )
    } else {
        (
            " FOLLOWING ".to_string(),
            Style::default().fg(Color::Green).bold(),
        )
    };
    let title = Line::from(vec![
        Span::raw(format!(
            " NMEA [{}] ({}) ",
            filter_label,
            app.nmea_buffer.len()
        )),
        Span::styled(mode, mode_style),
        Span::raw(" "),
    ]);
    let block = Block::bordered().title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if height == 0 {
        return;
    }

    if total == 0 {
        // Show help when empty, generated from the key binding table.
        let mut help = vec![
            Line::raw(""),
            Line::styled("  Waiting for NMEA sentences...", Style::default().italic()),
            Line::raw(""),
        ];
        for b in tab_bindings(ActiveTab::Nmea) {
            help.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{:<10}", b.label),
                    Style::default().fg(Color::Yellow).bold(),
                ),
                Span::raw(b.desc),
            ]));
        }
        f.render_widget(Paragraph::new(help), inner);
        return;
    }

    // Walk backwards from the newest line, skipping the scrolled-off tail,
    // and borrow just the lines that fit.
    let mut window: Vec<&String> = app
        .nmea_buffer
        .iter()
        .rev()
        .filter(matches)
        .skip(scroll)
        .take(height)
        .collect();
    window.reverse();

    let lines: Vec<Line> = window
        .into_iter()
        .map(|s| Line::styled(s.as_str(), Style::default().fg(nmea_color(nmea_type(s)))))
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

/// Sentence type used for filtering and coloring.
///
/// - Standard sentences (`$GPGGA,...`, `!AIVDM,...`) drop the 2-char talker
///   ID: `GGA`, `VDM`.
/// - Proprietary sentences (address starting with `P`, e.g. `$PUBX,00,...`,
///   `$PGRME,...`) return the whole address: `PUBX`, `PGRME`.
/// - Anything else (no `$`/`!`, short or non-ASCII address) is `???`.
///
/// Never slices inside a multibyte character.
pub fn nmea_type(sentence: &str) -> &str {
    let Some(body) = sentence
        .strip_prefix('$')
        .or_else(|| sentence.strip_prefix('!'))
    else {
        return "???";
    };
    let address = body.split([',', '*']).next().unwrap_or("");
    if address.is_empty() || !address.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return "???";
    }
    if address.starts_with('P') {
        address
    } else if address.len() >= 5 {
        // ASCII-only, so byte indexing is on a char boundary.
        &address[2..]
    } else {
        "???"
    }
}

fn nmea_color(sentence_type: &str) -> Color {
    match sentence_type {
        "GGA" => Color::Green,
        "RMC" => Color::LightBlue,
        "GSA" => Color::Yellow,
        "GSV" => Color::LightYellow,
        "VTG" => Color::Cyan,
        "GLL" => Color::Magenta,
        "ZDA" => Color::LightCyan,
        "TXT" => Color::DarkGray,
        _ => Color::Reset,
    }
}

#[cfg(test)]
mod tests {
    use super::nmea_type;

    #[test]
    fn standard_sentences() {
        assert_eq!(nmea_type("$GPGGA,123519,4807.038,N"), "GGA");
        assert_eq!(nmea_type("$GNRMC,1,2,3*6A"), "RMC");
        assert_eq!(
            nmea_type("!AIVDM,1,1,,A,15M67FC000G?ufbE`FepT@3n00Sa,0*5C"),
            "VDM"
        );
        assert_eq!(nmea_type("$GPZDA*48"), "ZDA");
    }

    #[test]
    fn proprietary_sentences() {
        assert_eq!(nmea_type("$PUBX,00,081350.00,4717.113210,N"), "PUBX");
        assert_eq!(nmea_type("$PGRME,15.0,M,45.0,M,25.0,M*1C"), "PGRME");
    }

    #[test]
    fn malformed_input_does_not_panic() {
        for s in [
            "",
            "$",
            "!",
            "$G",
            "$GPG",
            "$,",
            "GPGGA,1",
            "$GP\u{00e9}GA,1",
            "$\u{1f600}\u{1f600}",
            "$GPGGA",
            "$ab\u{00e9}",
            "$GP GGA,1",
        ] {
            let t = nmea_type(s);
            assert!(t == "???" || t.is_ascii(), "{s:?} -> {t:?}");
        }
        assert_eq!(nmea_type("$GP\u{00e9}GA,1"), "???");
        assert_eq!(nmea_type("$GPG"), "???");
        assert_eq!(nmea_type("GPGGA,1"), "???");
        assert_eq!(nmea_type("$GPGGA"), "GGA");
    }
}
