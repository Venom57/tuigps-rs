use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, CoordFormat, SETTINGS_BINDINGS, SettingsField, UnitSystem};
use crate::gps_logger::LogMode;

pub fn render_settings(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Clear, area);

    let dialog_area = centered_rect(62, 22, area);
    // Terminal-default background with a colored border: text styles below
    // use the default foreground (or Yellow accents) so every line is
    // readable on both dark and light themes. Only the divider is DarkGray.
    let block = Block::bordered()
        .title(" Settings ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let sel = app.settings_field;
    let editing = app.settings_editing;

    let units_desc = match app.units {
        UnitSystem::Metric => "km/h, meters",
        UnitSystem::Imperial => "mph, feet",
        UnitSystem::Nautical => "knots, feet",
    };

    let coord_desc = match app.coord_format {
        CoordFormat::DD => "51.5074000\u{00b0} N",
        CoordFormat::DMS => "51\u{00b0} 30' 26.640\" N",
        CoordFormat::DDM => "51\u{00b0} 30.444000' N",
    };

    let log_desc = match app.log_mode {
        LogMode::Gpx => "track for mapping tools",
        LogMode::Csv => "spreadsheet, one row per fix",
        LogMode::Both => ".gpx + .csv, same name",
    };
    // Long paths keep their tail (the part that identifies the directory)
    let log_dir = tail_ellipsis(
        &if editing && sel == SettingsField::LogDir {
            app.settings_edit_buf.clone()
        } else {
            app.log_dir.to_string_lossy().into_owned()
        },
        MAX_DIR_CHARS,
    );

    let lines = vec![
        Line::raw(""),
        Line::styled("  Connection", Style::default().bold().underlined()),
        render_field(
            "Host",
            if editing && sel == SettingsField::Host {
                &app.settings_edit_buf
            } else {
                &app.host
            },
            sel == SettingsField::Host,
            editing && sel == SettingsField::Host,
            "gpsd hostname or IP",
        ),
        render_field(
            "Port",
            &if editing && sel == SettingsField::Port {
                app.settings_edit_buf.clone()
            } else {
                app.port.to_string()
            },
            sel == SettingsField::Port,
            editing && sel == SettingsField::Port,
            "gpsd port (default 2947)",
        ),
        Line::raw(""),
        Line::styled("  Display", Style::default().bold().underlined()),
        render_cycle_field(
            "Units",
            app.units.as_str(),
            sel == SettingsField::Units,
            units_desc,
        ),
        render_cycle_field(
            "Coords",
            app.coord_format.as_str().to_uppercase().as_str(),
            sel == SettingsField::CoordFormat,
            coord_desc,
        ),
        Line::raw(""),
        Line::styled("  Logging (l)", Style::default().bold().underlined()),
        render_cycle_field(
            "Format",
            app.log_mode.label(),
            sel == SettingsField::LogFormat,
            log_desc,
        ),
        render_field(
            "Dir",
            &log_dir,
            sel == SettingsField::LogDir,
            editing && sel == SettingsField::LogDir,
            "existing directory, ~ ok",
        ),
        Line::raw(""),
        render_save_button(sel == SettingsField::Save, app.config_path.is_some()),
        message_line(&app.settings_message),
        Line::styled(
            "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            Style::default().fg(Color::DarkGray),
        ),
    ]
    .into_iter()
    .chain(help_lines())
    .collect::<Vec<_>>();

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_save_button<'a>(selected: bool, persists: bool) -> Line<'a> {
    let marker = if selected { "> " } else { "  " };
    let style = if selected {
        Style::default().fg(Color::Black).bg(Color::Yellow).bold()
    } else {
        Style::default().bold()
    };
    let note = if persists {
        "  apply, reconnect if needed, save to config"
    } else {
        "  apply (config file unavailable)"
    };
    Line::from(vec![
        Span::styled(marker, Style::default().fg(Color::Yellow)),
        Span::styled("[ Save ]", style),
        Span::styled(note, Style::default().italic()),
    ])
}

/// Validation error (e.g. invalid port) shown inside the dialog, since the
/// dialog hides the status bar.
fn message_line<'a>(message: &str) -> Line<'a> {
    if message.is_empty() {
        Line::raw("")
    } else {
        Line::styled(
            format!("  ! {}", message),
            Style::default().fg(Color::Red).bold(),
        )
    }
}

/// Key help generated from `SETTINGS_BINDINGS`, two keys per line.
fn help_lines() -> Vec<Line<'static>> {
    let key_w = SETTINGS_BINDINGS
        .iter()
        .map(|b| b.label.chars().count())
        .max()
        .unwrap_or(0);
    SETTINGS_BINDINGS
        .chunks(2)
        .map(|pair| {
            let mut spans = vec![Span::raw("  ")];
            for (i, b) in pair.iter().enumerate() {
                spans.push(Span::styled(
                    format!("{:<key_w$}", b.label),
                    Style::default().fg(Color::Yellow).bold(),
                ));
                spans.push(Span::raw(" "));
                if i == 0 && pair.len() > 1 {
                    spans.push(Span::raw(format!("{:<20}", b.desc)));
                } else {
                    spans.push(Span::raw(b.desc));
                }
            }
            Line::from(spans)
        })
        .collect()
}

