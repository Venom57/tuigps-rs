use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::{gnss_color, gnss_name};
use crate::formatting::fmt;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let mut sats: Vec<_> = app.gps_data.satellites.iter().collect();
    sats.sort_by(|a, b| a.gnssid.cmp(&b.gnssid).then(a.prn.cmp(&b.prn)));

    let rows: Vec<Row> = sats
        .iter()
        .map(|sat| {
            let color = gnss_color(sat.gnssid);
            let snr_color = if sat.snr.is_finite() {
                if sat.snr > 30.0 {
                    Color::Green
                } else if sat.snr > 20.0 {
                    Color::Yellow
                } else {
                    Color::Red
                }
            } else {
                Color::DarkGray
            };

            Row::new(vec![
                Cell::from(gnss_name(sat.gnssid)).style(Style::default().fg(color)),
                Cell::from(format!("{}", sat.prn)),
                Cell::from(format!("{}", sat.svid)),
                Cell::from(fmt(sat.elevation, 0, "°")),
                Cell::from(fmt(sat.azimuth, 0, "°")),
                Cell::from(fmt(sat.snr, 1, "")).style(Style::default().fg(snr_color)),
                Cell::from(if sat.used { "*" } else { "" })
                    .style(Style::default().fg(Color::Green)),
                Cell::from(format!("{}", sat.sigid)),
                Cell::from(if sat.health == 1 {
                    "OK".to_string()
                } else {
                    format!("{}", sat.health)
                })
                .style(if sat.health == 1 {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Red)
                }),
            ])
        })
        .collect();

    let header = Row::new(["GNSS", "PRN", "SV", "El", "Az", "SNR", "U", "Sig", "Health"])
        .style(Style::default().bold());

    let widths = [
        Constraint::Length(8),
        Constraint::Length(5),
        Constraint::Length(4),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(2),
        Constraint::Length(4),
        Constraint::Length(7),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::bordered().title(" Satellites "));

    f.render_widget(table, area);
}
