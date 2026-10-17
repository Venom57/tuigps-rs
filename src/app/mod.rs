//! Application state and key dispatch.
//!
//! [`App`] owns everything the UI renders. Its behaviour is split across
//! submodules by concern, each adding an `impl App` block:
//!
//! - `keys`: the key binding table (dispatch, help overlay, status hints)
//! - `settings`: the settings dialog
//! - `device`: Device tab controls and u-blox command execution
//! - `nmea`: NMEA buffer, pause, filter and scrolling
//! - `scroll`: shared scroll requests and satellite-table scrolling
//! - `timing`: TOFF capture and armed clock sync
//! - `gpsd`: applying gpsd events, staleness and reconnect generations
//! - `event_loop`: [`run`], the async loop driving all of the above

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

use crate::clock_sync::ClockSyncRequest;
use crate::config::{Config, Startup};
use crate::data_model::GPSData;
use crate::gps_logger::{GpsLogger, LogFormat};
use crate::position_hold::PositionHold;

mod device;
mod event_loop;
mod gpsd;
mod keys;
mod nmea;
mod scroll;
mod settings;
#[cfg(test)]
mod test_util;
mod timing;

pub use device::{
    DEVICE_CONTROL_COUNT, DeviceCommand, DeviceConfigState, DeviceConfirm, DeviceValues,
    GNSS_NAMES_CONFIG, NAV_RATES, OUTPUT_LOG_MAX, POWER_MODES, PPS_FREQUENCIES, SERIAL_SPEEDS,
};
pub use event_loop::run;
pub use gpsd::STALE_AFTER_SECS;
pub use keys::{
    Action, GLOBAL_BINDINGS, KeyBinding, SETTINGS_BINDINGS, lookup_binding, tab_bindings,
};
pub use nmea::NMEA_BUFFER_MAX;
pub use scroll::Scroll;
pub use settings::{SettingsField, validate_port};
pub use timing::TimingState;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActiveTab {
    Dashboard,
    Satellites,
    Timing,
    Device,
    Nmea,
}

impl ActiveTab {
    pub fn title(&self) -> &'static str {
        match self {
            ActiveTab::Dashboard => "Dashboard",
            ActiveTab::Satellites => "Satellites",
            ActiveTab::Timing => "Timing",
            ActiveTab::Device => "Device",
            ActiveTab::Nmea => "NMEA",
        }
    }

    pub const ALL: &[ActiveTab] = &[
        ActiveTab::Dashboard,
        ActiveTab::Satellites,
        ActiveTab::Timing,
        ActiveTab::Device,
        ActiveTab::Nmea,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnitSystem {
    Metric,
    Imperial,
    Nautical,
}

impl UnitSystem {
    pub fn as_str(&self) -> &'static str {
        match self {
            UnitSystem::Metric => "metric",
            UnitSystem::Imperial => "imperial",
            UnitSystem::Nautical => "nautical",
        }
    }

    fn next(self) -> Self {
        match self {
            UnitSystem::Metric => UnitSystem::Imperial,
            UnitSystem::Imperial => UnitSystem::Nautical,
            UnitSystem::Nautical => UnitSystem::Metric,
        }
    }

    fn prev(self) -> Self {
        self.next().next()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[allow(clippy::upper_case_acronyms)]
pub enum CoordFormat {
    DD,
    DMS,
    DDM,
}

impl CoordFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            CoordFormat::DD => "dd",
            CoordFormat::DMS => "dms",
            CoordFormat::DDM => "ddm",
        }
    }

    fn next(self) -> Self {
        match self {
            CoordFormat::DD => CoordFormat::DMS,
            CoordFormat::DMS => CoordFormat::DDM,
            CoordFormat::DDM => CoordFormat::DD,
        }
    }

    fn prev(self) -> Self {
        self.next().next()
    }
}

pub struct App {
    pub gps_data: GPSData,
    pub active_tab: ActiveTab,
    pub units: UnitSystem,
    pub coord_format: CoordFormat,
    pub should_quit: bool,
    pub show_settings: bool,
    pub show_help: bool,

    // Subsystems
    pub logger: Option<GpsLogger>,
    pub log_dir: PathBuf,
    pub position_hold: Option<PositionHold>,

