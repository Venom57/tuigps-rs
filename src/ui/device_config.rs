use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Min(0),    // config controls
        Constraint::Length(10), // output log
    ])
    .split(area);

    // Controls placeholder
    let block = Block::bordered().title(" Device Configuration (u-blox) ");
    let inner = block.inner(chunks[0]);
    f.render_widget(block, chunks[0]);

    let lines = vec![
        Line::raw("Arrow keys to navigate, Enter to activate"),
        Line::raw(""),
        Line::raw("Nav Rate:      [1 Hz]"),
        Line::raw("Power Mode:    [Full Power]"),
        Line::raw("PPS Frequency: [1 Hz]"),
        Line::raw("Serial Speed:  [9600]"),
        Line::raw(""),
        Line::raw("Constellations: GPS GLONASS Galileo BeiDou SBAS QZSS"),
        Line::raw(""),
        Line::raw("[Save Config]  [Cold Boot]  [Clock Sync]"),
    ];

    f.render_widget(Paragraph::new(lines), inner);

    // Output log
    let log_block = Block::bordered().title(" Output ");
    let log_inner = log_block.inner(chunks[1]);
    f.render_widget(log_block, chunks[1]);

    let log_lines: Vec<Line> = app
        .device_config_log
        .iter()
        .rev()
        .take(log_inner.height as usize)
        .map(|s| Line::raw(s.clone()))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    f.render_widget(Paragraph::new(log_lines), log_inner);
}
