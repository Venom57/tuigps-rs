use ratatui::prelude::*;

use crate::app::App;
use crate::ui::*;

pub fn render_dashboard(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Ratio(3, 10),
        Constraint::Ratio(4, 10),
        Constraint::Ratio(3, 10),
    ])
    .split(area);

    let row1 = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(rows[0]);

    let row2 = Layout::horizontal([
        Constraint::Ratio(2, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(rows[1]);

    let row3 = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(rows[2]);

    position::render(f, row1[0], app);
    fix::render(f, row1[1], app);
    velocity::render(f, row1[2], app);
    sky_plot::render(f, row2[0], app);
    signal_chart::render(f, row2[1], app);
    error_panel::render(f, row3[0], app);
    device_panel::render(f, row3[1], app);
    time_panel::render(f, row3[2], app, false);
}