fn render_field<'a>(
    label: &str,
    value: &str,
    selected: bool,
    editing: bool,
    hint: &str,
) -> Line<'a> {
    let marker = if selected { "> " } else { "  " };
    let value_style = if editing {
        Style::default().fg(Color::Yellow).bold().underlined()
    } else if selected {
        Style::default().bold()
    } else {
        Style::default()
    };

    let display = if editing {
        format!("{}_", value)
    } else {
        value.to_string()
    };

    let hint_text = if selected {
        format!("  ({})", hint)
    } else {
        String::new()
    };

    Line::from(vec![
        Span::styled(marker, Style::default().fg(Color::Yellow)),
        Span::raw(format!("{:<8}", format!("{}:", label))),
        Span::styled(display, value_style),
        Span::styled(hint_text, Style::default().italic()),
    ])
}

fn render_cycle_field<'a>(label: &str, value: &str, selected: bool, description: &str) -> Line<'a> {
    let marker = if selected { "> " } else { "  " };
    let value_style = if selected {
        Style::default().fg(Color::Yellow).bold()
    } else {
        Style::default()
    };
    let arrows = if selected { " \u{25c0} \u{25b6} " } else { "" };

    let desc_text = format!("  {}", description);

    Line::from(vec![
        Span::styled(marker, Style::default().fg(Color::Yellow)),
        Span::raw(format!("{:<8}", format!("{}:", label))),
        Span::styled(value.to_string(), value_style),
        Span::styled(arrows, Style::default().fg(Color::Yellow)),
        Span::styled(desc_text, Style::default().italic()),
    ])
}

/// Characters of the log directory shown before it is cut from the left.
const MAX_DIR_CHARS: usize = 24;

/// `s` unchanged if it fits in `max` chars, else `…` plus its last chars.
fn tail_ellipsis(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let tail: String = s.chars().skip(n - (max - 1)).collect();
    format!("\u{2026}{tail}")
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_ellipsis_keeps_end_of_long_paths() {
        assert_eq!(tail_ellipsis("/short", 10), "/short");
        assert_eq!(tail_ellipsis("/home/u/tracks", 14), "/home/u/tracks");
        assert_eq!(tail_ellipsis("/home/u/tracks", 8), "\u{2026}/tracks");
        assert_eq!(tail_ellipsis("/home/u/tracks", 8).chars().count(), 8);
        assert_eq!(
            tail_ellipsis("/h/\u{00e9}t\u{00e9}s", 4),
            "\u{2026}t\u{00e9}s"
        );
    }
}
