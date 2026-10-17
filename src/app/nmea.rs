//! NMEA tab: sentence buffer, pause buffer, type filter and scrolling.

use std::collections::VecDeque;

use super::App;
use super::scroll::Scroll;

// NMEA filter types to cycle through
const NMEA_FILTERS: &[&str] = &["", "GGA", "RMC", "GSA", "GSV", "VTG", "GLL", "ZDA"];

/// Maximum NMEA lines kept in the buffer (and in the pause buffer).
pub const NMEA_BUFFER_MAX: usize = 1000;

impl App {
    fn nmea_matches(&self, sentence: &str) -> bool {
        self.nmea_filter.is_empty()
            || crate::ui::nmea_viewer::nmea_type(sentence) == self.nmea_filter
    }

    /// Number of buffered NMEA lines passing the current filter.
    pub fn nmea_visible_count(&self) -> usize {
        if self.nmea_filter.is_empty() {
            self.nmea_buffer.len()
        } else {
            self.nmea_buffer
                .iter()
                .filter(|s| self.nmea_matches(s))
                .count()
        }
    }

    /// Largest useful NMEA scroll offset (top of the buffer in view).
    pub fn nmea_max_scroll(&self) -> usize {
        self.nmea_visible_count()
            .saturating_sub(self.nmea_view_rows.get())
    }

    /// True while the NMEA view shows the newest lines as they arrive.
    pub fn nmea_following(&self) -> bool {
        !self.nmea_paused && self.nmea_scroll_offset == 0
    }

    pub fn scroll_nmea(&mut self, scroll: Scroll) {
        let max = self.nmea_max_scroll();
        // Start from the effective (clamped) offset so overshoot from
        // earlier never needs undoing.
        let cur = self.nmea_scroll_offset.min(max);
        self.nmea_scroll_offset = match scroll {
            Scroll::Up(n) => cur.saturating_add(n).min(max),
            Scroll::Down(n) => cur.saturating_sub(n),
            Scroll::Top => max,
            Scroll::Bottom => 0,
        };
    }

    /// Append a sentence to the live NMEA buffer. While scrolled back, the
    /// bottom-relative offset grows with each visible line so the view
    /// stays on the same sentences.
    pub fn push_nmea(&mut self, sentence: String) {
        let visible = self.nmea_matches(&sentence);
        push_capped(&mut self.nmea_buffer, sentence);
        if self.nmea_scroll_offset > 0 {
            if visible {
                self.nmea_scroll_offset += 1;
            }
            // Lines evicted from the front can push the anchor off the top.
            self.nmea_scroll_offset = self.nmea_scroll_offset.min(self.nmea_max_scroll());
        }
    }

    /// A sentence from gpsd: shown live, or held while paused.
    pub(super) fn receive_nmea(&mut self, sentence: String) {
        if self.nmea_paused {
            push_capped(&mut self.nmea_pause_buffer, sentence);
        } else {
            self.push_nmea(sentence);
        }
    }

    pub(super) fn clear_nmea(&mut self) {
        self.nmea_buffer.clear();
        self.nmea_pause_buffer.clear();
        self.nmea_scroll_offset = 0;
        self.set_status("NMEA buffer cleared");
    }

    pub(super) fn toggle_nmea_pause(&mut self) {
        if self.nmea_paused {
            // Unpause: merge pause buffer into main buffer
            while let Some(s) = self.nmea_pause_buffer.pop_front() {
                push_capped(&mut self.nmea_buffer, s);
            }
        }
        self.nmea_paused = !self.nmea_paused;
        self.nmea_scroll_offset = 0;
    }

    pub(super) fn cycle_nmea_filter(&mut self) {
        self.nmea_filter_idx = (self.nmea_filter_idx + 1) % NMEA_FILTERS.len();
        self.nmea_filter = NMEA_FILTERS[self.nmea_filter_idx].to_string();
        self.nmea_scroll_offset = 0;
        let label = if self.nmea_filter.is_empty() {
            "ALL"
        } else {
            &self.nmea_filter
        };
        self.set_status(format!("Filter: {}", label));
    }
}