    // Settings
    pub host: String,
    pub port: u16,
    pub settings_field: SettingsField,
    pub settings_editing: bool,
    pub settings_edit_buf: String,
    /// Validation error shown inside the settings dialog.
    pub settings_message: String,
    /// Values when the dialog was opened; restored on Esc.
    pub settings_snapshot: Option<Config>,
    /// Where settings are saved on apply (`None`: not persisted).
    pub config_path: Option<PathBuf>,

    // NMEA state
    pub nmea_buffer: VecDeque<String>,
    pub nmea_paused: bool,
    pub nmea_pause_buffer: VecDeque<String>,
    pub nmea_filter: String,
    pub nmea_filter_idx: usize,
    /// Lines scrolled back from the newest; 0 = following new sentences.
    pub nmea_scroll_offset: usize,
    /// NMEA lines visible in the last render (0 = not rendered yet).
    pub nmea_view_rows: Cell<usize>,

    // Satellite table scroll
    pub sat_scroll_offset: usize,
    /// Satellite rows visible in the last render (0 = not rendered yet).
    pub sat_view_rows: Cell<usize>,

    // Device config state
    pub device_config: DeviceConfigState,
    pub pending_device_cmds: Vec<DeviceCommand>,

    // Clock sync
    /// TOFF samples/capture and clock-sync arming (Timing tab).
    pub timing: TimingState,
    /// Clock sync request waiting to be dispatched by the event loop.
    pub pending_clock_sync: Option<ClockSyncRequest>,

    /// Generation of the current gpsd connection task; events tagged with an
    /// older generation (from an aborted task) are ignored.
    pub connection_generation: u64,

    // Staleness
    pub stale: bool,
    pub stale_seconds: f64,

    // Reconnect signal
    pub reconnect_requested: bool,

    // Status message (shown on current tab, auto-clears)
    pub status_message: String,
    pub status_message_tick: u32,

    // Channel for async command output (device config)
    pub cmd_tx: mpsc::Sender<String>,
    pub cmd_rx: mpsc::Receiver<String>,

    // Channel for async status messages (clock sync results)
    pub status_tx: mpsc::Sender<String>,
    pub status_rx: mpsc::Receiver<String>,
}

impl App {
    pub fn new(host: String, port: u16) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (status_tx, status_rx) = mpsc::channel(8);
        Self {
            gps_data: GPSData::default(),
            active_tab: ActiveTab::Dashboard,
            units: UnitSystem::Metric,
            coord_format: CoordFormat::DD,
            should_quit: false,
            show_settings: false,
            show_help: false,
            logger: None,
            log_dir: PathBuf::from("."),
            position_hold: None,
            host,
            port,
            settings_field: SettingsField::Host,
            settings_editing: false,
            settings_edit_buf: String::new(),
            settings_message: String::new(),
            settings_snapshot: None,
            config_path: None,
            nmea_buffer: VecDeque::with_capacity(NMEA_BUFFER_MAX),
            nmea_paused: false,
            nmea_pause_buffer: VecDeque::with_capacity(NMEA_BUFFER_MAX),
            nmea_filter: String::new(),
            nmea_filter_idx: 0,
            nmea_scroll_offset: 0,
            nmea_view_rows: Cell::new(0),
            sat_scroll_offset: 0,
            sat_view_rows: Cell::new(0),
            device_config: DeviceConfigState::new(),
            pending_device_cmds: Vec::new(),
            timing: TimingState::default(),
            pending_clock_sync: None,
            connection_generation: 0,
            stale: false,
            stale_seconds: 0.0,
            reconnect_requested: false,
            status_message: String::new(),
            status_message_tick: 0,
            cmd_tx,
            cmd_rx,
            status_tx,
            status_rx,
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = msg.into();
        self.status_message_tick = 5; // show for 5 ticks (seconds)
    }

    /// Build the app from startup settings (config file + CLI overrides).
    pub fn from_startup(startup: Startup) -> Self {
        let Startup {
            config,
            config_path,
            message,
        } = startup;
        let mut app = App::new(config.host, config.port);
        app.units = config.units;
        app.coord_format = config.coord_format;
        app.config_path = config_path;
        if let Some(msg) = message {
            app.set_status(msg);
        }
        app
    }

    /// The persistable settings currently in effect.
    pub fn current_config(&self) -> Config {
        Config {
            host: self.host.clone(),
            port: self.port,
            units: self.units,
            coord_format: self.coord_format,
        }
    }

