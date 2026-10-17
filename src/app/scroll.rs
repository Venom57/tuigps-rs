//! Scroll requests shared by the NMEA and Satellites tabs, plus
//! satellite-table scrolling.

use super::{ActiveTab, App};

/// Scroll request, independent of which list is scrolled. `Up` moves
/// towards the start of the list (older NMEA lines / first satellite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scroll {
    Up(usize),
    Down(usize),
    Top,
    Bottom,
}

/// Page size used before the first render has recorded the view height.
const DEFAULT_PAGE: usize = 20;

impl App {
    pub(super) fn page_size(&self) -> usize {
        let rows = match self.active_tab {
            ActiveTab::Nmea => self.nmea_view_rows.get(),
            _ => self.sat_view_rows.get(),
        };
        if rows == 0 {
            DEFAULT_PAGE
        } else {
            rows.saturating_sub(1).max(1)
        }
    }

    /// Largest useful satellite-table offset (last row at the bottom).
    pub fn sat_max_scroll(&self) -> usize {
        self.gps_data
            .satellites
            .len()
            .saturating_sub(self.sat_view_rows.get())
    }

    pub fn scroll_satellites(&mut self, scroll: Scroll) {
        let max = self.sat_max_scroll();
        let cur = self.sat_scroll_offset.min(max);
        self.sat_scroll_offset = match scroll {
            Scroll::Up(n) => cur.saturating_sub(n),
            Scroll::Down(n) => cur.saturating_add(n).min(max),
            Scroll::Top => 0,
            Scroll::Bottom => max,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_util::*;
    use crate::ui;
    use crossterm::event::KeyCode;
    use ratatui::Terminal;

    // === Satellites tab ===

    /// Satellites tab with `n` satellites and an 8-row view.
    fn sat_app(n: u16) -> App {
        let mut app = new_app();
        app.active_tab = ActiveTab::Satellites;
        app.sat_view_rows.set(8);
        app.gps_data.satellites = (0..n)
            .map(|i| crate::data_model::SatelliteInfo {
                prn: i + 1,
                gnssid: 0,
                svid: i + 1,
                elevation: 10.0,
                azimuth: 10.0,
                snr: 30.0,
                used: false,
                sigid: 0,
                health: 1,
                freqid: None,
            })
            .collect();
        app
    }

    #[test]
    fn test_satellite_scroll() {
        let mut app = sat_app(20);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.sat_scroll_offset, 1);

        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.sat_scroll_offset, 0);

        // Can't scroll below 0
        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.sat_scroll_offset, 0);

        app.handle_event(key_event(KeyCode::PageDown));
        assert_eq!(app.sat_scroll_offset, 7);
        app.handle_event(key_event(KeyCode::End));
        assert_eq!(app.sat_scroll_offset, 12);
        app.handle_event(key_event(KeyCode::PageUp));
        assert_eq!(app.sat_scroll_offset, 5);
        app.handle_event(key_event(KeyCode::Home));
        assert_eq!(app.sat_scroll_offset, 0);
    }

    #[test]
    fn test_satellite_scroll_clamped() {
        let mut app = sat_app(10);
        for _ in 0..20 {
            app.handle_event(key_event(KeyCode::Down));
        }
        assert_eq!(app.sat_scroll_offset, 2);
        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.sat_scroll_offset, 1);

        // List shrank (satellites set): the old offset is clamped first.
        app.sat_scroll_offset = 2;
        app.gps_data.satellites.truncate(9);
        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.sat_scroll_offset, 0);

        // Fewer satellites than rows.
        let mut app = sat_app(3);
        app.handle_event(key_event(KeyCode::End));
        assert_eq!(app.sat_scroll_offset, 0);
    }

    #[test]
    fn test_render_records_view_rows() {
        use ratatui::backend::TestBackend;
        let mut app = sat_app(40);
        app.sat_view_rows.set(0);
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| ui::render(f, &app)).unwrap();
        // 24 rows - tab bar - summary - status - 2 borders - header
        assert_eq!(app.sat_view_rows.get(), 18);
        app.handle_event(key_event(KeyCode::End));
        assert_eq!(app.sat_scroll_offset, 22);

        app.active_tab = ActiveTab::Nmea;
        term.draw(|f| ui::render(f, &app)).unwrap();
        // 24 rows - tab bar - status - 2 borders
        assert_eq!(app.nmea_view_rows.get(), 20);
    }
}
