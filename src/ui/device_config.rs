use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{
    App, DeviceConfigState, DeviceConfirm, GNSS_NAMES_CONFIG, NAV_RATES, POWER_MODES,
    PPS_FREQUENCIES, SERIAL_SPEEDS,
};

/// Index of the "Save Config" control.
const SAVE_CONTROL: usize = 10;

/// Inner width from which constellations get their own column.
const TWO_COLUMN_MIN: u16 = 56;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Min(0),     // config controls
        Constraint::Length(10), // output log
    ])
    .split(area);

    render_controls(f, chunks[0], app);
    render_log(f, chunks[1], app);

    if let Some(confirm) = &app.device_config.confirm {
        render_confirm(f, area, confirm);
    }
}

fn render_controls(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Device Configuration (u-blox) ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let dc = &app.device_config;
    let sel = dc.selected_control;

    let legend = Line::from(vec![
        Span::styled(" *", pending_style()),
        Span::raw(" changed, Enter to apply   "),
        Span::styled("\u{2713}", Style::default().fg(Color::Green)),
        Span::raw(" sent this session   "),
        Span::styled("(not read from receiver)", Style::default().italic()),
    ]);

    let mut settings = vec![
        control_line(dc, 0, "Nav Rate", NAV_RATES[dc.nav_rate_idx].0),
        control_line(dc, 1, "Power Mode", POWER_MODES[dc.power_mode_idx].0),
        control_line(
            dc,
            2,
            "Serial Speed",
            &format!("{} baud", SERIAL_SPEEDS[dc.serial_speed_idx]),
        ),
        control_line(
            dc,
            3,
            "PPS Frequency",
            PPS_FREQUENCIES[dc.pps_frequency_idx].0,
        ),
        Line::raw(""),
    ];
    let save_style = if sel == SAVE_CONTROL {
        Style::default().fg(Color::Yellow).bold()
    } else {
        Style::default()
    };
    settings.push(Line::from(vec![
        marker(sel == SAVE_CONTROL),
        Span::styled("[Save Config]", save_style),
        sent_mark(dc, SAVE_CONTROL),
    ]));

    let mut gnss = vec![Line::styled(" Constellations:", Style::default().bold())];
    for (i, name) in GNSS_NAMES_CONFIG.iter().enumerate() {
        let control = i + 4;
        let enabled = dc.gnss_enabled[i];
        let pending = dc.is_pending(control);
        let state = if enabled { "[x]" } else { "[ ]" };
        let color = if enabled { Color::Green } else { Color::Red };
        let name_style = if sel == control {
            Style::default().bold()
        } else {
            Style::default()
        };
        gnss.push(Line::from(vec![
            marker(sel == control),
            Span::styled(state, Style::default().fg(color)),
            if pending {
                Span::styled("*", pending_style())
            } else {
                Span::raw(" ")
            },
            Span::styled(*name, name_style),
            sent_mark(dc, control),
        ]));
    }

    let [legend_area, body] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(inner);
    f.render_widget(Paragraph::new(legend), legend_area);

    if inner.width >= TWO_COLUMN_MIN {
        let [left, right] =
            Layout::horizontal([Constraint::Min(34), Constraint::Length(22)]).areas(body);
        f.render_widget(Paragraph::new(settings), left);
        f.render_widget(Paragraph::new(gnss), right);
    } else {
        settings.push(Line::raw(""));
        settings.extend(gnss);
        f.render_widget(Paragraph::new(settings), body);
    }
}

fn pending_style() -> Style {
    Style::default().fg(Color::Yellow).bold()
}

fn marker(selected: bool) -> Span<'static> {
    Span::styled(
        if selected { "> " } else { "  " },
        Style::default().fg(Color::Yellow),
    )
}

/// "✓" after a control applied this session (and unchanged since).
fn sent_mark(dc: &DeviceConfigState, control: usize) -> Span<'static> {
    if dc.sent.get(control).copied().unwrap_or(false) && !dc.is_pending(control) {
        Span::styled(" \u{2713}", Style::default().fg(Color::Green))
    } else {
        Span::raw("")
    }
}

fn control_line<'a>(dc: &DeviceConfigState, control: usize, label: &str, value: &str) -> Line<'a> {
    let selected = dc.selected_control == control;
    let pending = dc.is_pending(control);
    let value_style = if pending {
        pending_style().italic()
    } else if selected {
        Style::default().fg(Color::Yellow).bold()
    } else {
        Style::default()
    };
    let arrows = if selected { " < >" } else { "" };

    Line::from(vec![
        marker(selected),
        Span::raw(format!("{:<15}", format!("{}:", label))),
        Span::styled(value.to_string(), value_style),
        if pending {
            Span::styled("*", pending_style())
        } else {
            Span::raw("")
        },
        sent_mark(dc, control),
        Span::styled(arrows, Style::default().fg(Color::Yellow)),
    ])
}

/// Confirmation box over the Device tab. Destructive changes get a red
/// border, a WARNING title and the explanation.
fn render_confirm(f: &mut Frame, area: Rect, confirm: &DeviceConfirm) {
    let destructive = confirm.destructive();
    let accent = if destructive {
        Color::Red
    } else {
        Color::Yellow
    };
    let title = if destructive {
        " WARNING: confirm device change "
    } else {
        " Confirm device change "
    };

    let mut lines = vec![Line::styled(
        confirm.prompt.clone(),
        Style::default().bold(),
    )];
    if let Some(warning) = &confirm.warning {
        lines.push(Line::styled(
            warning.clone(),
            Style::default().fg(Color::Red),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(" y ", Style::default().fg(Color::Black).bg(accent).bold()),
        Span::raw(" apply    "),
        Span::styled("any other key", Style::default().bold()),
        Span::raw(" cancel"),
    ]));

    let width = area.width.min(60);
    let text_w = width.saturating_sub(4).max(1) as usize;
    // Rows needed with wrapping: estimate from character counts.
    let rows: usize = lines
        .iter()
        .map(|l| l.width().max(1).div_ceil(text_w))
        .sum();
    let height = (rows as u16 + 2).min(area.height);
    let dialog = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );

    f.render_widget(Clear, dialog);
    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(accent).bold());
    let inner = block.inner(dialog).inner(Margin::new(1, 0));
    f.render_widget(block, dialog);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
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
