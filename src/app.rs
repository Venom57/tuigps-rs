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
}

impl App {
    pub fn new(host: String, port: u16) -> Self {
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
        }
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
                self.gps_data = *data;
                self.stale = false;
                self.stale_seconds = 0.0;

                // Log if active
                if let Some(ref mut logger) = self.logger {
                    let _ = logger.log_point(&self.gps_data);
                }

                // Position hold
                if let Some(ref mut hold) = self.position_hold
                    && self.gps_data.has_fix() {
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
                    self.nmea_scroll_offset = 0;
                }
                KeyCode::Char('f') => {
                    self.nmea_filter_idx = (self.nmea_filter_idx + 1) % NMEA_FILTERS.len();
                    self.nmea_filter = NMEA_FILTERS[self.nmea_filter_idx].to_string();
                    self.nmea_scroll_offset = 0;
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
                }
                KeyCode::Char('c') => {
                    self.gps_data.toff_samples.clear();
                    self.gps_data.toff_armed_offset = f64::NAN;
                    self.gps_data.toff_armed_gps_time.clear();
                    self.gps_data.toff_armed_sys_time = f64::NAN;
                }
                KeyCode::Char('k') => {
                    // Clock sync from GPS time
                    if !self.gps_data.time.is_empty() && self.gps_data.last_seen > 0.0 {
                        match clock_sync::set_clock_from_gps(
                            &self.gps_data.time,
                            self.gps_data.last_seen,
                        ) {
                            Ok(msg) => {
                                self.device_config.output_log.push(msg);
                            }
                            Err(e) => {
                                self.device_config
                                    .output_log
                                    .push(format!("Clock sync error: {}", e));
                            }
                        }
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
        match dc.selected_control {
            0 => {
                let (name, ms) = NAV_RATES[dc.nav_rate_idx];
                dc.output_log
                    .push(format!("Set nav rate: {} ({}ms)", name, ms));
            }
            1 => {
                let (name, mode) = POWER_MODES[dc.power_mode_idx];
                dc.output_log
                    .push(format!("Set power mode: {} ({})", name, mode));
            }
            2 => {
                let speed = SERIAL_SPEEDS[dc.serial_speed_idx];
                dc.output_log
                    .push(format!("Set serial speed: {} baud", speed));
            }
            3 => {
                let (name, freq) = PPS_FREQUENCIES[dc.pps_frequency_idx];
                dc.output_log
                    .push(format!("Set PPS frequency: {} ({})", name, freq));
            }
            4..=9 => {
                let idx = dc.selected_control - 4;
                let name = GNSS_NAMES_CONFIG[idx];
                let state = if dc.gnss_enabled[idx] {
                    "enabled"
                } else {
                    "disabled"
                };
                dc.output_log.push(format!("{}: {}", name, state));
            }
            10 => {
                dc.output_log.push("Save config requested".to_string());
            }
            _ => {}
        }
    }
}

pub async fn run(terminal: &mut Terminal<impl Backend>, host: &str, port: u16) -> Result<()> {
    let mut app = App::new(host.to_string(), port);

    // Channel for gpsd events
    let (tx, mut rx) = mpsc::channel(100);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Spawn gpsd task with armed TOFF
    let armed_toff = app.armed_toff.clone();
    let mut gpsd_handle = tokio::spawn(gpsd_client::gpsd_task(
        host.to_string(),
        port,
        tx.clone(),
        shutdown_rx.clone(),
        armed_toff.clone(),
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

            let new_armed = app.armed_toff.clone();
            gpsd_handle = tokio::spawn(gpsd_client::gpsd_task(
                app.host.clone(),
                app.port,
                tx.clone(),
                shutdown_rx.clone(),
                new_armed,
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
