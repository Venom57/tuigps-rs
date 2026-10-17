//! Applying gpsd events to app state: connection generations, per-TPV
//! processing (TOFF, clock sync, logging, position hold) and staleness.

use super::App;
use super::nmea::NMEA_BUFFER_MAX;
use crate::data_model::GPSData;
use crate::gpsd_client::{GpsdEvent, GpsdMessage, MsgClass, TaggedEvent};

/// Seconds without a TPV before data is flagged stale.
pub const STALE_AFTER_SECS: f64 = 10.0;

impl App {
    /// Handle an event from a gpsd task, ignoring events from a task that
    /// has since been replaced (reconnect). Returns whether it was applied.
    pub fn handle_tagged_event(&mut self, (generation, event): TaggedEvent) -> bool {
        if generation != self.connection_generation {
            return false;
        }
        self.handle_gpsd_event(event);
        true
    }

    /// Start a new connection generation; called when the gpsd task is
    /// restarted. Returns the generation the new task must tag events with.
    pub fn begin_reconnect(&mut self) -> u64 {
        self.connection_generation += 1;
        self.gps_data.connected = false;
        self.gps_data.error_message = "Reconnecting...".to_string();
        self.stale = false;
        self.stale_seconds = 0.0;
        self.connection_generation
    }

    pub fn handle_gpsd_event(&mut self, event: GpsdEvent) {
        match event {
            GpsdEvent::Connected => {
                // Fresh connection: drop data from any previous session.
                // App-managed timing state lives outside GPSData and is kept.
                self.gps_data = GPSData {
                    connected: true,
                    ..Default::default()
                };
                self.stale = false;
                self.stale_seconds = 0.0;
            }
            GpsdEvent::Message(msg) => self.handle_message(msg),
            GpsdEvent::Error(msg) => {
                self.gps_data.connected = false;
                self.gps_data.error_message = msg;
            }
            GpsdEvent::Nmea(sentence) => {
                if self.nmea_paused {
                    if self.nmea_pause_buffer.len() >= NMEA_BUFFER_MAX {
                        self.nmea_pause_buffer.pop_front();
                    }
                    self.nmea_pause_buffer.push_back(sentence);
                } else {
                    self.push_nmea(sentence);
                }
            }
        }
    }

    fn handle_message(&mut self, msg: GpsdMessage) {
        let prev_time = if msg.class == MsgClass::Tpv {
            Some(self.gps_data.time.clone())
        } else {
            None
        };
        msg.apply(&mut self.gps_data);
        self.gps_data.connected = true;

        match msg.class {
            MsgClass::Tpv => self.on_tpv(&prev_time.unwrap_or_default()),
            MsgClass::Error => {
                let text = msg.json["message"].as_str().unwrap_or("unknown error");
                self.set_status(format!("gpsd error: {}", text));
            }
            _ => {}
        }
    }

    /// Per-TPV processing. Everything here runs only on TPV messages, never
    /// on SKY/PPS/TOFF/DEVICE traffic.
    fn on_tpv(&mut self, prev_time: &str) {
        self.stale = false;
        self.stale_seconds = 0.0;

        let time = &self.gps_data.time;
        let new_time = !time.is_empty() && time != prev_time;
        // A TPV without a timestamp still counts as a new epoch for sampling.
        let new_epoch = time.is_empty() || new_time;

        if new_time {
            self.on_new_tpv_time();
        }

        // Log if active (the logger de-duplicates by time)
        if let Some(ref mut logger) = self.logger
            && let Err(e) = logger.log_point(&self.gps_data)
        {
            let msg = format!("Log write error: {}", e);
            self.set_status(msg);
        }

        // Position hold: one sample per new fix
        if new_epoch
            && self.gps_data.has_fix()
            && let Some(ref mut hold) = self.position_hold
        {
            hold.add_fix(
                self.gps_data.latitude,
                self.gps_data.longitude,
                self.gps_data.alt_msl,
            );
        }
    }

