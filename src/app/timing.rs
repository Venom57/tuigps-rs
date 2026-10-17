//! Timing tab state: TOFF samples, one-shot armed TOFF capture and armed
//! clock sync. This state is managed by the app (not reported by gpsd), so
//! it lives in [`TimingState`] on `App` rather than in `GPSData`, and is
//! unaffected when a reconnect resets the GPS data.

use super::App;
use crate::clock_sync::{self, ClockSyncRequest};

/// Number of TOFF samples kept; the oldest is dropped first.
const TOFF_SAMPLES_MAX: usize = 20;

/// App-managed timing state shown on the Timing tab.
#[derive(Debug, Clone, PartialEq)]
pub struct TimingState {
    /// Recent TOFF offsets (GPS time minus TPV receipt time, seconds),
    /// oldest first, at most 20.
    pub toff_samples: Vec<f64>,
    /// Offset from the last armed capture (`NAN`: none).
    pub toff_armed_offset: f64,
    /// GPS time of the last armed capture (empty: none).
    pub toff_armed_gps_time: String,
    /// TPV receipt time (Unix seconds) of the last armed capture (`NAN`: none).
    pub toff_armed_sys_time: f64,
    /// A one-shot TOFF capture fires on the next new TPV time.
    pub armed_toff: bool,
    /// Clock sync fires on the next new TPV time with a 2D/3D fix.
    pub armed_clock_set: bool,
}

impl Default for TimingState {
    fn default() -> Self {
        Self {
            toff_samples: Vec::new(),
            toff_armed_offset: f64::NAN,
            toff_armed_gps_time: String::new(),
            toff_armed_sys_time: f64::NAN,
            armed_toff: false,
            armed_clock_set: false,
        }
    }
}

impl TimingState {
    /// Add a TOFF sample to the circular buffer. If a capture is armed it
    /// also records `offset`, `gps_time` and `receipt` and disarms.
    /// Returns whether an armed capture fired.
    fn record_toff(&mut self, offset: f64, gps_time: &str, receipt: f64) -> bool {
        if self.toff_samples.len() >= TOFF_SAMPLES_MAX {
            self.toff_samples.remove(0);
        }
        self.toff_samples.push(offset);

        if !self.armed_toff {
            return false;
        }
        self.armed_toff = false;
        self.toff_armed_offset = offset;
        self.toff_armed_gps_time = gps_time.to_string();
        self.toff_armed_sys_time = receipt;
        true
    }

    /// Drop all TOFF data and any pending arm. Returns whether there was
    /// data to drop.
    fn clear_toff(&mut self) -> bool {
        let had_data = !self.toff_samples.is_empty() || self.toff_armed_offset.is_finite();
        self.toff_samples.clear();
        self.toff_armed_offset = f64::NAN;
        self.toff_armed_gps_time.clear();
        self.toff_armed_sys_time = f64::NAN;
        self.armed_toff = false;
        had_data
    }
}

impl App {
    pub(super) fn arm_toff(&mut self) {
        self.timing.armed_toff = true;
        self.set_status("TOFF armed — waiting for next fix...");
    }

    pub(super) fn clear_toff(&mut self) {
        if self.timing.clear_toff() {
            self.set_status("TOFF data cleared");
        } else {
            self.set_status("TOFF data already empty");
        }
    }

    pub(super) fn toggle_clock_sync(&mut self) {
        if self.timing.armed_clock_set {
            self.timing.armed_clock_set = false;
            self.set_status("Clock sync disarmed");
        } else {
            self.timing.armed_clock_set = true;
            self.set_status("Clock sync ARMED — will fire on next 2D/3D fix");
        }
    }

