use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::prelude::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

use crate::clock_sync;
use crate::data_model::GPSData;
use crate::gps_logger::{GpsLogger, LogFormat};
use crate::gpsd_client::{self, GpsdEvent};
use crate::position_hold::PositionHold;
use crate::ui;

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

#[derive(Debug, Clone, Copy, PartialEq)]
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
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
}

// NMEA filter types to cycle through
const NMEA_FILTERS: &[&str] = &["", "GGA", "RMC", "GSA", "GSV", "VTG", "GLL", "ZDA"];

// Settings overlay field indices
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsField {
    Host,
    Port,
    Units,
    CoordFormat,
}

impl SettingsField {
    const ALL: &[SettingsField] = &[
        SettingsField::Host,
        SettingsField::Port,
        SettingsField::Units,
        SettingsField::CoordFormat,
    ];

    fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let all = Self::ALL;
        let idx = all.iter().position(|f| *f == self).unwrap_or(0);
        all[(idx + all.len() - 1) % all.len()]
    }
}

// Device configuration state
pub struct DeviceConfigState {
    pub nav_rate_idx: usize,
    pub power_mode_idx: usize,
    pub serial_speed_idx: usize,
    pub pps_frequency_idx: usize,
    pub gnss_enabled: [bool; 6], // GPS, GLONASS, Galileo, BeiDou, SBAS, QZSS
    pub raw_command: String,
    pub output_log: Vec<String>,
    pub selected_control: usize,
    pub proto_version: String,
}

pub const NAV_RATES: &[(&str, u32)] = &[
    ("1 Hz", 1000),
    ("2 Hz", 500),
    ("5 Hz", 200),
    ("10 Hz", 100),
];

pub const POWER_MODES: &[(&str, u8)] = &[
    ("Full Power", 0),
    ("Balanced", 1),
    ("1Hz Interval", 2),
    ("2Hz Interval", 3),
    ("4Hz Interval", 4),
];

pub const SERIAL_SPEEDS: &[u32] = &[4800, 9600, 19200, 38400, 57600, 115200, 230400];

pub const PPS_FREQUENCIES: &[(&str, u32)] = &[
    ("1 Hz", 1),
    ("2 Hz", 2),
    ("5 Hz", 5),
    ("10 Hz", 10),
];

pub const GNSS_NAMES_CONFIG: &[&str] = &["GPS", "GLONASS", "Galileo", "BeiDou", "SBAS", "QZSS"];

// Names used by ubxtool -e/-d flags
const GNSS_UBXTOOL_NAMES: &[&str] = &["GPS", "GLONASS", "GALILEO", "BEIDOU", "SBAS", "QZSS"];

// Total number of device config controls
pub const DEVICE_CONTROL_COUNT: usize = 11; // nav_rate, power, serial, pps, 6 GNSS toggles, save

impl DeviceConfigState {
    pub fn new() -> Self {
        Self {
            nav_rate_idx: 0,
            power_mode_idx: 0,
            serial_speed_idx: 1, // 9600
            pps_frequency_idx: 0,
            gnss_enabled: [true; 6],
            raw_command: String::new(),
            output_log: Vec::new(),
            selected_control: 0,
            proto_version: "18".to_string(),
        }
    }
}

pub struct App {
    pub gps_data: GPSData,
    pub active_tab: ActiveTab,
    pub units: UnitSystem,
    pub coord_format: CoordFormat,
    pub should_quit: bool,
    pub show_settings: bool,

    // Subsystems
    pub logger: Option<GpsLogger>,
    pub position_hold: Option<PositionHold>,

    // Settings
    pub host: String,
    pub port: u16,
    pub settings_field: SettingsField,
    pub settings_editing: bool,
    pub settings_edit_buf: String,

    // NMEA state
    pub nmea_buffer: VecDeque<String>,
    pub nmea_paused: bool,
    pub nmea_pause_buffer: VecDeque<String>,
    pub nmea_filter: String,
    pub nmea_filter_idx: usize,
    pub nmea_scroll_offset: usize,

    // Satellite table scroll
    pub sat_scroll_offset: usize,

    // Device config state
    pub device_config: DeviceConfigState,

    // Clock sync
    pub armed_clock_set: bool,
    pub armed_toff: Arc<AtomicBool>,

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
}

