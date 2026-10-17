//! Timing tab: TOFF clearing and armed clock-sync dispatch.

use super::App;
use crate::clock_sync;

impl App {
    pub(super) fn clear_toff(&mut self) {
        let had_data =
            !self.gps_data.toff_samples.is_empty() || self.gps_data.toff_armed_offset.is_finite();
        self.gps_data.toff_samples.clear();
        self.gps_data.toff_armed_offset = f64::NAN;
        self.gps_data.toff_armed_gps_time.clear();
        self.gps_data.toff_armed_sys_time = f64::NAN;
        self.armed_toff = false;
        if had_data {
            self.set_status("TOFF data cleared");
        } else {
            self.set_status("TOFF data already empty");
        }
    }

    /// Execute a queued clock sync on a blocking thread; the result comes
    /// back through `status_rx`. Never called from tests.
    pub(super) fn dispatch_clock_sync(&mut self) {
        if let Some(req) = self.pending_clock_sync.take() {
            let tx = self.status_tx.clone();
            tokio::spawn(async move {
                let msg = match clock_sync::sync_clock_async(req).await {
                    Ok(msg) => msg,
                    Err(e) => format!("Clock sync error: {}", e),
                };
                let _ = tx.send(msg).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ActiveTab;
    use crate::app::test_util::*;
    use crate::clock_sync::ClockSyncRequest;
    use crossterm::event::KeyCode;

    // === Timing tab ===

    #[test]
    fn test_timing_arm_toff() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;

        app.handle_event(key_event(KeyCode::Char('a')));
        assert!(app.armed_toff);
    }

    #[test]
    fn test_timing_clear_toff() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;
        app.gps_data.toff_samples = vec![1.0, 2.0, 3.0];
        app.gps_data.toff_armed_offset = 0.5;

        app.handle_event(key_event(KeyCode::Char('c')));
        assert!(app.gps_data.toff_samples.is_empty());
        assert!(app.gps_data.toff_armed_offset.is_nan());
        assert!(app.status_message.contains("cleared"));
    }

    #[test]
    fn test_timing_clear_empty() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;

        app.handle_event(key_event(KeyCode::Char('c')));
        assert!(app.status_message.contains("empty"));
    }

    #[test]
    fn test_timing_arm_shows_status() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;

        app.handle_event(key_event(KeyCode::Char('a')));
        assert!(app.armed_toff);
        assert!(app.status_message.contains("armed"));
    }

    #[test]
    fn test_timing_clock_sync_arm_toggle() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;

        app.handle_event(key_event(KeyCode::Char('k')));
        assert!(app.armed_clock_set);
        assert!(app.status_message.contains("ARMED"));

