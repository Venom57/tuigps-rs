//! `?` help overlay. Content comes from the key binding tables in
//! `app.rs`, the same tables that drive key dispatch and the status bar.

use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{ActiveTab, App, GLOBAL_BINDINGS, KeyBinding, tab_bindings};

/// Inner width from which the global and tab sections sit side by side.
const TWO_COLUMN_MIN_INNER: u16 = 70;
const MAX_WIDTH: u16 = 92;

pub fn render_help(f: &mut Frame, area: Rect, app: &App) {
    if area.width < 4 || area.height < 3 {
        return;
    }

    let global = section("Global", GLOBAL_BINDINGS);
    let tab = tab_section(app.active_tab);
    let footer = Line::from(vec![
        Span::styled("Esc", key_style()),
        Span::raw(" or "),
        Span::styled("?", key_style()),
        Span::raw(" to close"),
    ]);

    let width = area.width.min(MAX_WIDTH);
    let two_columns = width.saturating_sub(2) >= TWO_COLUMN_MIN_INNER;
    let body_rows = if two_columns {
        global.len().max(tab.len())
    } else {
        global.len() + tab.len() + 1
    };
    let height = (body_rows as u16 + 4).min(area.height);
    let dialog = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );

    f.render_widget(Clear, dialog);
    let block = Block::bordered()
        .title(" Help ")
        .title_bottom(footer.right_aligned())
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(dialog).inner(Margin::new(1, 1));
    f.render_widget(block, dialog);

    if two_columns {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .spacing(2)
                .areas(inner);
        f.render_widget(Paragraph::new(global), left);
        f.render_widget(Paragraph::new(tab), right);
    } else {
        // Tab keys first: they are what the user is most likely looking for.
        let mut lines = tab;
        lines.push(Line::raw(""));
        lines.extend(global);
        f.render_widget(Paragraph::new(lines), inner);
    }
}

fn key_style() -> Style {
    Style::default().fg(Color::Yellow).bold()
}

fn tab_section(tab: ActiveTab) -> Vec<Line<'static>> {
    let title = format!("{} tab", tab.title());
    let bindings = tab_bindings(tab);
    if bindings.is_empty() {
        vec![
            Line::styled(title, Style::default().bold().underlined()),
            Line::styled("(no tab-specific keys)", Style::default().italic()),
        ]
    } else {
        section(&title, bindings)
    }
}

/// Heading plus one line per binding: key label padded to a column, then
/// the description.
fn section(title: &str, bindings: &[KeyBinding]) -> Vec<Line<'static>> {
    let key_w = bindings
        .iter()
        .map(|b| b.label.chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = vec![Line::styled(
        title.to_string(),
        Style::default().bold().underlined(),
    )];
    for b in bindings {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<key_w$}", b.label), key_style()),
            Span::raw("  "),
            Span::raw(b.desc),
        ]));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn draw(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render_help(f, f.area(), app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn lists_every_global_and_tab_binding_when_wide() {
        for tab in ActiveTab::ALL {
            let mut app = App::new("h".into(), 1);
            app.active_tab = *tab;
            let text = draw(&app, 120, 40);
            for b in GLOBAL_BINDINGS.iter().chain(tab_bindings(*tab)) {
                assert!(
                    text.contains(b.desc),
                    "{tab:?} missing {:?}\n{text}",
                    b.desc
                );
            }
            assert!(text.contains(&format!("{} tab", tab.title())), "{text}");
        }
    }

    #[test]
    fn tiny_areas_do_not_panic() {
        let app = App::new("h".into(), 1);
        for (w, h) in [(0, 0), (1, 1), (3, 2), (4, 3), (10, 4), (20, 6)] {
            draw(&app, w, h);
        }
    }
}