    /// Staleness is based on the age of the last TPV, so PPS/DEVICE/etc.
    /// traffic cannot mask lost fixes. Before any TPV arrives, the last
    /// message of any class is used instead.
    pub fn update_staleness(&mut self, now: f64) {
        let reference = if self.gps_data.last_tpv > 0.0 {
            self.gps_data.last_tpv
        } else {
            self.gps_data.last_seen
        };
        if reference <= 0.0 {
            return;
        }
        let age = now - reference;
        self.stale = age > STALE_AFTER_SECS;
        self.stale_seconds = if self.stale { age } else { 0.0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_util::*;
    use crossterm::event::KeyCode;

    // === Staleness detection ===

    #[test]
    fn test_staleness() {
        let mut app = new_app();
        let now = now();

        // Recent data - not stale
        app.gps_data.last_seen = now;
        app.tick();
        assert!(!app.stale);

        // Old data - stale
        app.gps_data.last_seen = now - 15.0;
        app.tick();
        assert!(app.stale);
        assert!(app.stale_seconds > 10.0);
    }

    #[test]
    fn test_staleness_uses_tpv_age_not_pps() {
        let mut app = new_app();
        let t0 = 1_000_000.0;
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00Z", t0));
        // PPS keeps arriving, TPVs have stopped
        for i in 1..=15 {
            app.handle_gpsd_event(json_event(r#"{"class":"PPS"}"#, t0 + i as f64));
        }
        app.update_staleness(t0 + 15.0);
        assert!(app.stale);
        assert!((app.stale_seconds - 15.0).abs() < 1e-9);

        // A new TPV clears it
        app.handle_gpsd_event(tpv("2024-01-15T12:30:16Z", t0 + 16.0));
        assert!(!app.stale);
        app.update_staleness(t0 + 17.0);
        assert!(!app.stale);
        assert_eq!(app.stale_seconds, 0.0);
    }

    #[test]
    fn test_non_tpv_message_does_not_clear_stale() {
        let mut app = new_app();
        app.stale = true;
        app.stale_seconds = 15.0;
        app.handle_gpsd_event(json_event(r#"{"class":"SKY"}"#, now()));
        assert!(app.stale);
    }

    // === GPS event handling ===

    #[test]
    fn test_gpsd_tpv_clears_stale() {
        let mut app = new_app();
        app.stale = true;
        app.stale_seconds = 15.0;

        app.handle_gpsd_event(json_event(r#"{"class":"TPV","mode":1}"#, now()));
        assert!(!app.stale);
        assert_eq!(app.stale_seconds, 0.0);
        assert!(app.gps_data.connected);
    }

    #[test]
    fn test_message_applies_in_place() {
        let mut app = new_app();
        app.handle_gpsd_event(json_event(
            r#"{"class":"SKY","hdop":0.9,"satellites":[{"PRN":3,"used":true}]}"#,
            1.0,
        ));
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00Z", 2.0));
        // TPV does not disturb SKY-derived state
        assert!((app.gps_data.dop.hdop - 0.9).abs() < 1e-10);
        assert_eq!(app.gps_data.satellites.len(), 1);
        assert_eq!(app.gps_data.last_tpv, 2.0);
    }

    #[test]
    fn test_gpsd_error_class_sets_status() {
        let mut app = new_app();
        app.handle_gpsd_event(json_event(
            r#"{"class":"ERROR","message":"unrecognized request"}"#,
            1.0,
        ));
        assert!(app.status_message.contains("unrecognized request"));
    }

    #[test]
    fn test_connected_resets_gps_data_but_keeps_toff() {
        let mut app = new_app();
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00Z", now()));
        app.timing.toff_samples = vec![0.1];
        app.handle_gpsd_event(GpsdEvent::Error("closed".to_string()));
        assert!(!app.gps_data.connected);

        app.handle_gpsd_event(GpsdEvent::Connected);
        assert!(app.gps_data.connected);
        assert!(app.gps_data.error_message.is_empty());
        assert!(app.gps_data.latitude.is_nan());
        assert_eq!(app.timing.toff_samples, vec![0.1]);
    }

    #[test]
    fn test_reconnect_discards_stale_generation_events() {
        let mut app = new_app();
        assert!(app.handle_tagged_event((0, GpsdEvent::Connected)));

        let generation = app.begin_reconnect();
        assert_eq!(generation, 1);
        assert_eq!(app.gps_data.error_message, "Reconnecting...");

        // Events still queued from the aborted task must be ignored
        assert!(!app.handle_tagged_event((0, tpv("2024-01-15T12:30:00Z", now()))));
        assert!(!app.handle_tagged_event((0, GpsdEvent::Connected)));
        assert!(!app.gps_data.connected);
        assert_eq!(app.gps_data.error_message, "Reconnecting...");
        assert!(app.gps_data.latitude.is_nan());

        // Events from the new task are applied
        assert!(app.handle_tagged_event((1, GpsdEvent::Connected)));
        assert!(app.gps_data.connected);
    }

    // === Position hold ===

    #[test]
    fn test_hold_samples_only_new_tpvs() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('h')));
        let t = now();
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00Z", t));
        app.handle_gpsd_event(tpv("2024-01-15T12:30:01Z", t + 1.0));
        // Non-TPV traffic while the fix is still valid
        for line in [
            r#"{"class":"SKY","hdop":1.0}"#,
            r#"{"class":"PPS"}"#,
            r#"{"class":"TOFF"}"#,
        ] {
            app.handle_gpsd_event(json_event(line, t + 1.5));
        }
        // Duplicate TPV for the same epoch
        app.handle_gpsd_event(tpv("2024-01-15T12:30:01Z", t + 1.6));
        assert_eq!(hold_count(&app), 2);

        app.handle_gpsd_event(tpv("2024-01-15T12:30:02Z", t + 2.0));
        assert_eq!(hold_count(&app), 3);
    }

    #[test]
    fn test_gpsd_error() {
        let mut app = new_app();
        app.gps_data.connected = true;

        app.handle_gpsd_event(GpsdEvent::Error("connection refused".to_string()));
        assert!(!app.gps_data.connected);
        assert_eq!(app.gps_data.error_message, "connection refused");
    }
}