        app.handle_event(key_event(KeyCode::Char('k')));
        assert!(!app.armed_clock_set);
        assert!(app.status_message.contains("disarmed"));
    }

    fn armed_app() -> App {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;
        app.handle_event(key_event(KeyCode::Char('k')));
        assert!(app.armed_clock_set);
        app
    }

    #[test]
    fn test_timing_clock_sync_stays_armed_without_gps_time() {
        let mut app = armed_app();
        // A TPV without time must not queue a clock sync
        app.handle_gpsd_event(json_event(
            r#"{"class":"TPV","mode":3,"lat":1.0,"lon":2.0}"#,
            now(),
        ));
        assert!(app.armed_clock_set);
        assert!(app.pending_clock_sync.is_none());
    }

    #[test]
    fn test_clock_sync_ignores_non_tpv_messages() {
        let mut app = armed_app();
        let t = now();
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.000Z", t));
        // Consume the first trigger, re-arm, then send only non-TPV traffic
        app.pending_clock_sync = None;
        app.handle_event(key_event(KeyCode::Char('k')));
        for line in [
            r#"{"class":"SKY","hdop":1.0}"#,
            r#"{"class":"PPS","real_sec":1}"#,
            r#"{"class":"TOFF","real_sec":1}"#,
            r#"{"class":"DEVICE","path":"/dev/x"}"#,
        ] {
            app.handle_gpsd_event(json_event(line, t + 0.5));
        }
        assert!(app.armed_clock_set);
        assert!(app.pending_clock_sync.is_none());
    }

    #[test]
    fn test_clock_sync_requires_fix() {
        let mut app = armed_app();
        app.handle_gpsd_event(json_event(
            r#"{"class":"TPV","mode":1,"time":"2024-01-15T12:30:00.000Z"}"#,
            now(),
        ));
        assert!(app.armed_clock_set);
        assert!(app.pending_clock_sync.is_none());
    }

    #[test]
    fn test_clock_sync_requires_fresh_tpv_time() {
        let mut app = new_app();
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.000Z", now()));
        app.active_tab = ActiveTab::Timing;
        app.handle_event(key_event(KeyCode::Char('k')));
        // Repeated TPV for the same epoch is not fresh
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.000Z", now()));
        assert!(app.pending_clock_sync.is_none());
        assert!(app.armed_clock_set);
    }

    #[test]
    fn test_clock_sync_queued_on_fresh_fix_with_tpv_receipt() {
        let mut app = armed_app();
        let t = now();
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.123Z", t));
        // Later non-TPV traffic must not change the receipt time used
        app.handle_gpsd_event(json_event(r#"{"class":"PPS"}"#, t + 0.7));
        assert!(!app.armed_clock_set);
        assert_eq!(
            app.pending_clock_sync,
            Some(ClockSyncRequest {
                gps_time: "2024-01-15T12:30:00.123Z".to_string(),
                tpv_receipt: t,
            })
        );
        assert!(app.status_message.contains("Clock sync"));
    }

    #[test]
    fn test_timing_keys_only_on_timing_tab() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Dashboard;

        // 'a' on Dashboard should not arm TOFF
        app.handle_event(key_event(KeyCode::Char('a')));
        assert!(!app.armed_toff);
    }

    // === TOFF ===

    #[test]
    fn test_toff_accumulation_in_app() {
        let mut app = new_app();
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.000Z", now()));
        assert_eq!(app.gps_data.toff_samples.len(), 1);
    }

    #[test]
    fn test_toff_uses_tpv_receipt_time() {
        let mut app = new_app();
        // 2024-01-15T12:30:00Z = 1705321800
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.000Z", 1_705_321_800.25));
        assert!((app.gps_data.toff_samples[0] + 0.25).abs() < 1e-6);
    }

    #[test]
    fn test_toff_circular_buffer_in_app() {
        let mut app = new_app();
        let now = now();

        // Send 25 updates to overflow the 20-element buffer
        for i in 0..25 {
            let time = format!("2024-01-15T12:30:{:02}.000Z", i % 60);
            app.handle_gpsd_event(tpv(&time, now + i as f64));
        }

        assert_eq!(app.gps_data.toff_samples.len(), 20); // capped
    }

    #[test]
    fn test_toff_arm_and_capture() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;

        // Arm the TOFF
        app.handle_event(key_event(KeyCode::Char('a')));
        assert!(app.armed_toff);

        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.000Z", now()));

        // Armed TOFF should have fired
        assert!(!app.armed_toff);
        assert!(app.gps_data.toff_armed_offset.is_finite());
        assert_eq!(app.gps_data.toff_armed_gps_time, "2024-01-15T12:30:00.000Z");
        assert!(app.status_message.contains("captured"));
    }

    #[test]
    fn test_toff_clear_persists_across_updates() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;
        let now = now();

        // Accumulate some TOFF data
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.000Z", now));
        assert!(!app.gps_data.toff_samples.is_empty());

        // Clear it
        app.handle_event(key_event(KeyCode::Char('c')));
        assert!(app.gps_data.toff_samples.is_empty());

        // Further non-TPV traffic and a timeless TPV keep it empty
        app.handle_gpsd_event(json_event(r#"{"class":"SKY"}"#, now + 0.5));
        app.handle_gpsd_event(json_event(r#"{"class":"TPV","mode":1}"#, now + 1.0));
        assert!(app.gps_data.toff_samples.is_empty());
    }
}
