use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{ActiveTab, App};

pub mod constellation;
pub mod dashboard;
pub mod device_config;
pub mod device_panel;
pub mod error_panel;
pub mod fix;
pub mod nmea_viewer;
pub mod position;
pub mod satellite_table;
pub mod settings;
pub mod signal_chart;
pub mod sky_plot;
pub mod status_bar;
pub mod time_panel;
pub mod velocity;

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(1),   // tab bar
        Constraint::Min(0),     // main content
        Constraint::Length(1),  // status bar
    ])
    .split(f.area());

    render_tab_bar(f, chunks[0], app);

    match app.active_tab {
        ActiveTab::Dashboard => dashboard::render_dashboard(f, chunks[1], app),
        ActiveTab::Satellites => render_satellites(f, chunks[1], app),
        ActiveTab::Timing => render_timing(f, chunks[1], app),
        ActiveTab::Device => device_config::render(f, chunks[1], app),
        ActiveTab::Nmea => nmea_viewer::render(f, chunks[1], app),
    }

    status_bar::render_status_bar(f, chunks[2], app);

    if app.show_settings {
        settings::render_settings(f, f.area(), app);
    }
}

fn render_tab_bar(f: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = ActiveTab::ALL
        .iter()
        .map(|t| {
            let style = if *t == app.active_tab {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::styled(t.title(), style)
        })
        .collect();

    let tabs = Tabs::new(titles)
        .select(
            ActiveTab::ALL
                .iter()
                .position(|t| *t == app.active_tab)
                .unwrap_or(0),
        )
        .divider("|");

    f.render_widget(tabs, area);
}

fn render_satellites(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(10), // constellation summary
        Constraint::Min(0),    // satellite table
    ])
    .split(area);

    constellation::render(f, chunks[0], app);
    satellite_table::render(f, chunks[1], app);
}

fn render_timing(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Ratio(1, 2),
        Constraint::Ratio(1, 2),
    ])
    .split(area);

    time_panel::render(f, chunks[0], app, true);
    device_panel::render(f, chunks[1], app);
}