/// Append to an NMEA buffer, dropping the oldest line beyond
/// [`NMEA_BUFFER_MAX`].
fn push_capped(buf: &mut VecDeque<String>, sentence: String) {
    if buf.len() >= NMEA_BUFFER_MAX {
        buf.pop_front();
    }
    buf.push_back(sentence);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ActiveTab;
    use crate::app::test_util::*;
    use crate::gpsd_client::GpsdEvent;
    use crossterm::event::KeyCode;

    // === NMEA tab ===

    #[test]
    fn test_nmea_pause() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Nmea;
        assert!(!app.nmea_paused);

        app.handle_event(key_event(KeyCode::Char('p')));
        assert!(app.nmea_paused);

        app.handle_event(key_event(KeyCode::Char('p')));
        assert!(!app.nmea_paused);
    }

    #[test]
    fn test_nmea_clear() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Nmea;
        app.nmea_buffer.push_back("$GPGGA,test".to_string());
        assert!(!app.nmea_buffer.is_empty());

        app.handle_event(key_event(KeyCode::Char('c')));
        assert!(app.nmea_buffer.is_empty());
    }

    #[test]
    fn test_nmea_filter_cycle() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Nmea;
        assert_eq!(app.nmea_filter, "");

        app.handle_event(key_event(KeyCode::Char('f')));
        assert_eq!(app.nmea_filter, "GGA");

        app.handle_event(key_event(KeyCode::Char('f')));
        assert_eq!(app.nmea_filter, "RMC");
    }

    /// NMEA tab with `n` GGA/RMC sentences and a 10-row view.
    fn nmea_app(n: usize) -> App {
        let mut app = new_app();
        app.active_tab = ActiveTab::Nmea;
        app.nmea_view_rows.set(10);
        for i in 0..n {
            let kind = if i % 2 == 0 { "GGA" } else { "RMC" };
            app.handle_gpsd_event(GpsdEvent::Nmea(format!("$GP{kind},{i}")));
        }
        app
    }

    #[test]
    fn test_nmea_scroll() {
        let mut app = nmea_app(50);
        assert!(app.nmea_following());

        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.nmea_scroll_offset, 1);
        assert!(!app.nmea_following());

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.nmea_scroll_offset, 0);

        // Page = view rows - 1
        app.handle_event(key_event(KeyCode::PageUp));
        assert_eq!(app.nmea_scroll_offset, 9);

        app.handle_event(key_event(KeyCode::PageDown));
        assert_eq!(app.nmea_scroll_offset, 0);

        app.handle_event(key_event(KeyCode::Home));
        assert_eq!(app.nmea_scroll_offset, 40); // oldest line at the top
        app.handle_event(key_event(KeyCode::End));
        assert_eq!(app.nmea_scroll_offset, 0);
        assert!(app.nmea_following());
    }

    #[test]
    fn test_nmea_scroll_clamped_to_content() {
        let mut app = nmea_app(15);
        for _ in 0..30 {
            app.handle_event(key_event(KeyCode::Up));
        }
        assert_eq!(app.nmea_scroll_offset, 5);
        // One Down press undoes one step, not 25 overshoot steps.
        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.nmea_scroll_offset, 4);

        // Fewer lines than the view: nothing to scroll.
        let mut app = nmea_app(3);
        app.handle_event(key_event(KeyCode::PageUp));
        assert_eq!(app.nmea_scroll_offset, 0);
    }

    #[test]
    fn test_nmea_scroll_clamp_respects_filter() {
        let mut app = nmea_app(30); // 15 GGA
        app.handle_event(key_event(KeyCode::Char('f'))); // GGA only
        app.handle_event(key_event(KeyCode::Home));
        assert_eq!(app.nmea_scroll_offset, 5);
    }

    #[test]
    fn test_nmea_stale_offset_clamped_before_scrolling() {
        // An offset left over from a longer buffer is clamped first.
        let mut app = nmea_app(20);
        app.nmea_scroll_offset = 500;
        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.nmea_scroll_offset, 9);
    }

    /// The lines visible in a `rows`-row window at the current offset.
    fn nmea_window(app: &App, rows: usize) -> Vec<String> {
        let mut w: Vec<String> = app
            .nmea_buffer
            .iter()
            .rev()
            .filter(|s| app.nmea_matches(s))
            .skip(app.nmea_scroll_offset)
            .take(rows)
            .cloned()
            .collect();
        w.reverse();
        w
    }

    #[test]
    fn test_nmea_view_anchored_while_scrolled() {
        let mut app = nmea_app(50);
        app.handle_event(key_event(KeyCode::PageUp));
        let before = nmea_window(&app, 10);
        for i in 0..5 {
            app.handle_gpsd_event(GpsdEvent::Nmea(format!("$GPGGA,new{i}")));
        }
        assert_eq!(nmea_window(&app, 10), before);
        assert_eq!(app.nmea_scroll_offset, 14);
    }

    #[test]
    fn test_nmea_anchor_with_filter_ignores_hidden_lines() {
        let mut app = nmea_app(50);
        app.handle_event(key_event(KeyCode::Char('f'))); // GGA
        app.handle_event(key_event(KeyCode::Up));
        let before = nmea_window(&app, 10);
        app.handle_gpsd_event(GpsdEvent::Nmea("$GPRMC,hidden".into()));
        assert_eq!(app.nmea_scroll_offset, 1);
        app.handle_gpsd_event(GpsdEvent::Nmea("$GPGGA,shown".into()));
        assert_eq!(app.nmea_scroll_offset, 2);
        assert_eq!(nmea_window(&app, 10), before);
    }

    #[test]
    fn test_nmea_anchor_at_full_buffer() {
        let mut app = nmea_app(NMEA_BUFFER_MAX);
        app.handle_event(key_event(KeyCode::PageUp));
        let before = nmea_window(&app, 10);
        app.handle_gpsd_event(GpsdEvent::Nmea("$GPGGA,evicts".into()));
        assert_eq!(app.nmea_buffer.len(), NMEA_BUFFER_MAX);
        assert_eq!(nmea_window(&app, 10), before);

        // Scrolled to the very top: evictions slide the view, never past
        // the end of the content.
        app.handle_event(key_event(KeyCode::Home));
        for i in 0..3 {
            app.handle_gpsd_event(GpsdEvent::Nmea(format!("$GPGGA,x{i}")));
        }
        assert_eq!(app.nmea_scroll_offset, app.nmea_max_scroll());
    }

    #[test]
    fn test_nmea_following_stays_at_bottom() {
        let mut app = nmea_app(50);
        app.handle_gpsd_event(GpsdEvent::Nmea("$GPGGA,latest".into()));
        assert_eq!(app.nmea_scroll_offset, 0);
        assert_eq!(nmea_window(&app, 10).last().unwrap(), "$GPGGA,latest");
    }

    #[test]
    fn test_nmea_pause_buffer() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Nmea;

        // Pause
        app.handle_event(key_event(KeyCode::Char('p')));
        assert!(app.nmea_paused);

        // Simulate NMEA arriving while paused
        app.handle_gpsd_event(GpsdEvent::Nmea("$GPGGA,paused".to_string()));
        assert_eq!(app.nmea_pause_buffer.len(), 1);
        assert_eq!(app.nmea_buffer.len(), 0);

        // Unpause - buffer should merge
        app.handle_event(key_event(KeyCode::Char('p')));
        assert!(!app.nmea_paused);
        assert_eq!(app.nmea_buffer.len(), 1);
        assert_eq!(app.nmea_pause_buffer.len(), 0);
    }

    // === Tab-specific keys don't fire on wrong tab ===

    #[test]
    fn test_nmea_keys_only_on_nmea_tab() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Dashboard;
        app.nmea_buffer.push_back("test".to_string());

        // 'c' on Dashboard should not clear NMEA buffer
        app.handle_event(key_event(KeyCode::Char('c')));
        assert!(!app.nmea_buffer.is_empty());
    }
}