impl App {
    pub fn new(host: String, port: u16) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        Self {
            gps_data: GPSData::default(),
            active_tab: ActiveTab::Dashboard,
            units: UnitSystem::Metric,
            coord_format: CoordFormat::DD,
            should_quit: false,
            show_settings: false,
            logger: None,
            position_hold: None,
            host,
            port,
            settings_field: SettingsField::Host,
            settings_editing: false,
            settings_edit_buf: String::new(),
            nmea_buffer: VecDeque::with_capacity(1000),
            nmea_paused: false,
            nmea_pause_buffer: VecDeque::with_capacity(1000),
            nmea_filter: String::new(),
            nmea_filter_idx: 0,
            nmea_scroll_offset: 0,
            sat_scroll_offset: 0,
            device_config: DeviceConfigState::new(),
            armed_clock_set: false,
            armed_toff: Arc::new(AtomicBool::new(false)),
            stale: false,
            stale_seconds: 0.0,
            reconnect_requested: false,
            status_message: String::new(),
            status_message_tick: 0,
            cmd_tx: cmd_tx,
            cmd_rx: cmd_rx,
        }
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = msg.into();
        self.status_message_tick = 5; // show for 5 ticks (seconds)
    }

    pub fn handle_event(&mut self, event: Event) {
        if self.show_settings {
            self.handle_settings_input(event);
            return;
        }

        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                return;
            }
            match key.code {
                KeyCode::Char('q') => self.should_quit = true,
                KeyCode::Char('r') => self.reconnect_requested = true,
                KeyCode::Char('s') => {
                    self.show_settings = true;
                    self.settings_editing = false;
                    self.settings_field = SettingsField::Host;
                }
                KeyCode::Char('u') => self.cycle_units(),
                KeyCode::Char('m') => self.open_maps(),
                KeyCode::Char('l') => self.toggle_logging(),
                KeyCode::Char('h') => self.toggle_hold(),
                KeyCode::Tab => self.next_tab(),
                KeyCode::BackTab => self.prev_tab(),
                _ => self.handle_tab_input(key),
            }
        }
    }

    pub fn handle_gpsd_event(&mut self, event: GpsdEvent) {
        match event {
            GpsdEvent::Update(data) => {
                // Preserve app-managed TOFF state across gpsd updates
                let toff_samples = std::mem::take(&mut self.gps_data.toff_samples);
                let toff_armed_offset = self.gps_data.toff_armed_offset;
                let toff_armed_gps_time =
                    std::mem::take(&mut self.gps_data.toff_armed_gps_time);
                let toff_armed_sys_time = self.gps_data.toff_armed_sys_time;
                let prev_time = std::mem::take(&mut self.gps_data.time);

                self.gps_data = *data;

                // Restore app-managed TOFF state
                self.gps_data.toff_samples = toff_samples;
                self.gps_data.toff_armed_offset = toff_armed_offset;
                self.gps_data.toff_armed_gps_time = toff_armed_gps_time;
                self.gps_data.toff_armed_sys_time = toff_armed_sys_time;

                self.stale = false;
                self.stale_seconds = 0.0;

                // Compute TOFF offset only when GPS time changes (new TPV)
                let new_time = &self.gps_data.time;
                if !new_time.is_empty()
                    && *new_time != prev_time
                    && self.gps_data.last_seen > 0.0
                {
                    if let Ok(gps_time) =
                        chrono::DateTime::parse_from_rfc3339(new_time)
                    {
                        let gps_epoch = gps_time.timestamp() as f64
                            + gps_time.timestamp_subsec_nanos() as f64 / 1e9;
                        let offset = gps_epoch - self.gps_data.last_seen;

                        // Accumulate in circular buffer (max 20)
                        if self.gps_data.toff_samples.len() >= 20 {
                            self.gps_data.toff_samples.remove(0);
                        }
                        self.gps_data.toff_samples.push(offset);

                        // Check for armed single-shot TOFF capture
                        if self
                            .armed_toff
                            .compare_exchange(
                                true,
                                false,
                                Ordering::SeqCst,
                                Ordering::Relaxed,
                            )
                            .is_ok()
                        {
                            self.gps_data.toff_armed_offset = offset;
                            self.gps_data.toff_armed_gps_time =
                                self.gps_data.time.clone();
                            self.gps_data.toff_armed_sys_time =
                                self.gps_data.last_seen;
                            self.set_status(format!(
                                "TOFF captured: {}",
                                crate::formatting::fmt_offset(offset)
                            ));
                        }
                    }
                }

                // Armed clock sync — fires on fresh TPV for minimum latency
                if self.armed_clock_set
                    && !self.gps_data.time.is_empty()
                    && self.gps_data.last_seen > 0.0
                {
                    self.armed_clock_set = false;
                    match clock_sync::set_clock_from_gps(
                        &self.gps_data.time,
                        self.gps_data.last_seen,
                    ) {
                        Ok(msg) => self.set_status(&msg),
                        Err(e) => self.set_status(format!("Clock sync error: {}", e)),
                    }
                }

                // Log if active
                if let Some(ref mut logger) = self.logger {
                    let _ = logger.log_point(&self.gps_data);
                }

                // Position hold
                if let Some(ref mut hold) = self.position_hold
                    && self.gps_data.has_fix()
                {
                    hold.add_fix(
                        self.gps_data.latitude,
                        self.gps_data.longitude,
                        self.gps_data.alt_msl,
                    );
                }
            }
            GpsdEvent::Error(msg) => {
                self.gps_data.connected = false;
                self.gps_data.error_message = msg;
            }
            GpsdEvent::Nmea(sentence) => {
                if self.nmea_paused {
                    if self.nmea_pause_buffer.len() >= 1000 {
                        self.nmea_pause_buffer.pop_front();
                    }
                    self.nmea_pause_buffer.push_back(sentence);
                } else {
                    if self.nmea_buffer.len() >= 1000 {
                        self.nmea_buffer.pop_front();
                    }
                    self.nmea_buffer.push_back(sentence);
                }
            }
        }
    }

    pub fn tick(&mut self) {
        // Staleness detection
        if self.gps_data.last_seen > 0.0 {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            let age = now - self.gps_data.last_seen;
            if age > 10.0 {
                self.stale = true;
                self.stale_seconds = age;
            }
        }

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

    fn open_maps(&self) {
        if self.gps_data.has_fix() {
            let url = format!(
                "https://www.google.com/maps?q={},{}",
                self.gps_data.latitude, self.gps_data.longitude
            );
            let _ = open::that(url);
        }
    }

    fn toggle_logging(&mut self) {
        if let Some(ref mut logger) = self.logger {
            if logger.active {
                let _ = logger.stop();
                self.logger = None;
            }
        } else {
            let mut logger = GpsLogger::new(LogFormat::Gpx);
            if logger.start().is_ok() {
                self.logger = Some(logger);
            }
        }
    }

    fn toggle_hold(&mut self) {
        if self.position_hold.is_some() {
            self.position_hold = None;
        } else {
            self.position_hold = Some(PositionHold::new());
        }
    }

    fn handle_settings_input(&mut self, event: Event) {
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                return;
            }

            if self.settings_editing {
                // Text input mode
                match key.code {
                    KeyCode::Esc => {
                        self.settings_editing = false;
                    }
                    KeyCode::Enter => {
                        // Apply the edit
                        match self.settings_field {
                            SettingsField::Host => {
                                if !self.settings_edit_buf.is_empty() {
                                    self.host = self.settings_edit_buf.clone();
                                }
                            }
                            SettingsField::Port => {
                                if let Ok(p) = self.settings_edit_buf.parse::<u16>() {
                                    self.port = p;
                                }
                            }
                            _ => {}
                        }
                        self.settings_editing = false;
                    }
                    KeyCode::Backspace => {
                        self.settings_edit_buf.pop();
                    }
                    KeyCode::Char(c) => {
                        self.settings_edit_buf.push(c);
                    }
                    _ => {}
                }
                return;
            }

            // Navigation mode
            match key.code {
                KeyCode::Esc => self.show_settings = false,
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // Apply and close
                    self.reconnect_requested = true;
                    self.show_settings = false;
                }
                KeyCode::Up | KeyCode::BackTab => {
                    self.settings_field = self.settings_field.prev();
                }
                KeyCode::Down | KeyCode::Tab => {
                    self.settings_field = self.settings_field.next();
                }
                KeyCode::Enter => {
                    match self.settings_field {
                        SettingsField::Host => {
                            self.settings_editing = true;
                            self.settings_edit_buf = self.host.clone();
                        }
                        SettingsField::Port => {
                            self.settings_editing = true;
                            self.settings_edit_buf = self.port.to_string();
                        }
                        SettingsField::Units => {
                            self.units = self.units.next();
                        }
                        SettingsField::CoordFormat => {
                            self.coord_format = self.coord_format.next();
                        }
                    }
                }
                KeyCode::Left | KeyCode::Right => {
                    // Cycle values for Units/CoordFormat
                    match self.settings_field {
                        SettingsField::Units => self.units = self.units.next(),
                        SettingsField::CoordFormat => self.coord_format = self.coord_format.next(),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    fn handle_tab_input(&mut self, key: crossterm::event::KeyEvent) {
        match self.active_tab {
            ActiveTab::Nmea => match key.code {
                KeyCode::Char('p') => {
                    if self.nmea_paused {
                        // Unpause: merge pause buffer into main buffer
                        while let Some(s) = self.nmea_pause_buffer.pop_front() {
                            if self.nmea_buffer.len() >= 1000 {
                                self.nmea_buffer.pop_front();
                            }
                            self.nmea_buffer.push_back(s);
                        }
                    }
                    self.nmea_paused = !self.nmea_paused;
                    self.nmea_scroll_offset = 0;
                }
                KeyCode::Char('c') => {
                    self.nmea_buffer.clear();
                    self.nmea_pause_buffer.clear();
                    self.nmea_scroll_offset = 0;
                    self.set_status("NMEA buffer cleared");
                }
                KeyCode::Char('f') => {
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
                KeyCode::Up => {
                    self.nmea_scroll_offset = self.nmea_scroll_offset.saturating_add(1);
                }
                KeyCode::Down => {
                    self.nmea_scroll_offset = self.nmea_scroll_offset.saturating_sub(1);
                }
                KeyCode::PageUp => {
                    self.nmea_scroll_offset = self.nmea_scroll_offset.saturating_add(20);
                }
                KeyCode::PageDown => {
                    self.nmea_scroll_offset = self.nmea_scroll_offset.saturating_sub(20);
                }
                _ => {}
            },
            ActiveTab::Timing => match key.code {
                KeyCode::Char('a') => {
                    self.armed_toff.store(true, Ordering::SeqCst);
                    self.set_status("TOFF armed — waiting for next fix...");
                }
                KeyCode::Char('c') => {
                    let had_data = !self.gps_data.toff_samples.is_empty()
                        || self.gps_data.toff_armed_offset.is_finite();
                    self.gps_data.toff_samples.clear();
                    self.gps_data.toff_armed_offset = f64::NAN;
                    self.gps_data.toff_armed_gps_time.clear();
                    self.gps_data.toff_armed_sys_time = f64::NAN;
                    self.armed_toff.store(false, Ordering::SeqCst);
                    if had_data {
                        self.set_status("TOFF data cleared");
                    } else {
                        self.set_status("TOFF data already empty");
                    }
                }
                KeyCode::Char('k') => {
                    // Toggle armed clock sync
                    if self.armed_clock_set {
                        self.armed_clock_set = false;
                        self.set_status("Clock sync disarmed");
                    } else {
                        self.armed_clock_set = true;
                        self.set_status("Clock sync ARMED — will fire on next GPS fix");
                    }
                }
                _ => {}
            },
            ActiveTab::Satellites => match key.code {
                KeyCode::Up => {
                    self.sat_scroll_offset = self.sat_scroll_offset.saturating_sub(1);
                }
                KeyCode::Down => {
                    self.sat_scroll_offset = self.sat_scroll_offset.saturating_add(1);
                }
                KeyCode::PageUp => {
                    self.sat_scroll_offset = self.sat_scroll_offset.saturating_sub(20);
                }
                KeyCode::PageDown => {
                    self.sat_scroll_offset = self.sat_scroll_offset.saturating_add(20);
                }
                _ => {}
            },
            ActiveTab::Device => match key.code {
                KeyCode::Up => {
                    if self.device_config.selected_control > 0 {
                        self.device_config.selected_control -= 1;
                    }
                }
                KeyCode::Down => {
                    if self.device_config.selected_control < DEVICE_CONTROL_COUNT - 1 {
                        self.device_config.selected_control += 1;
                    }
                }
                KeyCode::Left => {
                    self.device_config_adjust(-1);
                }
                KeyCode::Right => {
                    self.device_config_adjust(1);
                }
                KeyCode::Enter => {
                    self.device_config_activate();
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn device_config_adjust(&mut self, dir: i32) {
        let dc = &mut self.device_config;
        match dc.selected_control {
            0 => {
                // Nav rate
                dc.nav_rate_idx =
                    (dc.nav_rate_idx as i32 + dir).rem_euclid(NAV_RATES.len() as i32) as usize;
            }
            1 => {
                // Power mode
                dc.power_mode_idx = (dc.power_mode_idx as i32 + dir)
                    .rem_euclid(POWER_MODES.len() as i32)
                    as usize;
            }
            2 => {
                // Serial speed
                dc.serial_speed_idx = (dc.serial_speed_idx as i32 + dir)
                    .rem_euclid(SERIAL_SPEEDS.len() as i32)
                    as usize;
            }
            3 => {
                // PPS frequency
                dc.pps_frequency_idx = (dc.pps_frequency_idx as i32 + dir)
                    .rem_euclid(PPS_FREQUENCIES.len() as i32)
                    as usize;
            }
            4..=9 => {
                // GNSS toggles
                let idx = dc.selected_control - 4;
                dc.gnss_enabled[idx] = !dc.gnss_enabled[idx];
            }
            _ => {}
        }
    }

    fn device_config_activate(&mut self) {
        let dc = &mut self.device_config;
        let proto = dc.proto_version.clone();
        match dc.selected_control {
            0 => {
                // Nav rate
                let (name, ms) = NAV_RATES[dc.nav_rate_idx];
                dc.output_log.push(format!("Setting nav rate: {} ({}ms)...", name, ms));
                self.run_ubxtool(&["-p", &format!("CFG-RATE,{}", ms)], &proto);
            }
            1 => {
                // Power mode
                let (name, mode) = POWER_MODES[dc.power_mode_idx];
                dc.output_log.push(format!("Setting power mode: {}...", name));
                self.run_ubxtool(&["-p", &format!("PMS,{}", mode)], &proto);
            }
            2 => {
                // Serial speed (two-step: ubxtool then gpsctl)
                let speed = SERIAL_SPEEDS[dc.serial_speed_idx];
                let device = self.gps_data.device.path.clone();
                dc.output_log.push(format!("Setting baud rate to {}...", speed));
                self.run_baud_rate_change(speed, &device, &proto);
            }
            3 => {
                // PPS frequency
                let (name, freq) = PPS_FREQUENCIES[dc.pps_frequency_idx];
                dc.output_log.push(format!("Setting PPS: {}...", name));
                let cmd = build_tp5_cmd(freq, 100000, true);
                self.run_ubxtool(&["-c", &cmd], &proto);
            }
            4..=9 => {
                // GNSS toggles
                let idx = dc.selected_control - 4;
                let name = GNSS_NAMES_CONFIG[idx];
                let ubx_name = GNSS_UBXTOOL_NAMES[idx];
                let enabled = dc.gnss_enabled[idx];
                let flag = if enabled { "-e" } else { "-d" };
                let state = if enabled { "Enabling" } else { "Disabling" };
                dc.output_log.push(format!("{} {}...", state, name));
                self.run_ubxtool(&[flag, ubx_name], &proto);
            }
            10 => {
                // Save config
                dc.output_log.push("Saving config to flash...".to_string());
                self.run_ubxtool(&["-p", "SAVE"], &proto);
            }
            _ => {}
        }
    }

    fn run_ubxtool(&self, args: &[&str], proto_ver: &str) {
        let tx = self.cmd_tx.clone();
        let full_args: Vec<String> = ["-P", proto_ver]
            .iter()
            .chain(args.iter())
            .map(|s| s.to_string())
            .collect();
        let cmd_str = format!("$ ubxtool {}", full_args.join(" "));
        let _ = tx.try_send(cmd_str);

        tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                tokio::process::Command::new("ubxtool")
                    .args(&full_args)
                    .output(),
            )
            .await;

            match result {
                Ok(Ok(output)) => {
                    let mut result =
                        String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let stderr =
                        String::from_utf8_lossy(&output.stderr).trim().to_string();
                    if !stderr.is_empty() {
                        if !result.is_empty() {
                            result.push('\n');
                        }
                        result.push_str(&stderr);
                    }
                    if result.is_empty() {
                        result = "(no output)".to_string();
                    }
                    let _ = tx.send(result).await;
                }
                Ok(Err(e)) => {
                    let msg = if e.kind() == std::io::ErrorKind::NotFound {
                        "Error: ubxtool not found (install gpsd-clients)".to_string()
                    } else {
                        format!("Error: {}", e)
                    };
                    let _ = tx.send(msg).await;
                }
                Err(_) => {
                    let _ = tx.send("Error: command timed out (10s)".to_string()).await;
                }
            }
        });
    }

    fn run_baud_rate_change(&self, speed: u32, device: &str, proto_ver: &str) {
        let tx = self.cmd_tx.clone();
        let proto = proto_ver.to_string();
        let device = device.to_string();
        let speed_str = speed.to_string();

        tokio::spawn(async move {
            // Step 1: Tell receiver to switch baud rate
            let step1 = tokio::time::timeout(
                Duration::from_secs(10),
                tokio::process::Command::new("ubxtool")
                    .args(["-P", &proto, "-S", &speed_str])
                    .output(),
            )
            .await;

            match step1 {
                Ok(Ok(output)) => {
                    let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    let combined =
                        if err.is_empty() { out } else { format!("{}\n{}", out, err) };
                    if !combined.trim().is_empty() {
                        let _ = tx.send(combined).await;
                    }
                }
                Ok(Err(e)) => {
                    let _ = tx.send(format!("ubxtool error: {}", e)).await;
                    return;
                }
                Err(_) => {
                    let _ = tx
                        .send("ubxtool timed out (10s)".to_string())
                        .await;
                    return;
                }
            }

            // Step 2: Tell gpsd the new speed
            let mut gpsctl_args = vec!["-s".to_string(), speed_str.clone()];
            if !device.is_empty() {
                gpsctl_args.push(device.clone());
            }
            let step2 = tokio::time::timeout(
                Duration::from_secs(10),
                tokio::process::Command::new("gpsctl")
                    .args(&gpsctl_args)
                    .output(),
            )
            .await;

            match step2 {
                Ok(Ok(output)) => {
                    if output.status.success() {
                        let dev_name = if device.is_empty() {
                            "default device".to_string()
                        } else {
                            device
                        };
                        let _ = tx
                            .send(format!("Baud rate set to {} on {}", speed_str, dev_name))
                            .await;
                    } else {
                        let err =
                            String::from_utf8_lossy(&output.stderr).trim().to_string();
                        let _ = tx.send(format!("gpsctl error: {}", err)).await;
                    }
                }
                Ok(Err(e)) => {
                    let _ = tx.send(format!("gpsctl error: {}", e)).await;
                }
                Err(_) => {
                    let _ = tx
                        .send("gpsctl timed out (10s)".to_string())
                        .await;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key_event(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn key_event_mod(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn new_app() -> App {
        App::new("localhost".to_string(), 2947)
    }

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
        let mut app = new_app();
        assert!(app.logger.is_none());
        app.handle_event(key_event(KeyCode::Char('l')));
        // Logger should be Some if file creation succeeded, None if it failed
        // (depends on filesystem, so just verify no crash)
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

    // === Settings overlay ===

    #[test]
    fn test_settings_field_navigation() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s'))); // open settings
        assert_eq!(app.settings_field, SettingsField::Host);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.settings_field, SettingsField::Port);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.settings_field, SettingsField::Units);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.settings_field, SettingsField::CoordFormat);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.settings_field, SettingsField::Host); // wraps

        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.settings_field, SettingsField::CoordFormat); // wraps back
    }

    #[test]
    fn test_settings_edit_host() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s'))); // open
        assert_eq!(app.settings_field, SettingsField::Host);

        app.handle_event(key_event(KeyCode::Enter)); // start editing
        assert!(app.settings_editing);
        assert_eq!(app.settings_edit_buf, "localhost");

        // Clear and type new host
        for _ in 0..9 {
            app.handle_event(key_event(KeyCode::Backspace));
        }
        for c in "10.0.0.1".chars() {
            app.handle_event(key_event(KeyCode::Char(c)));
        }
        assert_eq!(app.settings_edit_buf, "10.0.0.1");

        app.handle_event(key_event(KeyCode::Enter)); // apply
        assert!(!app.settings_editing);
        assert_eq!(app.host, "10.0.0.1");
    }

    #[test]
    fn test_settings_edit_cancel() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event(KeyCode::Enter)); // start editing host
        app.handle_event(key_event(KeyCode::Backspace));
        app.handle_event(key_event(KeyCode::Esc)); // cancel
        assert!(!app.settings_editing);
        assert_eq!(app.host, "localhost"); // unchanged
    }

    #[test]
    fn test_settings_cycle_units() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s'))); // open
        // Navigate to Units field
        app.handle_event(key_event(KeyCode::Down)); // Port
        app.handle_event(key_event(KeyCode::Down)); // Units
        assert_eq!(app.settings_field, SettingsField::Units);

        app.handle_event(key_event(KeyCode::Enter));
        assert_eq!(app.units, UnitSystem::Imperial);

        app.handle_event(key_event(KeyCode::Right));
        assert_eq!(app.units, UnitSystem::Nautical);
    }

    #[test]
    fn test_settings_cycle_coord_format() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s'))); // open
        // Navigate to CoordFormat
        app.handle_event(key_event(KeyCode::Down)); // Port
        app.handle_event(key_event(KeyCode::Down)); // Units
        app.handle_event(key_event(KeyCode::Down)); // CoordFormat
        assert_eq!(app.settings_field, SettingsField::CoordFormat);

        app.handle_event(key_event(KeyCode::Enter));
        assert_eq!(app.coord_format, CoordFormat::DMS);
    }

    #[test]
    fn test_settings_ctrl_s() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s'))); // open
        assert!(app.show_settings);

        app.handle_event(key_event_mod(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        ));
        assert!(!app.show_settings);
        assert!(app.reconnect_requested);
    }

    #[test]
    fn test_settings_blocks_global_keys() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s'))); // open settings
        assert!(app.show_settings);

        // 'q' should NOT quit while settings is open
        app.handle_event(key_event(KeyCode::Char('q')));
        assert!(!app.should_quit);
        assert!(app.show_settings);
    }

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

    #[test]
    fn test_nmea_scroll() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Nmea;

        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.nmea_scroll_offset, 1);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.nmea_scroll_offset, 0);

        app.handle_event(key_event(KeyCode::PageUp));
        assert_eq!(app.nmea_scroll_offset, 20);

        app.handle_event(key_event(KeyCode::PageDown));
        assert_eq!(app.nmea_scroll_offset, 0);
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

    // === Timing tab ===

    #[test]
    fn test_timing_arm_toff() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;

        app.handle_event(key_event(KeyCode::Char('a')));
        assert!(app.armed_toff.load(Ordering::SeqCst));
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
        assert!(app.armed_toff.load(Ordering::SeqCst));
        assert!(app.status_message.contains("armed"));
    }

    #[test]
    fn test_timing_clock_sync_no_gps() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;

        app.handle_event(key_event(KeyCode::Char('k')));
        assert!(app.status_message.contains("No GPS time"));
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

    // === Satellites tab ===

    #[test]
    fn test_satellite_scroll() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Satellites;

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.sat_scroll_offset, 1);

        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.sat_scroll_offset, 0);

        // Can't scroll below 0
        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.sat_scroll_offset, 0);
    }

    // === Device tab ===

    #[test]
    fn test_device_control_navigation() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Device;
        assert_eq!(app.device_config.selected_control, 0);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.device_config.selected_control, 1);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.device_config.selected_control, 2);

        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.device_config.selected_control, 1);

        // Can't go below 0
        app.handle_event(key_event(KeyCode::Up));
        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.device_config.selected_control, 0);
    }

    #[test]
    fn test_device_adjust_nav_rate() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Device;
        app.device_config.selected_control = 0; // Nav rate
        assert_eq!(app.device_config.nav_rate_idx, 0);

        app.handle_event(key_event(KeyCode::Right));
        assert_eq!(app.device_config.nav_rate_idx, 1);

        app.handle_event(key_event(KeyCode::Left));
        assert_eq!(app.device_config.nav_rate_idx, 0);

        // Wraps
        app.handle_event(key_event(KeyCode::Left));
        assert_eq!(app.device_config.nav_rate_idx, NAV_RATES.len() - 1);
    }

    #[test]
    fn test_device_gnss_toggle() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Device;
        app.device_config.selected_control = 4; // First GNSS toggle (GPS)
        assert!(app.device_config.gnss_enabled[0]);

        app.handle_event(key_event(KeyCode::Right)); // toggle
        assert!(!app.device_config.gnss_enabled[0]);

        app.handle_event(key_event(KeyCode::Left)); // toggle back
        assert!(app.device_config.gnss_enabled[0]);
    }

    #[test]
    fn test_device_activate() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Device;
        app.device_config.selected_control = 0;
        assert!(app.device_config.output_log.is_empty());

        app.handle_event(key_event(KeyCode::Enter));
        assert_eq!(app.device_config.output_log.len(), 1);
        assert!(app.device_config.output_log[0].contains("nav rate"));
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

    #[test]
    fn test_timing_keys_only_on_timing_tab() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Dashboard;

        // 'a' on Dashboard should not arm TOFF
        app.handle_event(key_event(KeyCode::Char('a')));
        assert!(!app.armed_toff.load(Ordering::SeqCst));
    }

    // === Staleness detection ===

    #[test]
    fn test_staleness() {
        let mut app = new_app();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

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

    // === GPS event handling ===

    #[test]
    fn test_gpsd_update_clears_stale() {
        let mut app = new_app();
        app.stale = true;
        app.stale_seconds = 15.0;

        app.handle_gpsd_event(GpsdEvent::Update(Box::new(GPSData::default())));
        assert!(!app.stale);
        assert_eq!(app.stale_seconds, 0.0);
    }

    #[test]
    fn test_toff_accumulation_in_app() {
        let mut app = new_app();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        // Send update with GPS time
        let mut data = GPSData::default();
        data.time = "2024-01-15T12:30:00.000Z".to_string();
        data.last_seen = now;
        data.connected = true;
        app.handle_gpsd_event(GpsdEvent::Update(Box::new(data)));

        assert_eq!(app.gps_data.toff_samples.len(), 1);
    }

    #[test]
    fn test_toff_circular_buffer_in_app() {
        let mut app = new_app();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        // Send 25 updates to overflow the 20-element buffer
        for i in 0..25 {
            let mut data = GPSData::default();
            data.time = format!("2024-01-15T12:30:{:02}.000Z", i % 60);
            data.last_seen = now + i as f64;
            data.connected = true;
            app.handle_gpsd_event(GpsdEvent::Update(Box::new(data)));
        }

        assert_eq!(app.gps_data.toff_samples.len(), 20); // capped
    }

    #[test]
    fn test_toff_arm_and_capture() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        // Arm the TOFF
        app.handle_event(key_event(KeyCode::Char('a')));
        assert!(app.armed_toff.load(Ordering::SeqCst));

        // Send a GPS update with time
        let mut data = GPSData::default();
        data.time = "2024-01-15T12:30:00.000Z".to_string();
        data.last_seen = now;
        data.connected = true;
        app.handle_gpsd_event(GpsdEvent::Update(Box::new(data)));

        // Armed TOFF should have fired
        assert!(!app.armed_toff.load(Ordering::SeqCst));
        assert!(app.gps_data.toff_armed_offset.is_finite());
        assert_eq!(app.gps_data.toff_armed_gps_time, "2024-01-15T12:30:00.000Z");
        assert!(app.status_message.contains("captured"));
    }

    #[test]
    fn test_toff_clear_persists_across_updates() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Timing;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        // Accumulate some TOFF data
        let mut data = GPSData::default();
        data.time = "2024-01-15T12:30:00.000Z".to_string();
        data.last_seen = now;
        data.connected = true;
        app.handle_gpsd_event(GpsdEvent::Update(Box::new(data)));
        assert!(!app.gps_data.toff_samples.is_empty());

        // Clear it
        app.handle_event(key_event(KeyCode::Char('c')));
        assert!(app.gps_data.toff_samples.is_empty());

        // Send another update WITHOUT time — toff_samples should stay empty
        let mut data2 = GPSData::default();
        data2.connected = true;
        data2.last_seen = now + 1.0;
        app.handle_gpsd_event(GpsdEvent::Update(Box::new(data2)));
        assert!(app.gps_data.toff_samples.is_empty()); // preserved!
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