    /// Timing work for a TPV whose time differs from the previous TPV's:
    /// TOFF sampling, armed TOFF capture and armed clock sync.
    pub(super) fn on_new_tpv_time(&mut self) {
        let data = &self.gps_data;

        // TOFF offset against the TPV receipt time
        if data.last_tpv > 0.0
            && let Ok(gps_time) = chrono::DateTime::parse_from_rfc3339(&data.time)
        {
            let gps_epoch =
                gps_time.timestamp() as f64 + gps_time.timestamp_subsec_nanos() as f64 / 1e9;
            let offset = gps_epoch - data.last_tpv;
            if self.timing.record_toff(offset, &data.time, data.last_tpv) {
                self.set_status(format!(
                    "TOFF captured: {}",
                    crate::formatting::fmt_offset(offset)
                ));
            }
        }

        // Armed clock sync: 2D/3D fix only. Execution is deferred to the
        // event loop so it never blocks here.
        if self.timing.armed_clock_set && self.gps_data.has_fix() {
            self.timing.armed_clock_set = false;
            self.pending_clock_sync = Some(ClockSyncRequest {
                gps_time: self.gps_data.time.clone(),
                tpv_receipt: self.gps_data.last_tpv,
            });
            self.set_status("Clock sync: setting system clock...");
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
    use crossterm::event::KeyCode;

    // === Timing tab ===

    #[test]
    fn test_timing_arm_toff() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;

        app.handle_event(key_event(KeyCode::Char('a')));
        assert!(app.timing.armed_toff);
    }

    #[test]
    fn test_timing_clear_toff() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;
        app.timing.toff_samples = vec![1.0, 2.0, 3.0];
        app.timing.toff_armed_offset = 0.5;

        app.handle_event(key_event(KeyCode::Char('c')));
        assert!(app.timing.toff_samples.is_empty());
        assert!(app.timing.toff_armed_offset.is_nan());
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
        assert!(app.timing.armed_toff);
        assert!(app.status_message.contains("armed"));
    }

    #[test]
    fn test_timing_clock_sync_arm_toggle() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;

        app.handle_event(key_event(KeyCode::Char('k')));
        assert!(app.timing.armed_clock_set);
        assert!(app.status_message.contains("ARMED"));

        app.handle_event(key_event(KeyCode::Char('k')));
        assert!(!app.timing.armed_clock_set);
        assert!(app.status_message.contains("disarmed"));
    }

    fn armed_app() -> App {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;
        app.handle_event(key_event(KeyCode::Char('k')));
        assert!(app.timing.armed_clock_set);
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
        assert!(app.timing.armed_clock_set);
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
        assert!(app.timing.armed_clock_set);
        assert!(app.pending_clock_sync.is_none());
    }

    #[test]
    fn test_clock_sync_requires_fix() {
        let mut app = armed_app();
        app.handle_gpsd_event(json_event(
            r#"{"class":"TPV","mode":1,"time":"2024-01-15T12:30:00.000Z"}"#,
            now(),
        ));
        assert!(app.timing.armed_clock_set);
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
        assert!(app.timing.armed_clock_set);
    }

    #[test]
    fn test_clock_sync_queued_on_fresh_fix_with_tpv_receipt() {
        let mut app = armed_app();
        let t = now();
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.123Z", t));
        // Later non-TPV traffic must not change the receipt time used
        app.handle_gpsd_event(json_event(r#"{"class":"PPS"}"#, t + 0.7));
        assert!(!app.timing.armed_clock_set);
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
        assert!(!app.timing.armed_toff);
    }

    // === TOFF ===

    #[test]
    fn test_toff_accumulation_in_app() {
        let mut app = new_app();
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.000Z", now()));
        assert_eq!(app.timing.toff_samples.len(), 1);
    }

    #[test]
    fn test_toff_uses_tpv_receipt_time() {
        let mut app = new_app();
        // 2024-01-15T12:30:00Z = 1705321800
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.000Z", 1_705_321_800.25));
        assert!((app.timing.toff_samples[0] + 0.25).abs() < 1e-6);
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

        assert_eq!(app.timing.toff_samples.len(), 20); // capped
    }

    #[test]
    fn test_toff_arm_and_capture() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;

        // Arm the TOFF
        app.handle_event(key_event(KeyCode::Char('a')));
        assert!(app.timing.armed_toff);

        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.000Z", now()));

        // Armed TOFF should have fired
        assert!(!app.timing.armed_toff);
        assert!(app.timing.toff_armed_offset.is_finite());
        assert_eq!(app.timing.toff_armed_gps_time, "2024-01-15T12:30:00.000Z");
        assert!(app.status_message.contains("captured"));
    }

    #[test]
    fn test_toff_clear_persists_across_updates() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;
        let now = now();

        // Accumulate some TOFF data
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00.000Z", now));
        assert!(!app.timing.toff_samples.is_empty());

        // Clear it
        app.handle_event(key_event(KeyCode::Char('c')));
        assert!(app.timing.toff_samples.is_empty());

        // Further non-TPV traffic and a timeless TPV keep it empty
        app.handle_gpsd_event(json_event(r#"{"class":"SKY"}"#, now + 0.5));
        app.handle_gpsd_event(json_event(r#"{"class":"TPV","mode":1}"#, now + 1.0));
        assert!(app.timing.toff_samples.is_empty());
    }
}
