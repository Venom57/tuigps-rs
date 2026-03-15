use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{
    App, GNSS_NAMES_CONFIG, NAV_RATES, POWER_MODES, PPS_FREQUENCIES,
    SERIAL_SPEEDS,
};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Min(0),     // config controls
        Constraint::Length(10), // output log
    ])
    .split(area);

    render_controls(f, chunks[0], app);
    render_log(f, chunks[1], app);
}

fn render_controls(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Device Configuration (u-blox) ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let dc = &app.device_config;
    let sel = dc.selected_control;

    let mut lines = vec![
        Line::styled(
            " Up/Down: select  Left/Right: adjust  Enter: apply",
            Style::default().fg(Color::DarkGray),
        ),
        Line::raw(""),
    ];

    // Control 0: Nav rate
    lines.push(control_line(
        "Nav Rate",
        NAV_RATES[dc.nav_rate_idx].0,
        sel == 0,
    ));

    // Control 1: Power mode
    lines.push(control_line(
        "Power Mode",
        POWER_MODES[dc.power_mode_idx].0,
        sel == 1,
    ));

    // Control 2: Serial speed
    lines.push(control_line(
        "Serial Speed",
        &format!("{} baud", SERIAL_SPEEDS[dc.serial_speed_idx]),
        sel == 2,
    ));

    // Control 3: PPS frequency
    lines.push(control_line(
        "PPS Frequency",
        PPS_FREQUENCIES[dc.pps_frequency_idx].0,
        sel == 3,
    ));

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        " Constellations:",
        Style::default().fg(Color::White).bold(),
    ));

    // Controls 4-9: GNSS toggles
    for (i, name) in GNSS_NAMES_CONFIG.iter().enumerate() {
        let enabled = dc.gnss_enabled[i];
        let marker = if sel == i + 4 { "> " } else { "  " };
        let state = if enabled { "[x]" } else { "[ ]" };
        let color = if enabled { Color::Green } else { Color::Red };
        let name_style = if sel == i + 4 {
            Style::default().fg(Color::White).bold()
        } else {
            Style::default().fg(Color::Gray)
        };

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Yellow)),
            Span::styled(state, Style::default().fg(color)),
            Span::raw(" "),
            Span::styled(*name, name_style),
        ]));
    }

    lines.push(Line::raw(""));

    // Control 10: Save config
    let save_style = if sel == 10 {
        Style::default().fg(Color::Yellow).bold()
    } else {
        Style::default().fg(Color::Gray)
    };
    let save_marker = if sel == 10 { "> " } else { "  " };
    lines.push(Line::from(vec![
        Span::styled(save_marker, Style::default().fg(Color::Yellow)),
        Span::styled("[Save Config]", save_style),
    ]));

    f.render_widget(Paragraph::new(lines), inner);
}

fn control_line<'a>(label: &str, value: &str, selected: bool) -> Line<'a> {
    let marker = if selected { "> " } else { "  " };
    let value_style = if selected {
        Style::default().fg(Color::Yellow).bold()
    } else {
        Style::default().fg(Color::White)
    };
    let arrows = if selected { " < > " } else { "" };

    Line::from(vec![
        Span::styled(marker.to_string(), Style::default().fg(Color::Yellow)),
        Span::raw(format!("{:<16}", format!("{}:", label))),
        Span::styled(value.to_string(), value_style),
        Span::styled(arrows.to_string(), Style::default().fg(Color::DarkGray)),
    ])
}

fn render_log(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Output ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let log_lines: Vec<Line> = app
        .device_config
        .output_log
        .iter()
        .rev()
        .take(inner.height as usize)
        .map(|s| Line::raw(s.clone()))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    f.render_widget(Paragraph::new(log_lines), inner);
}