/// Build a UBX-CFG-TP5 command string for `ubxtool -c`.
/// Format: class,id,payload_bytes (comma-separated hex).
fn build_tp5_cmd(freq_hz: u32, pulse_us: u32, active: bool) -> String {
    // flags: lockGnssFreq | lockedOtherSet | isFreq | isLength | alignToTow | polarity
    let mut flags: u32 = 0x02 | 0x04 | 0x08 | 0x10 | 0x20 | 0x40; // 0x7E
    if active {
        flags |= 0x01;
    }

    // Pack as little-endian: BBHhhIIIIiI
    let mut payload = Vec::with_capacity(32);
    payload.push(0u8); // tpIdx
    payload.push(1u8); // version
    payload.extend_from_slice(&0u16.to_le_bytes()); // reserved
    payload.extend_from_slice(&0i16.to_le_bytes()); // antCableDelay
    payload.extend_from_slice(&0i16.to_le_bytes()); // rfGroupDelay
    payload.extend_from_slice(&freq_hz.to_le_bytes()); // freqPeriod
    payload.extend_from_slice(&freq_hz.to_le_bytes()); // freqPeriodLock
    payload.extend_from_slice(&pulse_us.to_le_bytes()); // pulseLenRatio
    payload.extend_from_slice(&pulse_us.to_le_bytes()); // pulseLenRatioLock
    payload.extend_from_slice(&0i32.to_le_bytes()); // userConfigDelay
    payload.extend_from_slice(&flags.to_le_bytes()); // flags

    let mut parts = vec!["06".to_string(), "31".to_string()];
    for b in &payload {
        parts.push(format!("{:02x}", b));
    }
    parts.join(",")
}

