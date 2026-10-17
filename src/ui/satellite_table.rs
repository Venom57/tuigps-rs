use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::{gnss_color, gnss_name};
use crate::formatting::fmt;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let mut sats: Vec<_> = app.gps_data.satellites.iter().collect();
    sats.sort_unstable_by_key(|s| (s.gnssid, s.prn, s.sigid));

    // Apply scroll offset
    let visible_height = area.height.saturating_sub(3) as usize; // 2 borders + header
    let max_offset = sats.len().saturating_sub(visible_height);
    let offset = app.sat_scroll_offset.min(max_offset);
    let visible_sats = &sats[offset..sats.len().min(offset + visible_height)];

    let rows: Vec<Row> = visible_sats
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
                Cell::from(fmt(sat.elevation, 0, "\u{00b0}")),
                Cell::from(fmt(sat.azimuth, 0, "\u{00b0}")),
                Cell::from(fmt(sat.snr, 1, "")).style(Style::default().fg(snr_color)),
                Cell::from(if sat.used { "*" } else { "" })
                    .style(Style::default().fg(Color::Green)),
                Cell::from(format!("{}", sat.sigid)),
                health_cell(sat.health),
            ])
        })
        .collect();

    let scroll_info = if sats.len() > visible_height {
        format!(
            " Satellites [{}-{}/{}] ",
            offset + 1,
            (offset + visible_sats.len()).min(sats.len()),
            sats.len()
        )
    } else {
        format!(" Satellites ({}) ", sats.len())
    };

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
        .block(Block::bordered().title(scroll_info));

    f.render_widget(table, area);
}

/// gpsd health: 0 = unknown, 1 = OK, 2 = unhealthy.
fn health_cell(health: u8) -> Cell<'static> {
    match health {
        0 => Cell::from("---").style(Style::default().fg(Color::DarkGray)),
        1 => Cell::from("OK").style(Style::default().fg(Color::Green)),
        2 => Cell::from("BAD").style(Style::default().fg(Color::Red).bold()),
        n => Cell::from(n.to_string()),
    }
}