    /// Key handling precedence: Ctrl+C, settings dialog, help overlay,
    /// pending device confirmation, then the key binding table.
    pub fn handle_event(&mut self, event: Event) {
        let Event::Key(key) = event else {
            return;
        };
        if key.kind != KeyEventKind::Press {
            return;
        }

        // Ctrl+C always quits, regardless of tab, overlay or prompt.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        if self.show_settings {
            self.handle_settings_key(key);
            return;
        }

        if self.show_help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
            ) {
                self.show_help = false;
            }
            return;
        }

        if self.device_config.confirm.is_some() {
            self.handle_device_confirm(key);
            return;
        }

        // Ctrl/Alt chords are not bound (Ctrl+C is handled above).
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return;
        }

        if let Some(binding) = lookup_binding(self.active_tab, key.code) {
            self.run_action(binding.action, key.code);
        }
    }

    fn run_action(&mut self, action: Action, code: KeyCode) {
        match action {
            Action::ToggleHelp => self.show_help = !self.show_help,
            Action::JumpTab => {
                if let KeyCode::Char(c @ '1'..='9') = code
                    && let Some(tab) = ActiveTab::ALL.get(c as usize - '1' as usize)
                {
                    self.active_tab = *tab;
                }
            }
            Action::NextTab => self.next_tab(),
            Action::PrevTab => self.prev_tab(),
            Action::Quit => self.should_quit = true,
            Action::Reconnect => self.reconnect_requested = true,
            Action::OpenSettings => self.open_settings(),
            Action::CycleUnits => self.cycle_units(),
            Action::OpenMaps => self.open_maps(),
            Action::ToggleLog => self.toggle_logging(),
            Action::ToggleHold => self.toggle_hold(),
            Action::CloseOverlay => {
                // Nothing open: Esc just dismisses the status message.
                self.status_message.clear();
                self.status_message_tick = 0;
            }
            Action::ScrollLine | Action::ScrollPage | Action::ScrollEnds => {
                let scroll = match code {
                    KeyCode::Up => Scroll::Up(1),
                    KeyCode::Down => Scroll::Down(1),
                    KeyCode::PageUp => Scroll::Up(self.page_size()),
                    KeyCode::PageDown => Scroll::Down(self.page_size()),
                    KeyCode::Home => Scroll::Top,
                    _ => Scroll::Bottom,
                };
                match self.active_tab {
                    ActiveTab::Nmea => self.scroll_nmea(scroll),
                    ActiveTab::Satellites => self.scroll_satellites(scroll),
                    _ => {}
                }
            }
            Action::NmeaPause => self.toggle_nmea_pause(),
            Action::NmeaFilter => self.cycle_nmea_filter(),
            Action::NmeaClear => {
                self.nmea_buffer.clear();
                self.nmea_pause_buffer.clear();
                self.nmea_scroll_offset = 0;
                self.set_status("NMEA buffer cleared");
            }
            Action::ToffArm => self.arm_toff(),
            Action::ToffClear => self.clear_toff(),
            Action::ClockSyncToggle => self.toggle_clock_sync(),
            Action::DeviceSelect => {
                let dc = &mut self.device_config;
                if code == KeyCode::Up {
                    dc.selected_control = dc.selected_control.saturating_sub(1);
                } else if dc.selected_control < DEVICE_CONTROL_COUNT - 1 {
                    dc.selected_control += 1;
                }
            }
            Action::DeviceAdjust => {
                self.device_config_adjust(if code == KeyCode::Left { -1 } else { 1 })
            }
            Action::DeviceApply => self.device_config_activate(),
            Action::Info => {}
        }
    }

    pub fn tick(&mut self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.update_staleness(now);

        // Auto-clear status message
        if self.status_message_tick > 0 {
            self.status_message_tick -= 1;
            if self.status_message_tick == 0 {
                self.status_message.clear();
            }
        }
    }

    fn next_tab(&mut self) {
        let tabs = ActiveTab::ALL;
        let idx = tabs.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        self.active_tab = tabs[(idx + 1) % tabs.len()];
    }

    fn prev_tab(&mut self) {
        let tabs = ActiveTab::ALL;
        let idx = tabs.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        self.active_tab = tabs[(idx + tabs.len() - 1) % tabs.len()];
    }

    fn cycle_units(&mut self) {
        self.units = self.units.next();
    }

    fn maps_url(&self) -> Option<String> {
        self.gps_data.has_fix().then(|| {
            format!(
                "https://www.google.com/maps?q={},{}",
                self.gps_data.latitude, self.gps_data.longitude
            )
        })
    }

    fn open_maps(&mut self) {
        match self.maps_url() {
            None => self.set_status("No fix: cannot open map"),
            Some(url) => match open::that(&url) {
                Ok(()) => self.set_status("Opened map in browser"),
                Err(e) => self.set_status(format!("Failed to open browser: {}", e)),
            },
        }
    }

    fn toggle_logging(&mut self) {
        if self.logger.is_some() {
            self.stop_logging();
        } else {
            let mut logger = GpsLogger::new(LogFormat::Gpx, &self.log_dir);
            match logger.start() {
                Ok(()) => {
                    self.set_status(format!("Logging to {}", logger.filename));
                    self.logger = Some(logger);
                }
                Err(e) => self.set_status(format!("Failed to start logging: {}", e)),
            }
        }
    }

    /// Stop and close the active log (if any), reporting the result.
    pub fn stop_logging(&mut self) {
        if let Some(mut logger) = self.logger.take() {
            match logger.stop() {
                Ok(()) => self.set_status(format!(
                    "Logging stopped: {} points written to {}",
                    logger.point_count, logger.filename
                )),
                Err(e) => self.set_status(format!("Error closing log {}: {}", logger.filename, e)),
            }
        }
    }

    fn toggle_hold(&mut self) {
        if let Some(hold) = self.position_hold.take() {
            match hold.result() {
                Some(r) => self.set_status(format!(
                    "Hold stopped: {} fixes, mean {:.7}, {:.7}",
                    r.count, r.mean_lat, r.mean_lon
                )),
                None => self.set_status("Hold stopped (fewer than 2 fixes)"),
            }
        } else {
            self.position_hold = Some(PositionHold::new());
            self.set_status("Position hold started");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_util::*;

    // === Global keybindings ===

    #[test]
    fn test_quit() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('q')));
        assert!(app.should_quit);
    }

    #[test]
    fn test_reconnect() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('r')));
        assert!(app.reconnect_requested);
    }

    #[test]
    fn test_settings_open_close() {
        let mut app = new_app();
        assert!(!app.show_settings);
        app.handle_event(key_event(KeyCode::Char('s')));
        assert!(app.show_settings);
        app.handle_event(key_event(KeyCode::Esc));
        assert!(!app.show_settings);
    }

    #[test]
    fn test_cycle_units() {
        let mut app = new_app();
        assert_eq!(app.units, UnitSystem::Metric);
        app.handle_event(key_event(KeyCode::Char('u')));
        assert_eq!(app.units, UnitSystem::Imperial);
        app.handle_event(key_event(KeyCode::Char('u')));
        assert_eq!(app.units, UnitSystem::Nautical);
        app.handle_event(key_event(KeyCode::Char('u')));
        assert_eq!(app.units, UnitSystem::Metric);
    }

    #[test]
    fn test_toggle_logging() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = new_app();
        app.log_dir = dir.path().to_path_buf();
        assert!(app.logger.is_none());

        app.handle_event(key_event(KeyCode::Char('l')));
        let filename = app
            .logger
            .as_ref()
            .expect("logging started")
            .filename
            .clone();
        assert!(filename.starts_with(dir.path().to_str().unwrap()));
        assert!(std::path::Path::new(&filename).exists());

        app.handle_event(key_event(KeyCode::Char('l')));
        assert!(app.logger.is_none());
        let contents = std::fs::read_to_string(&filename).unwrap();
        assert!(contents.trim_end().ends_with("</gpx>"));
    }

    #[test]
    fn test_toggle_hold() {
        let mut app = new_app();
        assert!(app.position_hold.is_none());
        app.handle_event(key_event(KeyCode::Char('h')));
        assert!(app.position_hold.is_some());
        app.handle_event(key_event(KeyCode::Char('h')));
        assert!(app.position_hold.is_none());
    }

    #[test]
    fn test_tab_switching() {
        let mut app = new_app();
        assert_eq!(app.active_tab, ActiveTab::Dashboard);
        app.handle_event(key_event(KeyCode::Tab));
        assert_eq!(app.active_tab, ActiveTab::Satellites);
        app.handle_event(key_event(KeyCode::Tab));
        assert_eq!(app.active_tab, ActiveTab::Timing);
        app.handle_event(key_event(KeyCode::Tab));
        assert_eq!(app.active_tab, ActiveTab::Device);
        app.handle_event(key_event(KeyCode::Tab));
        assert_eq!(app.active_tab, ActiveTab::Nmea);
        app.handle_event(key_event(KeyCode::Tab));
        assert_eq!(app.active_tab, ActiveTab::Dashboard); // wraps
    }

    #[test]
    fn test_backtab() {
        let mut app = new_app();
        assert_eq!(app.active_tab, ActiveTab::Dashboard);
        app.handle_event(key_event(KeyCode::BackTab));
        assert_eq!(app.active_tab, ActiveTab::Nmea); // wraps backwards
    }

    #[test]
    fn test_from_startup_applies_config() {
        let app = App::from_startup(Startup {
            config: Config {
                host: "gps.lan".into(),
                port: 4000,
                units: UnitSystem::Nautical,
                coord_format: CoordFormat::DDM,
            },
            config_path: Some(PathBuf::from("/nonexistent/config.toml")),
            message: Some("hello".into()),
        });
        assert_eq!(app.host, "gps.lan");
        assert_eq!(app.port, 4000);
        assert_eq!(app.units, UnitSystem::Nautical);
        assert_eq!(app.coord_format, CoordFormat::DDM);
        assert_eq!(app.status_message, "hello");
        assert_eq!(app.current_config().port, 4000);
    }

    #[test]
    fn test_ctrl_c_quits_globally() {
        // On NMEA tab: quits and does not clear the buffer
        let mut app = new_app();
        app.active_tab = ActiveTab::Nmea;
        app.nmea_buffer.push_back("$GPGGA".to_string());
        app.handle_event(key_event_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
        assert_eq!(app.nmea_buffer.len(), 1);

        // On Timing tab: quits and keeps TOFF samples
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;
        app.timing.toff_samples = vec![1.0];
        app.handle_event(key_event_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
        assert_eq!(app.timing.toff_samples.len(), 1);

        // With settings open (even while editing)
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event(KeyCode::Enter));
        assert!(app.settings_editing);
        app.handle_event(key_event_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
        assert_eq!(app.settings_edit_buf, "localhost");
    }

    #[test]
    fn test_status_message_auto_clears() {
        let mut app = new_app();
        app.set_status("test message");
        assert!(!app.status_message.is_empty());

        for _ in 0..5 {
            app.tick();
        }
        assert!(app.status_message.is_empty());
    }

    #[test]
    fn test_ctrl_c_quits_from_every_state() {
        let ctrl_c = || key_event_mod(KeyCode::Char('c'), KeyModifiers::CONTROL);

        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('?')));
        assert!(app.show_help);
        app.handle_event(ctrl_c());
        assert!(app.should_quit);

        let mut app = device_app(DEVICE_CONTROL_COUNT - 1);
        app.handle_event(key_event(KeyCode::Enter));
        assert!(app.device_config.confirm.is_some());
        app.handle_event(ctrl_c());
        assert!(app.should_quit);
        assert!(app.pending_device_cmds.is_empty());

        let mut app = new_app();
        type_port(&mut app, "12");
        app.handle_event(ctrl_c());
        assert!(app.should_quit);
        assert_eq!(app.settings_edit_buf, "12");
    }

    // === Help overlay and navigation ===

    #[test]
    fn test_help_toggle_and_close() {
        for close in [KeyCode::Char('?'), KeyCode::Esc, KeyCode::Char('q')] {
            let mut app = new_app();
            app.handle_event(key_event(KeyCode::Char('?')));
            assert!(app.show_help);
            app.handle_event(key_event(close));
            assert!(!app.show_help, "{close:?}");
            assert!(!app.should_quit, "{close:?}");
        }
    }

    #[test]
    fn test_help_shift_question_mark() {
        // Most terminals report '?' with SHIFT.
        let mut app = new_app();
        app.handle_event(key_event_mod(KeyCode::Char('?'), KeyModifiers::SHIFT));
        assert!(app.show_help);
    }

    #[test]
    fn test_help_swallows_other_keys() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Nmea;
        app.nmea_buffer.push_back("$GPGGA".into());
        app.handle_event(key_event(KeyCode::Char('?')));
        for code in [
            KeyCode::Char('c'),
            KeyCode::Char('s'),
            KeyCode::Char('1'),
            KeyCode::Tab,
            KeyCode::Char('r'),
        ] {
            app.handle_event(key_event(code));
        }
        assert!(app.show_help);
        assert_eq!(app.nmea_buffer.len(), 1);
        assert!(!app.show_settings);
        assert_eq!(app.active_tab, ActiveTab::Nmea);
        assert!(!app.reconnect_requested);
    }

    #[test]
    fn test_number_keys_jump_to_tabs() {
        let mut app = new_app();
        for (c, tab) in ['1', '2', '3', '4', '5'].iter().zip(ActiveTab::ALL) {
            app.handle_event(key_event(KeyCode::Char(*c)));
            assert_eq!(app.active_tab, *tab);
        }
        // Works from every tab, and unbound digits do nothing.
        for tab in ActiveTab::ALL {
            app.active_tab = *tab;
            app.handle_event(key_event(KeyCode::Char('6')));
            assert_eq!(app.active_tab, *tab);
            app.handle_event(key_event(KeyCode::Char('2')));
            assert_eq!(app.active_tab, ActiveTab::Satellites);
        }
    }

    #[test]
    fn test_esc_in_main_view_clears_status() {
        let mut app = new_app();
        app.set_status("something");
        app.handle_event(key_event(KeyCode::Esc));
        assert!(app.status_message.is_empty());
        assert!(!app.should_quit);
    }

    #[test]
    fn test_ctrl_chords_unbound_in_main_view() {
        let mut app = new_app();
        app.handle_event(key_event_mod(KeyCode::Char('q'), KeyModifiers::CONTROL));
        assert!(!app.should_quit);
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(!app.show_settings);
    }

    #[test]
    fn test_hold_toggle_feedback() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('h')));
        assert!(app.status_message.contains("started"));
        app.handle_gpsd_event(tpv("2024-01-15T12:30:00Z", 1.0));
        app.handle_gpsd_event(tpv("2024-01-15T12:30:01Z", 2.0));
        app.handle_event(key_event(KeyCode::Char('h')));
        assert!(
            app.status_message.contains("2 fixes"),
            "{}",
            app.status_message
        );
    }

    // === Feedback / misc ===

    #[test]
    fn test_logging_status_feedback() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = new_app();
        app.log_dir = dir.path().to_path_buf();
        app.handle_event(key_event(KeyCode::Char('l')));
        assert!(app.status_message.starts_with("Logging to "));
        assert!(app.status_message.contains("tuigps_"));

        app.handle_gpsd_event(tpv("2024-01-15T12:30:00Z", 1.0));
        app.handle_gpsd_event(tpv("2024-01-15T12:30:01Z", 2.0));
        app.handle_event(key_event(KeyCode::Char('l')));
        assert!(
            app.status_message.contains("2 points"),
            "{}",
            app.status_message
        );
    }

    #[test]
    fn test_logging_start_failure_reported() {
        let mut app = new_app();
        app.log_dir = PathBuf::from("/nonexistent/definitely/not/here");
        app.handle_event(key_event(KeyCode::Char('l')));
        assert!(app.logger.is_none());
        assert!(app.status_message.contains("Failed to start logging"));
    }

    #[test]
    fn test_stop_logging_on_quit_closes_gpx() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = new_app();
        app.log_dir = dir.path().to_path_buf();
        app.handle_event(key_event(KeyCode::Char('l')));
        let filename = app.logger.as_ref().unwrap().filename.clone();
        app.handle_event(key_event(KeyCode::Char('q')));
        assert!(app.should_quit);
        // What run() does after the loop exits
        app.stop_logging();
        assert!(app.logger.is_none());
        let contents = std::fs::read_to_string(&filename).unwrap();
        assert!(contents.trim_end().ends_with("</gpx>"));
    }

    #[test]
    fn test_maps_without_fix_reports() {
        let mut app = new_app();
        assert!(app.maps_url().is_none());
        app.handle_event(key_event(KeyCode::Char('m')));
        assert!(app.status_message.contains("No fix"));

        app.handle_gpsd_event(tpv("2024-01-15T12:30:00Z", 1.0));
        assert_eq!(
            app.maps_url().as_deref(),
            Some("https://www.google.com/maps?q=51.5,-0.1")
        );
    }
}