pub async fn run(terminal: &mut Terminal<impl Backend>, host: &str, port: u16) -> Result<()> {
    let mut app = App::new(host.to_string(), port);

    // Channel for gpsd events
    let (tx, mut rx) = mpsc::channel(100);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Spawn gpsd task
    let mut gpsd_handle = tokio::spawn(gpsd_client::gpsd_task(
        host.to_string(),
        port,
        tx.clone(),
        shutdown_rx.clone(),
    ));

    // Event stream
    let mut event_reader = crossterm::event::EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            Some(Ok(event)) = event_reader.next() => {
                app.handle_event(event);
            }
            Some(gpsd_event) = rx.recv() => {
                app.handle_gpsd_event(gpsd_event);
            }
            Some(cmd_output) = app.cmd_rx.recv() => {
                // Split multi-line output into separate log entries
                for line in cmd_output.lines() {
                    if !line.trim().is_empty() {
                        app.device_config.output_log.push(line.to_string());
                    }
                }
            }
            _ = tick.tick() => {
                app.tick();
            }
        }

        // Handle reconnect
        if app.reconnect_requested {
            app.reconnect_requested = false;
            // Abort old task and start new one
            gpsd_handle.abort();
            let _ = gpsd_handle.await;

            // Reset connection state
            app.gps_data.connected = false;
            app.gps_data.error_message = "Reconnecting...".to_string();
            app.stale = false;

            gpsd_handle = tokio::spawn(gpsd_client::gpsd_task(
                app.host.clone(),
                app.port,
                tx.clone(),
                shutdown_rx.clone(),
            ));
        }

        terminal.draw(|f| ui::render(f, &app))?;

        if app.should_quit {
            let _ = shutdown_tx.send(true);
            break;
        }
    }

    gpsd_handle.abort();
    Ok(())
}
