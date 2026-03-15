use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, CoordFormat, SettingsField, UnitSystem};

pub fn render_settings(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Clear, area);

    let dialog_area = centered_rect(62, 20, area);
    let block = Block::bordered()
        .title(" Settings ")
        .style(Style::default().bg(Color::DarkGray));
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

    let lines = vec![
        Line::raw(""),
        Line::styled(
            "  Connection",
            Style::default().fg(Color::White).bold(),
        ),
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
        Line::styled(
            "  Display",
            Style::default().fg(Color::White).bold(),
        ),
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
        Line::styled(
            "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            Style::default().fg(Color::DarkGray),
        ),
        Line::styled(
            "  Up/Down  navigate    Enter  edit / cycle value",
            Style::default().fg(Color::DarkGray),
        ),
        Line::styled(
            "  \u{2190}/\u{2192}    cycle value  Esc    close",
            Style::default().fg(Color::DarkGray),
        ),
        Line::styled(
            "  Ctrl+S   apply & reconnect to gpsd",
            Style::default().fg(Color::DarkGray),
        ),
    ];

    f.render_widget(Paragraph::new(lines), inner);
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

    let hint_text = if selected {
        format!("  ({})", hint)
    } else {
        String::new()
    };

    Line::from(vec![
        Span::styled(marker, Style::default().fg(Color::Yellow)),
        Span::raw(format!("{:<8}", format!("{}:", label))),
        Span::styled(display, value_style),
        Span::styled(hint_text, Style::default().fg(Color::DarkGray)),
    ])
}

fn render_cycle_field<'a>(
    label: &str,
    value: &str,
    selected: bool,
    description: &str,
) -> Line<'a> {
    let marker = if selected { "> " } else { "  " };
    let value_style = if selected {
        Style::default().fg(Color::Yellow).bold()
    } else {
        Style::default().fg(Color::Gray)
    };
    let arrows = if selected { " \u{25c0} \u{25b6} " } else { "" };

    let desc_text = format!("  {}", description);

    Line::from(vec![
        Span::styled(marker, Style::default().fg(Color::Yellow)),
        Span::raw(format!("{:<8}", format!("{}:", label))),
        Span::styled(value.to_string(), value_style),
        Span::styled(arrows, Style::default().fg(Color::DarkGray)),
        Span::styled(desc_text, Style::default().fg(Color::DarkGray)),
    ])
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
