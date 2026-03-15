use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEventKind};
use futures::StreamExt;
use ratatui::prelude::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

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
}

#[derive(Debug, Clone, Copy, PartialEq)]
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

    // NMEA state
    pub nmea_buffer: VecDeque<String>,
    pub nmea_paused: bool,
    pub nmea_filter: String,

    // Device config state
    pub device_config_log: Vec<String>,

    // Clock sync
    pub armed_clock_set: bool,
    pub armed_toff: Arc<AtomicBool>,
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
            nmea_buffer: VecDeque::with_capacity(1000),
            nmea_paused: false,
            nmea_filter: String::new(),
            device_config_log: Vec::new(),
            armed_clock_set: false,
            armed_toff: Arc::new(AtomicBool::new(false)),
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
                KeyCode::Char('s') => self.show_settings = true,
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
                self.gps_data = data;

                // Log if active
                if let Some(ref mut logger) = self.logger {
                    let _ = logger.log_point(&self.gps_data);
                }

                // Position hold
                if let Some(ref mut hold) = self.position_hold {
                    if self.gps_data.has_fix() {
                        hold.add_fix(
                            self.gps_data.latitude,
                            self.gps_data.longitude,
                            self.gps_data.alt_msl,
                        );
                    }
                }
            }
            GpsdEvent::Error(msg) => {
                self.gps_data.connected = false;
                self.gps_data.error_message = msg;
            }
            GpsdEvent::Nmea(sentence) => {
                if !self.nmea_paused {
                    if self.nmea_buffer.len() >= 1000 {
                        self.nmea_buffer.pop_front();
                    }
                    self.nmea_buffer.push_back(sentence);
                }
            }
        }
    }

    pub fn tick(&mut self) {
        // Staleness detection could go here
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
        self.units = match self.units {
            UnitSystem::Metric => UnitSystem::Imperial,
            UnitSystem::Imperial => UnitSystem::Nautical,
            UnitSystem::Nautical => UnitSystem::Metric,
        };
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
            match key.code {
                KeyCode::Esc => self.show_settings = false,
                _ => {}
            }
        }
    }

    fn handle_tab_input(&mut self, key: crossterm::event::KeyEvent) {
        match self.active_tab {
            ActiveTab::Nmea => match key.code {
                KeyCode::Char('p') => self.nmea_paused = !self.nmea_paused,
                KeyCode::Char('c') => self.nmea_buffer.clear(),
                _ => {}
            },
            ActiveTab::Timing => match key.code {
                KeyCode::Char('a') => {
                    self.armed_toff.store(true, Ordering::SeqCst);
                }
                _ => {}
            },
            _ => {}
        }
    }
}

pub async fn run(terminal: &mut Terminal<impl Backend>, host: &str, port: u16) -> Result<()> {
    let mut app = App::new(host.to_string(), port);

    // Channel for gpsd events
    let (tx, mut rx) = mpsc::channel(100);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Spawn gpsd task
    let gpsd_handle = tokio::spawn(gpsd_client::gpsd_task(
        host.to_string(),
        port,
        tx.clone(),
        shutdown_rx,
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

        terminal.draw(|f| ui::render(f, &app))?;

        if app.should_quit {
            let _ = shutdown_tx.send(true);
            break;
        }
    }

    gpsd_handle.abort();
    Ok(())
}
