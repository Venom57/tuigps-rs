use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, SettingsField};

pub fn render_settings(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Clear, area);

    let dialog_area = centered_rect(60, 16, area);
    let block = Block::bordered()
        .title(" Settings ")
        .style(Style::default().bg(Color::DarkGray));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let sel = app.settings_field;
    let editing = app.settings_editing;

    let lines = vec![
        Line::raw(""),
        render_field(
            "Host",
            if editing && sel == SettingsField::Host {
                &app.settings_edit_buf
            } else {
                &app.host
            },
            sel == SettingsField::Host,
            editing && sel == SettingsField::Host,
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
        ),
        Line::raw(""),
        render_cycle_field(
            "Units",
            app.units.as_str(),
            sel == SettingsField::Units,
        ),
        render_cycle_field(
            "Coords",
            app.coord_format.as_str(),
            sel == SettingsField::CoordFormat,
        ),
        Line::raw(""),
        Line::styled(
            "  Up/Down: navigate  Enter: edit/cycle  Esc: close",
            Style::default().fg(Color::DarkGray),
        ),
        Line::styled(
            "  Ctrl+S: apply & reconnect",
            Style::default().fg(Color::DarkGray),
        ),
    ];

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_field<'a>(label: &str, value: &str, selected: bool, editing: bool) -> Line<'a> {
    let marker = if selected { "> " } else { "  " };
    let value_style = if editing {
        Style::default().fg(Color::Yellow).bg(Color::Black).bold()
    } else if selected {
        Style::default().fg(Color::White).bold()
    } else {
        Style::default().fg(Color::Gray)
    };

    let display = if editing {
        format!("{}_", value)
    } else {
        value.to_string()
    };

    Line::from(vec![
        Span::styled(marker, Style::default().fg(Color::Yellow)),
        Span::raw(format!("{:<8}", format!("{}:", label))),
        Span::styled(display, value_style),
    ])
}

fn render_cycle_field<'a>(label: &str, value: &str, selected: bool) -> Line<'a> {
    let marker = if selected { "> " } else { "  " };
    let value_style = if selected {
        Style::default().fg(Color::Yellow).bold()
    } else {
        Style::default().fg(Color::Gray)
    };
    let arrows = if selected { " <  > " } else { "" };

    Line::from(vec![
        Span::styled(marker, Style::default().fg(Color::Yellow)),
        Span::raw(format!("{:<8}", format!("{}:", label))),
        Span::styled(value.to_string(), value_style),
        Span::styled(arrows, Style::default().fg(Color::DarkGray)),
    ])
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
