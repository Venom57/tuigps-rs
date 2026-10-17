use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

use crate::clock_sync::{self, ClockSyncRequest};
use crate::config::{self, Config, Startup};
use crate::data_model::GPSData;
use crate::gps_logger::{GpsLogger, LogFormat};
use crate::gpsd_client::{self, GpsdEvent, GpsdMessage, MsgClass, TaggedEvent};
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

/// Maximum number of lines kept in the Device tab output log.
pub const OUTPUT_LOG_MAX: usize = 500;

/// Seconds without a TPV before data is flagged stale.
pub const STALE_AFTER_SECS: f64 = 10.0;

/// Minimum interval between redraws triggered by gpsd/command traffic (~20 fps).
const FRAME_INTERVAL: Duration = Duration::from_millis(50);

/// Upper bound on gpsd events drained per loop iteration before drawing.
const MAX_DRAIN: usize = 256;

/// Consecutive terminal input errors tolerated before giving up.
const MAX_INPUT_ERRORS: u32 = 20;

// NMEA filter types to cycle through
const NMEA_FILTERS: &[&str] = &["", "GGA", "RMC", "GSA", "GSV", "VTG", "GLL", "ZDA"];

// Settings overlay field indices
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsField {
    Host,
    Port,
    Units,
    CoordFormat,
    /// The "[ Save ]" button: Enter applies like Ctrl+S.
    Save,
}

impl SettingsField {
    pub const ALL: &[SettingsField] = &[
        SettingsField::Host,
        SettingsField::Port,
        SettingsField::Units,
        SettingsField::CoordFormat,
        SettingsField::Save,
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

/// Longest host name accepted by the settings dialog (DNS limit).
const MAX_HOST_LEN: usize = 253;

/// Validate a port typed into the settings dialog.
pub fn validate_port(text: &str) -> Result<u16, String> {
    if text.is_empty() {
        return Err("Port is empty: enter 1-65535".to_string());
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Port must be digits only".to_string());
    }
    match text.parse::<u32>() {
        Ok(p) if (1..=65535).contains(&p) => Ok(p as u16),
        _ => Err(format!("Invalid port {}: must be 1-65535", text)),
    }
}

// ---------------------------------------------------------------------------
// Key bindings: one table drives dispatch, the help overlay and the status
// bar hints, so they cannot drift apart.
// ---------------------------------------------------------------------------

/// What a key does. Some actions are resolved further by the key code
/// (e.g. `ScrollLine` is Up or Down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ToggleHelp,
    JumpTab,
    NextTab,
    PrevTab,
    Quit,
    Reconnect,
    OpenSettings,
    CycleUnits,
    OpenMaps,
    ToggleLog,
    ToggleHold,
    CloseOverlay,
    ScrollLine,
    ScrollPage,
    ScrollEnds,
    NmeaPause,
    NmeaFilter,
    NmeaClear,
    ToffArm,
    ToffClear,
    ClockSyncToggle,
    DeviceSelect,
    DeviceAdjust,
    DeviceApply,
    /// Documentation-only row (handled elsewhere, e.g. Ctrl+C).
    Info,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyBinding {
    /// Key codes that trigger the action (empty for documentation rows).
    pub keys: &'static [KeyCode],
    /// How the key is shown in help and the status bar.
    pub label: &'static str,
    /// Description for the help overlay.
    pub desc: &'static str,
    /// Short status-bar hint, if the binding is shown there.
    pub hint: Option<&'static str>,
    pub action: Action,
}

const fn kb(
    keys: &'static [KeyCode],
    label: &'static str,
    desc: &'static str,
    hint: Option<&'static str>,
    action: Action,
) -> KeyBinding {
    KeyBinding {
        keys,
        label,
        desc,
        hint,
        action,
    }
}

pub const GLOBAL_BINDINGS: &[KeyBinding] = &[
    kb(
        &[KeyCode::Char('?')],
        "?",
        "Toggle this help",
        Some("help"),
        Action::ToggleHelp,
    ),
    kb(
        &[
            KeyCode::Char('1'),
            KeyCode::Char('2'),
            KeyCode::Char('3'),
            KeyCode::Char('4'),
            KeyCode::Char('5'),
        ],
        "1-5",
        "Jump to tab N",
        None,
        Action::JumpTab,
    ),
    kb(&[KeyCode::Tab], "Tab", "Next tab", None, Action::NextTab),
    kb(
        &[KeyCode::BackTab],
        "Shift+Tab",
        "Previous tab",
        None,
        Action::PrevTab,
    ),
    kb(
        &[KeyCode::Char('q')],
        "q",
        "Quit",
        Some("quit"),
        Action::Quit,
    ),
    kb(&[], "Ctrl+C", "Quit (works everywhere)", None, Action::Info),
    kb(
        &[KeyCode::Char('r')],
        "r",
        "Reconnect to gpsd",
        Some("reconnect"),
        Action::Reconnect,
    ),
    kb(
        &[KeyCode::Char('s')],
        "s",
        "Settings (host/port/units)",
        Some("settings"),
        Action::OpenSettings,
    ),
    kb(
        &[KeyCode::Char('u')],
        "u",
        "Cycle units (not saved)",
        Some("units"),
        Action::CycleUnits,
    ),
    kb(
        &[KeyCode::Char('m')],
        "m",
        "Open position in Maps",
        Some("maps"),
        Action::OpenMaps,
    ),
    kb(
        &[KeyCode::Char('l')],
        "l",
        "Start/stop GPX logging",
        Some("log"),
        Action::ToggleLog,
    ),
    kb(
        &[KeyCode::Char('h')],
        "h",
        "Start/stop position hold",
        Some("hold"),
        Action::ToggleHold,
    ),
    kb(
        &[KeyCode::Esc],
        "Esc",
        "Close overlay / dismiss",
        None,
        Action::CloseOverlay,
    ),
];

const SCROLL_LINE: KeyBinding = kb(
    &[KeyCode::Up, KeyCode::Down],
    "\u{2191}/\u{2193}",
    "Scroll one line",
    Some("scroll"),
    Action::ScrollLine,
);
const SCROLL_PAGE: KeyBinding = kb(
    &[KeyCode::PageUp, KeyCode::PageDown],
    "PgUp/PgDn",
    "Scroll one page",
    None,
    Action::ScrollPage,
);

const SATELLITE_BINDINGS: &[KeyBinding] = &[
    SCROLL_LINE,
    SCROLL_PAGE,
    kb(
        &[KeyCode::Home, KeyCode::End],
        "Home/End",
        "First / last satellite",
        None,
        Action::ScrollEnds,
    ),
];

const NMEA_BINDINGS: &[KeyBinding] = &[
    kb(
        &[KeyCode::Char('p')],
        "p",
        "Pause / resume the stream",
        Some("pause"),
        Action::NmeaPause,
    ),
    kb(
        &[KeyCode::Char('f')],
        "f",
        "Cycle sentence filter",
        Some("filter"),
        Action::NmeaFilter,
    ),
    kb(
        &[KeyCode::Char('c')],
        "c",
        "Clear buffer",
        Some("clear"),
        Action::NmeaClear,
    ),
    kb(
        &[KeyCode::Up, KeyCode::Down],
        "\u{2191}/\u{2193}",
        "Scroll back / forward",
        None,
        Action::ScrollLine,
    ),
    SCROLL_PAGE,
    kb(
        &[KeyCode::Home, KeyCode::End],
        "Home/End",
        "Oldest / follow newest",
        None,
        Action::ScrollEnds,
    ),
];

const TIMING_BINDINGS: &[KeyBinding] = &[
    kb(
        &[KeyCode::Char('a')],
        "a",
        "Arm one-shot TOFF capture",
        Some("arm"),
        Action::ToffArm,
    ),
    kb(
        &[KeyCode::Char('c')],
        "c",
        "Clear TOFF data",
        Some("clear"),
        Action::ToffClear,
    ),
    kb(
        &[KeyCode::Char('k')],
        "k",
        "Arm clock sync (next fix)",
        Some("clock"),
        Action::ClockSyncToggle,
    ),
];

const DEVICE_BINDINGS: &[KeyBinding] = &[
    kb(
        &[KeyCode::Up, KeyCode::Down],
        "\u{2191}/\u{2193}",
        "Select control",
        Some("select"),
        Action::DeviceSelect,
    ),
    kb(
        &[KeyCode::Left, KeyCode::Right],
        "\u{2190}/\u{2192}",
        "Change value (* = pending)",
        Some("adjust"),
        Action::DeviceAdjust,
    ),
    kb(
        &[KeyCode::Enter],
        "Enter",
        "Apply (asks y/n first)",
        Some("apply"),
        Action::DeviceApply,
    ),
    kb(
        &[],
        "y/other",
        "Confirm / cancel prompt",
        None,
        Action::Info,
    ),
];

/// Keys shown in the settings dialog (dispatch is in the settings handler,
/// which depends on whether a text field is being edited).
pub const SETTINGS_BINDINGS: &[KeyBinding] = &[
    kb(&[], "\u{2191}/\u{2193}", "move", None, Action::Info),
    kb(&[], "Enter", "edit / cycle / save", None, Action::Info),
    kb(&[], "\u{2190}/\u{2192}", "cycle value", None, Action::Info),
    kb(&[], "Ctrl+S", "apply & save", None, Action::Info),
    kb(
        &[],
        "Esc",
        "cancel edit; again: discard & close",
        None,
        Action::Info,
    ),
];

/// Tab-specific bindings for `tab`.
pub fn tab_bindings(tab: ActiveTab) -> &'static [KeyBinding] {
    match tab {
        ActiveTab::Dashboard => &[],
        ActiveTab::Satellites => SATELLITE_BINDINGS,
        ActiveTab::Timing => TIMING_BINDINGS,
        ActiveTab::Device => DEVICE_BINDINGS,
        ActiveTab::Nmea => NMEA_BINDINGS,
    }
}

/// Find the binding for `code` on `tab`. Global bindings take precedence;
/// a unit test guarantees no tab binding collides with a global one.
pub fn lookup_binding(tab: ActiveTab, code: KeyCode) -> Option<&'static KeyBinding> {
    GLOBAL_BINDINGS
        .iter()
        .chain(tab_bindings(tab))
        .find(|b| b.keys.contains(&code))
}

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

/// Maximum NMEA lines kept in the buffer (and in the pause buffer).
pub const NMEA_BUFFER_MAX: usize = 1000;

/// A side-effecting command queued by the Device tab and executed by the event loop.
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceCommand {
    Ubxtool(Vec<String>),
    BaudRate { speed: u32, device: String },
}

/// Control values of the Device tab. Indices refer to the option tables
/// below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceValues {
    pub nav_rate_idx: usize,
    pub power_mode_idx: usize,
    pub serial_speed_idx: usize,
    pub pps_frequency_idx: usize,
    pub gnss_enabled: [bool; 6],
}

impl Default for DeviceValues {
    /// tuigps defaults. The receiver's actual configuration is not read
    /// back, so these are assumptions until a value is applied.
    fn default() -> Self {
        Self {
            nav_rate_idx: 0,
            power_mode_idx: 0,
            serial_speed_idx: 1, // 9600
            pps_frequency_idx: 0,
            gnss_enabled: [true; 6],
        }
    }
}

/// A Device-tab change awaiting `y` confirmation.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceConfirm {
    pub control: usize,
    /// One-line question, e.g. "Apply nav rate 5 Hz?".
    pub prompt: String,
    /// Extra warning for destructive changes (serial speed, disabling a
    /// constellation, save to flash). `Some` means destructive.
    pub warning: Option<String>,
    /// Line appended to the output log when confirmed.
    pub log: String,
    /// Commands queued when confirmed.
    pub commands: Vec<DeviceCommand>,
}

impl DeviceConfirm {
    pub fn destructive(&self) -> bool {
        self.warning.is_some()
    }
}

// Device configuration state
pub struct DeviceConfigState {
    pub nav_rate_idx: usize,
    pub power_mode_idx: usize,
    pub serial_speed_idx: usize,
    pub pps_frequency_idx: usize,
    pub gnss_enabled: [bool; 6], // GPS, GLONASS, Galileo, BeiDou, SBAS, QZSS
    pub output_log: Vec<String>,
    pub selected_control: usize,
    pub proto_version: String,
    /// Values last applied (sent) per control; initially the assumed
    /// defaults. A control whose value differs is shown as pending.
    pub applied: DeviceValues,
    /// Controls applied at least once this session.
    pub sent: [bool; DEVICE_CONTROL_COUNT],
    /// Change waiting for confirmation.
    pub confirm: Option<DeviceConfirm>,
}

pub const NAV_RATES: &[(&str, u32)] =
    &[("1 Hz", 1000), ("2 Hz", 500), ("5 Hz", 200), ("10 Hz", 100)];

pub const POWER_MODES: &[(&str, u8)] = &[
    ("Full Power", 0),
    ("Balanced", 1),
    ("1Hz Interval", 2),
    ("2Hz Interval", 3),
    ("4Hz Interval", 4),
];

pub const SERIAL_SPEEDS: &[u32] = &[4800, 9600, 19200, 38400, 57600, 115200, 230400];

pub const PPS_FREQUENCIES: &[(&str, u32)] = &[("1 Hz", 1), ("2 Hz", 2), ("5 Hz", 5), ("10 Hz", 10)];

pub const GNSS_NAMES_CONFIG: &[&str] = &["GPS", "GLONASS", "Galileo", "BeiDou", "SBAS", "QZSS"];

// Names used by ubxtool -e/-d flags
const GNSS_UBXTOOL_NAMES: &[&str] = &["GPS", "GLONASS", "GALILEO", "BEIDOU", "SBAS", "QZSS"];

// Total number of device config controls
pub const DEVICE_CONTROL_COUNT: usize = 11; // nav_rate, power, serial, pps, 6 GNSS toggles, save

impl Default for DeviceConfigState {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceConfigState {
    pub fn new() -> Self {
        let v = DeviceValues::default();
        Self {
            nav_rate_idx: v.nav_rate_idx,
            power_mode_idx: v.power_mode_idx,
            serial_speed_idx: v.serial_speed_idx,
            pps_frequency_idx: v.pps_frequency_idx,
            gnss_enabled: v.gnss_enabled,
            output_log: Vec::new(),
            selected_control: 0,
            proto_version: "18".to_string(),
            applied: v,
            sent: [false; DEVICE_CONTROL_COUNT],
            confirm: None,
        }
    }

    /// The values currently shown in the controls.
    pub fn current(&self) -> DeviceValues {
        DeviceValues {
            nav_rate_idx: self.nav_rate_idx,
            power_mode_idx: self.power_mode_idx,
            serial_speed_idx: self.serial_speed_idx,
            pps_frequency_idx: self.pps_frequency_idx,
            gnss_enabled: self.gnss_enabled,
        }
    }

    /// Whether `control` has been changed but not applied.
    pub fn is_pending(&self, control: usize) -> bool {
        let (c, a) = (self.current(), self.applied);
        match control {
            0 => c.nav_rate_idx != a.nav_rate_idx,
            1 => c.power_mode_idx != a.power_mode_idx,
            2 => c.serial_speed_idx != a.serial_speed_idx,
            3 => c.pps_frequency_idx != a.pps_frequency_idx,
            4..=9 => c.gnss_enabled[control - 4] != a.gnss_enabled[control - 4],
            _ => false,
        }
    }

    /// Record `control`'s current value as applied.
    fn mark_applied(&mut self, control: usize) {
        let c = self.current();
        match control {
            0 => self.applied.nav_rate_idx = c.nav_rate_idx,
            1 => self.applied.power_mode_idx = c.power_mode_idx,
            2 => self.applied.serial_speed_idx = c.serial_speed_idx,
            3 => self.applied.pps_frequency_idx = c.pps_frequency_idx,
            4..=9 => self.applied.gnss_enabled[control - 4] = c.gnss_enabled[control - 4],
            _ => {}
        }
        if let Some(s) = self.sent.get_mut(control) {
            *s = true;
        }
    }

    /// Append a line to the output log, dropping the oldest beyond the cap.
    pub fn push_log(&mut self, line: impl Into<String>) {
        self.output_log.push(line.into());
        if self.output_log.len() > OUTPUT_LOG_MAX {
            let excess = self.output_log.len() - OUTPUT_LOG_MAX;
            self.output_log.drain(..excess);
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
    pub armed_clock_set: bool,
    pub armed_toff: bool,
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
            armed_clock_set: false,
            armed_toff: false,
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
            Action::ToffArm => {
                self.armed_toff = true;
                self.set_status("TOFF armed — waiting for next fix...");
            }
            Action::ToffClear => self.clear_toff(),
            Action::ClockSyncToggle => {
                if self.armed_clock_set {
                    self.armed_clock_set = false;
                    self.set_status("Clock sync disarmed");
                } else {
                    self.armed_clock_set = true;
                    self.set_status("Clock sync ARMED — will fire on next 2D/3D fix");
                }
            }
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
                // Fresh connection: drop data from any previous session but
                // keep app-managed TOFF state.
                let old = std::mem::take(&mut self.gps_data);
                self.gps_data = GPSData {
                    connected: true,
                    toff_samples: old.toff_samples,
                    toff_armed_offset: old.toff_armed_offset,
                    toff_armed_gps_time: old.toff_armed_gps_time,
                    toff_armed_sys_time: old.toff_armed_sys_time,
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

        // TOFF offset against the TPV receipt time
        if new_time
            && self.gps_data.last_tpv > 0.0
            && let Ok(gps_time) = chrono::DateTime::parse_from_rfc3339(time)
        {
            let gps_epoch =
                gps_time.timestamp() as f64 + gps_time.timestamp_subsec_nanos() as f64 / 1e9;
            let offset = gps_epoch - self.gps_data.last_tpv;

            // Accumulate in circular buffer (max 20)
            if self.gps_data.toff_samples.len() >= 20 {
                self.gps_data.toff_samples.remove(0);
            }
            self.gps_data.toff_samples.push(offset);

            // Armed single-shot TOFF capture
            if self.armed_toff {
                self.armed_toff = false;
                self.gps_data.toff_armed_offset = offset;
                self.gps_data.toff_armed_gps_time = self.gps_data.time.clone();
                self.gps_data.toff_armed_sys_time = self.gps_data.last_tpv;
                self.set_status(format!(
                    "TOFF captured: {}",
                    crate::formatting::fmt_offset(offset)
                ));
            }
        }

        // Armed clock sync: fresh TPV with a 2D/3D fix only. Execution is
        // deferred to the event loop so it never blocks here.
        if self.armed_clock_set && new_time && self.gps_data.has_fix() {
            self.armed_clock_set = false;
            self.pending_clock_sync = Some(ClockSyncRequest {
                gps_time: self.gps_data.time.clone(),
                tpv_receipt: self.gps_data.last_tpv,
            });
            self.set_status("Clock sync: setting system clock...");
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

    // ------------------------------------------------------------------
    // Settings dialog
    //
    // Opening snapshots the current values. Units/coords preview live.
    // Esc while editing a text field cancels that field; Esc otherwise
    // discards every change and closes. Ctrl+S (or Enter on [Save])
    // applies: saves the config file and reconnects if host/port changed.
    // ------------------------------------------------------------------

    pub fn open_settings(&mut self) {
        self.show_settings = true;
        self.settings_editing = false;
        self.settings_field = SettingsField::Host;
        self.settings_message.clear();
        self.settings_snapshot = Some(self.current_config());
    }

    /// Discard all changes made in the dialog and close it.
    pub fn cancel_settings(&mut self) {
        if let Some(old) = self.settings_snapshot.take() {
            let changed = old != self.current_config();
            self.host = old.host;
            self.port = old.port;
            self.units = old.units;
            self.coord_format = old.coord_format;
            if changed {
                self.set_status("Settings changes discarded");
            }
        }
        self.close_settings();
    }

    /// Apply the dialog's values: persist them and reconnect if the gpsd
    /// address changed.
    pub fn apply_settings(&mut self) {
        let old = self
            .settings_snapshot
            .take()
            .unwrap_or_else(|| self.current_config());
        self.close_settings();

        let new = self.current_config();
        let mut msg = Vec::new();
        if old.host != new.host || old.port != new.port {
            self.reconnect_requested = true;
            msg.push(format!("Reconnecting to {}:{}", new.host, new.port));
        } else {
            msg.push("Settings applied".to_string());
        }
        if let Some(path) = &self.config_path {
            match config::save(path, &new) {
                Ok(()) => msg.push(format!("saved to {}", path.display())),
                Err(e) => msg.push(format!("NOT saved ({}: {})", path.display(), e)),
            }
        }
        self.set_status(msg.join("; "));
    }

    fn close_settings(&mut self) {
        self.show_settings = false;
        self.settings_editing = false;
        self.settings_message.clear();
    }

    /// Validate and store the text field being edited. On error the
    /// message is shown in the dialog and editing continues.
    fn commit_settings_edit(&mut self) -> bool {
        let result = match self.settings_field {
            SettingsField::Host => {
                let host = self.settings_edit_buf.trim();
                if host.is_empty() {
                    Err("Host must not be empty".to_string())
                } else {
                    self.host = host.to_string();
                    Ok(())
                }
            }
            SettingsField::Port => validate_port(&self.settings_edit_buf).map(|p| self.port = p),
            _ => Ok(()),
        };
        match result {
            Ok(()) => {
                self.settings_editing = false;
                self.settings_message.clear();
                true
            }
            Err(e) => {
                self.settings_message = e;
                false
            }
        }
    }

    fn handle_settings_key(&mut self, key: crossterm::event::KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('s') {
            if !self.settings_editing || self.commit_settings_edit() {
                self.apply_settings();
            }
            return;
        }

        if self.settings_editing {
            self.handle_settings_text_key(key);
            return;
        }

        match key.code {
            KeyCode::Esc => self.cancel_settings(),
            KeyCode::Up | KeyCode::BackTab => self.settings_field = self.settings_field.prev(),
            KeyCode::Down | KeyCode::Tab => self.settings_field = self.settings_field.next(),
            KeyCode::Enter => match self.settings_field {
                SettingsField::Host => {
                    self.settings_editing = true;
                    self.settings_edit_buf = self.host.clone();
                }
                SettingsField::Port => {
                    self.settings_editing = true;
                    self.settings_edit_buf = self.port.to_string();
                }
                SettingsField::Units => self.units = self.units.next(),
                SettingsField::CoordFormat => self.coord_format = self.coord_format.next(),
                SettingsField::Save => self.apply_settings(),
            },
            KeyCode::Left => match self.settings_field {
                SettingsField::Units => self.units = self.units.prev(),
                SettingsField::CoordFormat => self.coord_format = self.coord_format.prev(),
                _ => {}
            },
            KeyCode::Right => match self.settings_field {
                SettingsField::Units => self.units = self.units.next(),
                SettingsField::CoordFormat => self.coord_format = self.coord_format.next(),
                _ => {}
            },
            _ => {}
        }
    }

    fn handle_settings_text_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.settings_editing = false;
                self.settings_message.clear();
            }
            KeyCode::Enter => {
                self.commit_settings_edit();
            }
            KeyCode::Backspace => {
                self.settings_edit_buf.pop();
                self.settings_message.clear();
            }
            KeyCode::Char(_)
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {}
            KeyCode::Char(c) => match self.settings_field {
                SettingsField::Port => {
                    if !c.is_ascii_digit() {
                        self.settings_message = format!("Port accepts digits only ('{}')", c);
                    } else if self.settings_edit_buf.len() >= 5 {
                        self.settings_message = "Port is at most 5 digits (1-65535)".to_string();
                    } else {
                        self.settings_edit_buf.push(c);
                        self.settings_message = match validate_port(&self.settings_edit_buf) {
                            Ok(_) => String::new(),
                            Err(e) => e,
                        };
                    }
                }
                _ => {
                    if c.is_whitespace() || c.is_control() {
                        self.settings_message = "Host cannot contain spaces".to_string();
                    } else if self.settings_edit_buf.len() >= MAX_HOST_LEN {
                        self.settings_message = "Host name too long".to_string();
                    } else {
                        self.settings_edit_buf.push(c);
                        self.settings_message.clear();
                    }
                }
            },
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // Scrolling
    // ------------------------------------------------------------------

    fn page_size(&self) -> usize {
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

    /// Largest useful satellite-table offset (last row at the bottom).
    pub fn sat_max_scroll(&self) -> usize {
        self.gps_data
            .satellites
            .len()
            .saturating_sub(self.sat_view_rows.get())
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

    /// Append a sentence to the live NMEA buffer. While scrolled back, the
    /// bottom-relative offset grows with each visible line so the view
    /// stays on the same sentences.
    pub fn push_nmea(&mut self, sentence: String) {
        if self.nmea_buffer.len() >= NMEA_BUFFER_MAX {
            self.nmea_buffer.pop_front();
        }
        let visible = self.nmea_matches(&sentence);
        self.nmea_buffer.push_back(sentence);
        if self.nmea_scroll_offset > 0 {
            if visible {
                self.nmea_scroll_offset += 1;
            }
            // Lines evicted from the front can push the anchor off the top.
            self.nmea_scroll_offset = self.nmea_scroll_offset.min(self.nmea_max_scroll());
        }
    }

    fn toggle_nmea_pause(&mut self) {
        if self.nmea_paused {
            // Unpause: merge pause buffer into main buffer
            while let Some(s) = self.nmea_pause_buffer.pop_front() {
                if self.nmea_buffer.len() >= NMEA_BUFFER_MAX {
                    self.nmea_buffer.pop_front();
                }
                self.nmea_buffer.push_back(s);
            }
        }
        self.nmea_paused = !self.nmea_paused;
        self.nmea_scroll_offset = 0;
    }

    fn cycle_nmea_filter(&mut self) {
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

    fn clear_toff(&mut self) {
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
                dc.power_mode_idx =
                    (dc.power_mode_idx as i32 + dir).rem_euclid(POWER_MODES.len() as i32) as usize;
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

    /// Enter on a Device control: build the change and ask for
    /// confirmation. Nothing is queued until `y`.
    fn device_config_activate(&mut self) {
        let confirm = self.build_device_confirm();
        let prefix = if confirm.destructive() {
            "WARNING: "
        } else {
            ""
        };
        self.set_status(format!("{}{} y/n", prefix, confirm.prompt));
        self.device_config.confirm = Some(confirm);
    }

    fn build_device_confirm(&self) -> DeviceConfirm {
        let dc = &self.device_config;
        let control = dc.selected_control;
        let ubx =
            |args: &[&str]| DeviceCommand::Ubxtool(args.iter().map(|s| s.to_string()).collect());
        let (prompt, warning, log, commands) = match control {
            0 => {
                let (name, ms) = NAV_RATES[dc.nav_rate_idx];
                (
                    format!("Apply nav rate {}?", name),
                    None,
                    format!("Setting nav rate: {} ({}ms)...", name, ms),
                    vec![ubx(&["-p", &format!("CFG-RATE,{}", ms)])],
                )
            }
            1 => {
                let (name, mode) = POWER_MODES[dc.power_mode_idx];
                (
                    format!("Apply power mode {}?", name),
                    None,
                    format!("Setting power mode: {}...", name),
                    vec![ubx(&["-p", &format!("PMS,{}", mode)])],
                )
            }
            2 => {
                // Serial speed (two-step: ubxtool then gpsctl)
                let speed = SERIAL_SPEEDS[dc.serial_speed_idx];
                let device = self.gps_data.device.path.clone();
                (
                    format!("Change serial speed to {} baud?", speed),
                    Some(
                        "The link to the receiver can be lost if gpsd or the port does not \
                         follow the new speed."
                            .to_string(),
                    ),
                    format!("Setting baud rate to {}...", speed),
                    vec![DeviceCommand::BaudRate { speed, device }],
                )
            }
            3 => {
                let (name, freq) = PPS_FREQUENCIES[dc.pps_frequency_idx];
                let cmd = build_tp5_cmd(freq, 100000, true);
                (
                    format!("Apply PPS frequency {}?", name),
                    None,
                    format!("Setting PPS: {}...", name),
                    vec![ubx(&["-c", &cmd])],
                )
            }
            4..=9 => {
                let idx = control - 4;
                let name = GNSS_NAMES_CONFIG[idx];
                let ubx_name = GNSS_UBXTOOL_NAMES[idx];
                if dc.gnss_enabled[idx] {
                    (
                        format!("Enable {}?", name),
                        None,
                        format!("Enabling {}...", name),
                        vec![ubx(&["-e", ubx_name])],
                    )
                } else {
                    (
                        format!("DISABLE {}?", name),
                        Some(format!(
                            "The receiver stops tracking {} satellites; the fix may degrade \
                             or be lost.",
                            name
                        )),
                        format!("Disabling {}...", name),
                        vec![ubx(&["-d", ubx_name])],
                    )
                }
            }
            _ => (
                "Save receiver config to flash?".to_string(),
                Some(
                    "Overwrites the stored configuration; it persists across power cycles."
                        .to_string(),
                ),
                "Saving config to flash...".to_string(),
                vec![ubx(&["-p", "SAVE"])],
            ),
        };
        DeviceConfirm {
            control,
            prompt,
            warning,
            log,
            commands,
        }
    }

    /// Answer to a pending Device confirmation: `y` applies, any other key
    /// cancels.
    fn handle_device_confirm(&mut self, key: crossterm::event::KeyEvent) {
        let Some(confirm) = self.device_config.confirm.take() else {
            return;
        };
        let yes = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'))
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if yes {
            self.device_config.push_log(confirm.log);
            self.device_config.mark_applied(confirm.control);
            self.pending_device_cmds.extend(confirm.commands);
            self.set_status("Device command sent; see Output");
        } else {
            self.set_status(format!("Cancelled: {}", confirm.prompt));
        }
    }

    /// Execute queued device commands. Kept separate from key handling so
    /// tests can inspect the queue without spawning real processes.
    fn dispatch_device_commands(&mut self) {
        let proto = self.device_config.proto_version.clone();
        for cmd in std::mem::take(&mut self.pending_device_cmds) {
            match cmd {
                DeviceCommand::Ubxtool(args) => self.run_ubxtool(&args, &proto),
                DeviceCommand::BaudRate { speed, device } => {
                    self.run_baud_rate_change(speed, &device, &proto)
                }
            }
        }
    }

    /// Execute a queued clock sync on a blocking thread; the result comes
    /// back through `status_rx`. Never called from tests.
    fn dispatch_clock_sync(&mut self) {
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

    fn run_ubxtool(&self, args: &[String], proto_ver: &str) {
        let tx = self.cmd_tx.clone();
        let full_args: Vec<String> = ["-P".to_string(), proto_ver.to_string()]
            .into_iter()
            .chain(args.iter().cloned())
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
                    let mut result = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
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
                    let combined = if err.is_empty() {
                        out
                    } else {
                        format!("{}\n{}", out, err)
                    };
                    if !combined.trim().is_empty() {
                        let _ = tx.send(combined).await;
                    }
                }
                Ok(Err(e)) => {
                    let _ = tx.send(format!("ubxtool error: {}", e)).await;
                    return;
                }
                Err(_) => {
                    let _ = tx.send("ubxtool timed out (10s)".to_string()).await;
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
                        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        let _ = tx.send(format!("gpsctl error: {}", err)).await;
                    }
                }
                Ok(Err(e)) => {
                    let _ = tx.send(format!("gpsctl error: {}", e)).await;
                }
                Err(_) => {
                    let _ = tx.send("gpsctl timed out (10s)".to_string()).await;
                }
            }
        });
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

/// Redraw scheduling: urgent redraws (keys, tick) happen immediately; data
/// redraws are coalesced to at most one per [`FRAME_INTERVAL`].
#[derive(Debug)]
struct FrameLimiter {
    dirty: bool,
    urgent: bool,
    last_draw: Option<Instant>,
}

impl FrameLimiter {
    fn new() -> Self {
        Self {
            dirty: true,
            urgent: true,
            last_draw: None,
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn mark_urgent(&mut self) {
        self.dirty = true;
        self.urgent = true;
    }

    fn should_draw(&self, now: Instant) -> bool {
        self.dirty
            && (self.urgent
                || self
                    .last_draw
                    .is_none_or(|t| now.duration_since(t) >= FRAME_INTERVAL))
    }

    fn drew(&mut self, now: Instant) {
        self.dirty = false;
        self.urgent = false;
        self.last_draw = Some(now);
    }

    /// When the next coalesced redraw is due, if one is pending.
    fn deadline(&self) -> Option<Instant> {
        if !self.dirty {
            return None;
        }
        Some(
            self.last_draw
                .map_or_else(Instant::now, |t| t + FRAME_INTERVAL),
        )
    }
}

/// Run the TUI until quit. `startup` comes from [`config::startup`]
/// (config file merged with CLI overrides).
pub async fn run(terminal: &mut Terminal<impl Backend>, startup: Startup) -> Result<()> {
    let mut app = App::from_startup(startup);
    let result = event_loop(terminal, &mut app).await;
    // Always close the log so the GPX file is valid, even on error.
    app.stop_logging();
    result
}

async fn event_loop(terminal: &mut Terminal<impl Backend>, app: &mut App) -> Result<()> {
    // Channel for gpsd events, tagged with the connection generation
    let (tx, mut rx) = mpsc::channel::<TaggedEvent>(100);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let spawn_gpsd = |app: &App| {
        tokio::spawn(gpsd_client::gpsd_task(
            app.host.clone(),
            app.port,
            app.connection_generation,
            tx.clone(),
            shutdown_rx.clone(),
        ))
    };
    let mut gpsd_handle = spawn_gpsd(app);

    // Event stream
    let mut event_reader = crossterm::event::EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut frames = FrameLimiter::new();
    let mut input_errors = 0u32;

    let result = loop {
        let now = Instant::now();
        if frames.should_draw(now) {
            if let Err(e) = terminal.draw(|f| ui::render(f, app)) {
                break Err(e.into());
            }
            frames.drew(now);
        }

        let deadline = frames.deadline();
        tokio::select! {
            biased;
            event = event_reader.next() => match event {
                Some(Ok(event)) => {
                    input_errors = 0;
                    app.handle_event(event);
                    frames.mark_urgent();
                }
                Some(Err(e)) => {
                    input_errors += 1;
                    if input_errors >= MAX_INPUT_ERRORS {
                        // Input is unusable; quit rather than spin unquittable.
                        break Err(anyhow::anyhow!("terminal input failed: {}", e));
                    }
                    app.set_status(format!("Terminal input error: {}", e));
                    frames.mark_urgent();
                }
                None => {
                    // Input stream ended: no way to receive 'q', so quit.
                    app.should_quit = true;
                }
            },
            _ = tick.tick() => {
                app.tick();
                frames.mark_urgent();
            }
            Some(cmd_output) = app.cmd_rx.recv() => {
                // Split multi-line output into separate log entries
                for line in cmd_output.lines() {
                    if !line.trim().is_empty() {
                        app.device_config.push_log(line);
                    }
                }
                frames.mark_dirty();
            }
            Some(status) = app.status_rx.recv() => {
                app.set_status(status);
                frames.mark_urgent();
            }
            Some(event) = rx.recv() => {
                let mut changed = app.handle_tagged_event(event);
                // Drain whatever else is queued so one frame covers it all.
                for _ in 0..MAX_DRAIN {
                    match rx.try_recv() {
                        Ok(event) => changed |= app.handle_tagged_event(event),
                        Err(_) => break,
                    }
                }
                if changed {
                    frames.mark_dirty();
                }
            }
            _ = sleep_until_opt(deadline), if deadline.is_some() => {}
        }

        app.dispatch_device_commands();
        app.dispatch_clock_sync();

        // Handle reconnect
        if app.reconnect_requested {
            app.reconnect_requested = false;
            // Abort old task; any events it already queued carry the old
            // generation and are discarded by handle_tagged_event.
            gpsd_handle.abort();
            let _ = gpsd_handle.await;
            app.begin_reconnect();
            gpsd_handle = spawn_gpsd(app);
            frames.mark_urgent();
        }

        if app.should_quit {
            break Ok(());
        }
    };

    let _ = shutdown_tx.send(true);
    gpsd_handle.abort();
    result
}

async fn sleep_until_opt(deadline: Option<Instant>) {
    if let Some(d) = deadline {
        tokio::time::sleep_until(d.into()).await;
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

    fn now() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    }

    fn json_event(line: &str, receipt: f64) -> GpsdEvent {
        GpsdEvent::Message(GpsdMessage::parse(line, receipt).expect("valid gpsd message"))
    }

    /// A 3D-fix TPV at the given time, received at `receipt`.
    fn tpv(time: &str, receipt: f64) -> GpsdEvent {
        json_event(
            &format!(
                r#"{{"class":"TPV","mode":3,"lat":51.5,"lon":-0.1,"altMSL":10.0,"time":"{time}"}}"#
            ),
            receipt,
        )
    }

    fn hold_count(app: &App) -> u64 {
        app.position_hold
            .as_ref()
            .and_then(|h| h.result())
            .map_or(0, |r| r.count)
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
        assert_eq!(app.settings_field, SettingsField::Save);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.settings_field, SettingsField::Host); // wraps

        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.settings_field, SettingsField::Save); // wraps back
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

        // Nothing changed: apply closes without reconnecting.
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(!app.show_settings);
        assert!(!app.reconnect_requested);
        assert!(app.status_message.contains("applied"));
    }

    /// Open settings, select the port field and replace its text.
    fn type_port(app: &mut App, text: &str) {
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event(KeyCode::Down)); // Port
        app.handle_event(key_event(KeyCode::Enter)); // edit
        for _ in 0..10 {
            app.handle_event(key_event(KeyCode::Backspace));
        }
        for c in text.chars() {
            app.handle_event(key_event(KeyCode::Char(c)));
        }
    }

    #[test]
    fn test_settings_ctrl_s_reconnects_when_address_changed() {
        let mut app = new_app();
        type_port(&mut app, "3000");
        app.handle_event(key_event(KeyCode::Enter)); // commit field
        assert_eq!(app.port, 3000);
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(!app.show_settings);
        assert!(app.reconnect_requested);
        assert!(
            app.status_message
                .contains("Reconnecting to localhost:3000"),
            "{}",
            app.status_message
        );
    }

    #[test]
    fn test_settings_ctrl_s_while_editing_commits_field() {
        let mut app = new_app();
        type_port(&mut app, "4000");
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(!app.show_settings);
        assert_eq!(app.port, 4000);
        assert!(app.reconnect_requested);
    }

    #[test]
    fn test_settings_ctrl_s_with_invalid_port_stays_open() {
        let mut app = new_app();
        type_port(&mut app, "0");
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(app.show_settings);
        assert!(app.settings_editing);
        assert_eq!(app.port, 2947);
        assert!(!app.reconnect_requested);
        assert!(app.settings_message.contains("1-65535"));
    }

    #[test]
    fn test_settings_port_digits_only() {
        let mut app = new_app();
        type_port(&mut app, "12a3");
        assert_eq!(app.settings_edit_buf, "123");
        // The rejection is visible (it was replaced by the later valid digit,
        // so check by typing a letter last).
        app.handle_event(key_event(KeyCode::Char('x')));
        assert!(
            app.settings_message.contains("digits only"),
            "{}",
            app.settings_message
        );
        assert_eq!(app.settings_edit_buf, "123");
    }

    #[test]
    fn test_settings_port_max_five_digits() {
        let mut app = new_app();
        type_port(&mut app, "123456");
        assert_eq!(app.settings_edit_buf, "12345");
        assert!(app.settings_message.contains("5 digits"));
    }

    #[test]
    fn test_settings_port_out_of_range_rejected_on_enter() {
        for bad in ["0", "65536", "99999", ""] {
            let mut app = new_app();
            type_port(&mut app, bad);
            app.handle_event(key_event(KeyCode::Enter));
            assert!(app.settings_editing, "{bad}: should stay editing");
            assert_eq!(app.port, 2947, "{bad}");
            assert!(!app.settings_message.is_empty(), "{bad}");
        }
        let mut app = new_app();
        type_port(&mut app, "65535");
        app.handle_event(key_event(KeyCode::Enter));
        assert!(!app.settings_editing);
        assert_eq!(app.port, 65535);
        assert!(app.settings_message.is_empty());
    }

    #[test]
    fn test_validate_port() {
        assert_eq!(validate_port("1"), Ok(1));
        assert_eq!(validate_port("65535"), Ok(65535));
        assert!(validate_port("0").is_err());
        assert!(validate_port("65536").is_err());
        assert!(validate_port("").is_err());
        assert!(validate_port("-1").is_err());
        assert!(validate_port("12ab").is_err());
        assert!(validate_port("99999999999999999999").is_err());
    }

    #[test]
    fn test_settings_empty_host_rejected() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event(KeyCode::Enter));
        for _ in 0..20 {
            app.handle_event(key_event(KeyCode::Backspace));
        }
        app.handle_event(key_event(KeyCode::Enter));
        assert!(app.settings_editing);
        assert_eq!(app.host, "localhost");
        assert!(app.settings_message.contains("empty"));
        // Spaces are rejected visibly.
        app.handle_event(key_event(KeyCode::Char(' ')));
        assert!(app.settings_edit_buf.is_empty());
        assert!(app.settings_message.contains("spaces"));
    }

    #[test]
    fn test_settings_left_right_cycle_both_ways() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event(KeyCode::Down));
        app.handle_event(key_event(KeyCode::Down)); // Units
        app.handle_event(key_event(KeyCode::Left));
        assert_eq!(app.units, UnitSystem::Nautical);
        app.handle_event(key_event(KeyCode::Left));
        assert_eq!(app.units, UnitSystem::Imperial);
        app.handle_event(key_event(KeyCode::Right));
        assert_eq!(app.units, UnitSystem::Nautical);

        app.handle_event(key_event(KeyCode::Down)); // Coords
        app.handle_event(key_event(KeyCode::Left));
        assert_eq!(app.coord_format, CoordFormat::DDM);
        app.handle_event(key_event(KeyCode::Right));
        assert_eq!(app.coord_format, CoordFormat::DD);
        app.handle_event(key_event(KeyCode::Right));
        assert_eq!(app.coord_format, CoordFormat::DMS);
    }

    #[test]
    fn test_settings_esc_discards_all_changes() {
        let mut app = new_app();
        type_port(&mut app, "5000");
        app.handle_event(key_event(KeyCode::Enter)); // commit port
        app.handle_event(key_event(KeyCode::Down)); // Units
        app.handle_event(key_event(KeyCode::Right));
        assert_eq!(app.units, UnitSystem::Imperial); // live preview
        app.handle_event(key_event(KeyCode::Esc));
        assert!(!app.show_settings);
        assert_eq!(app.port, 2947);
        assert_eq!(app.units, UnitSystem::Metric);
        assert!(!app.reconnect_requested);
        assert!(app.status_message.contains("discarded"));
    }

    #[test]
    fn test_settings_esc_while_editing_cancels_field_only() {
        let mut app = new_app();
        type_port(&mut app, "5000");
        app.handle_event(key_event(KeyCode::Esc));
        assert!(app.show_settings);
        assert!(!app.settings_editing);
        assert_eq!(app.port, 2947);
        app.handle_event(key_event(KeyCode::Esc));
        assert!(!app.show_settings);
    }

    #[test]
    fn test_settings_enter_on_save_applies() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event(KeyCode::Up)); // wraps to Save
        assert_eq!(app.settings_field, SettingsField::Save);
        app.handle_event(key_event(KeyCode::Enter));
        assert!(!app.show_settings);
        assert!(app.settings_snapshot.is_none());
    }

    #[test]
    fn test_settings_apply_saves_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        let mut app = new_app();
        app.config_path = Some(path.clone());
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event(KeyCode::Down));
        app.handle_event(key_event(KeyCode::Down)); // Units
        app.handle_event(key_event(KeyCode::Right));
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(app.status_message.contains("saved to"));
        let (cfg, outcome) = config::load(&path);
        assert_eq!(outcome, config::LoadOutcome::Loaded);
        assert_eq!(cfg.units, UnitSystem::Imperial);
        assert_eq!(cfg.host, "localhost");
    }

    #[test]
    fn test_settings_save_failure_reported() {
        let dir = tempfile::tempdir().unwrap();
        // Parent "directory" is a file, so saving must fail.
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        let mut app = new_app();
        app.config_path = Some(blocker.join("config.toml"));
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(
            app.status_message.contains("NOT saved"),
            "{}",
            app.status_message
        );
        assert!(!app.show_settings);
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
    fn test_settings_digits_do_not_switch_tabs() {
        let mut app = new_app();
        type_port(&mut app, "3");
        assert_eq!(app.active_tab, ActiveTab::Dashboard);
        app.handle_event(key_event(KeyCode::Esc));
        // Navigation mode in settings also swallows digits.
        app.handle_event(key_event(KeyCode::Char('4')));
        assert_eq!(app.active_tab, ActiveTab::Dashboard);
        assert!(app.show_settings);
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
        app.gps_data.toff_samples = vec![1.0];
        app.handle_event(key_event_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
        assert_eq!(app.gps_data.toff_samples.len(), 1);

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
        // Enter only asks for confirmation.
        assert!(app.pending_device_cmds.is_empty());
        assert!(app.device_config.output_log.is_empty());
        let confirm = app.device_config.confirm.as_ref().expect("confirmation");
        assert!(!confirm.destructive());
        assert!(app.status_message.contains("Apply nav rate 1 Hz? y/n"));

        app.handle_event(key_event(KeyCode::Char('y')));
        assert!(app.device_config.confirm.is_none());
        assert_eq!(app.device_config.output_log.len(), 1);
        assert!(app.device_config.output_log[0].contains("nav rate"));
        // Queued, not executed: tests must never run ubxtool against real hardware
        assert_eq!(
            app.pending_device_cmds,
            vec![DeviceCommand::Ubxtool(vec![
                "-p".to_string(),
                "CFG-RATE,1000".to_string()
            ])]
        );
    }

    #[test]
    fn test_device_activate_baud_rate() {
        let mut app = new_app();
        app.active_tab = ActiveTab::Device;
        app.device_config.selected_control = 2;
        app.gps_data.device.path = "/dev/ttyACM0".to_string();

        app.handle_event(key_event(KeyCode::Enter));
        assert!(app.device_config.confirm.as_ref().unwrap().destructive());
        assert!(app.status_message.starts_with("WARNING"));
        assert!(app.pending_device_cmds.is_empty());
        app.handle_event(key_event(KeyCode::Char('y')));
        assert_eq!(
            app.pending_device_cmds,
            vec![DeviceCommand::BaudRate {
                speed: SERIAL_SPEEDS[app.device_config.serial_speed_idx],
                device: "/dev/ttyACM0".to_string(),
            }]
        );
    }

    fn device_app(control: usize) -> App {
        let mut app = new_app();
        app.active_tab = ActiveTab::Device;
        app.device_config.selected_control = control;
        app
    }

    #[test]
    fn test_device_confirm_cancel_with_other_key() {
        for cancel in [
            KeyCode::Char('n'),
            KeyCode::Esc,
            KeyCode::Enter,
            KeyCode::Down,
            KeyCode::Char('q'),
            KeyCode::Tab,
        ] {
            let mut app = device_app(0);
            app.handle_event(key_event(KeyCode::Right)); // 2 Hz, pending
            app.handle_event(key_event(KeyCode::Enter));
            assert!(app.device_config.confirm.is_some());
            app.handle_event(key_event(cancel));
            assert!(app.device_config.confirm.is_none(), "{cancel:?}");
            assert!(app.pending_device_cmds.is_empty(), "{cancel:?}");
            assert!(app.status_message.contains("Cancelled"), "{cancel:?}");
            // The cancelling key is consumed, not acted on.
            assert!(!app.should_quit, "{cancel:?}");
            assert_eq!(app.active_tab, ActiveTab::Device, "{cancel:?}");
            assert_eq!(app.device_config.selected_control, 0, "{cancel:?}");
            // Still pending: value differs from the applied one.
            assert!(app.device_config.is_pending(0));
        }
    }

    #[test]
    fn test_device_pending_vs_applied() {
        let mut app = device_app(0);
        assert!(!app.device_config.is_pending(0));
        app.handle_event(key_event(KeyCode::Right));
        assert!(app.device_config.is_pending(0));
        assert!(!app.device_config.sent[0]);

        app.handle_event(key_event(KeyCode::Enter));
        app.handle_event(key_event(KeyCode::Char('y')));
        assert!(!app.device_config.is_pending(0));
        assert!(app.device_config.sent[0]);
        assert_eq!(app.device_config.applied.nav_rate_idx, 1);

        // Changing back away from the applied value is pending again.
        app.handle_event(key_event(KeyCode::Left));
        assert!(app.device_config.is_pending(0));
        // Other controls untouched.
        assert!(!app.device_config.is_pending(1));
    }

    #[test]
    fn test_device_gnss_disable_is_destructive_enable_is_not() {
        let mut app = device_app(5); // GLONASS
        app.handle_event(key_event(KeyCode::Right)); // uncheck
        assert!(app.device_config.is_pending(5));
        app.handle_event(key_event(KeyCode::Enter));
        let c = app.device_config.confirm.clone().unwrap();
        assert!(c.destructive());
        assert!(c.prompt.contains("DISABLE GLONASS"));
        app.handle_event(key_event(KeyCode::Char('y')));
        assert_eq!(
            app.pending_device_cmds,
            vec![DeviceCommand::Ubxtool(vec!["-d".into(), "GLONASS".into()])]
        );
        assert!(!app.device_config.is_pending(5));

        app.pending_device_cmds.clear();
        app.handle_event(key_event(KeyCode::Right)); // re-check
        app.handle_event(key_event(KeyCode::Enter));
        assert!(!app.device_config.confirm.as_ref().unwrap().destructive());
        app.handle_event(key_event(KeyCode::Char('Y')));
        assert_eq!(
            app.pending_device_cmds,
            vec![DeviceCommand::Ubxtool(vec!["-e".into(), "GLONASS".into()])]
        );
    }

    #[test]
    fn test_device_save_is_destructive() {
        let mut app = device_app(DEVICE_CONTROL_COUNT - 1);
        app.handle_event(key_event(KeyCode::Enter));
        let c = app.device_config.confirm.clone().unwrap();
        assert!(c.destructive());
        assert!(c.prompt.contains("flash"));
        assert!(app.pending_device_cmds.is_empty());
        app.handle_event(key_event(KeyCode::Char('y')));
        assert_eq!(
            app.pending_device_cmds,
            vec![DeviceCommand::Ubxtool(vec!["-p".into(), "SAVE".into()])]
        );
        assert!(app.device_config.sent[DEVICE_CONTROL_COUNT - 1]);
    }

    #[test]
    fn test_device_non_destructive_controls() {
        for control in [0, 1, 3] {
            let mut app = device_app(control);
            app.handle_event(key_event(KeyCode::Enter));
            let c = app.device_config.confirm.clone().unwrap();
            assert!(!c.destructive(), "{control}");
            assert_eq!(c.control, control);
            assert_eq!(c.commands.len(), 1);
        }
    }

    #[test]
    fn test_device_ctrl_y_does_not_confirm() {
        let mut app = device_app(0);
        app.handle_event(key_event(KeyCode::Enter));
        app.handle_event(key_event_mod(KeyCode::Char('y'), KeyModifiers::ALT));
        assert!(app.pending_device_cmds.is_empty());
        assert!(app.device_config.confirm.is_none());
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
    fn test_no_binding_collisions() {
        // Every key maps to at most one binding per tab, so global
        // precedence never hides a tab key.
        for tab in ActiveTab::ALL {
            let mut seen = Vec::new();
            for b in GLOBAL_BINDINGS.iter().chain(tab_bindings(*tab)) {
                for k in b.keys {
                    assert!(!seen.contains(k), "{tab:?}: {k:?} bound twice");
                    seen.push(*k);
                }
            }
        }
    }

    #[test]
    fn test_every_binding_documented() {
        for b in GLOBAL_BINDINGS
            .iter()
            .chain(ActiveTab::ALL.iter().flat_map(|t| tab_bindings(*t)))
            .chain(SETTINGS_BINDINGS)
        {
            assert!(!b.label.is_empty() && !b.desc.is_empty(), "{b:?}");
            assert!(
                b.keys.is_empty() == (b.action == Action::Info),
                "only Info rows may lack keys: {b:?}"
            );
        }
    }

    #[test]
    fn test_lookup_binding_scopes_tab_keys() {
        assert_eq!(
            lookup_binding(ActiveTab::Nmea, KeyCode::Char('c')).map(|b| b.action),
            Some(Action::NmeaClear)
        );
        assert_eq!(
            lookup_binding(ActiveTab::Timing, KeyCode::Char('c')).map(|b| b.action),
            Some(Action::ToffClear)
        );
        assert!(lookup_binding(ActiveTab::Dashboard, KeyCode::Char('c')).is_none());
        assert_eq!(
            lookup_binding(ActiveTab::Device, KeyCode::Char('q')).map(|b| b.action),
            Some(Action::Quit)
        );
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
        assert!(!app.armed_toff);
    }

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
        app.gps_data.toff_samples = vec![0.1];
        app.handle_gpsd_event(GpsdEvent::Error("closed".to_string()));
        assert!(!app.gps_data.connected);

        app.handle_gpsd_event(GpsdEvent::Connected);
        assert!(app.gps_data.connected);
        assert!(app.gps_data.error_message.is_empty());
        assert!(app.gps_data.latitude.is_nan());
        assert_eq!(app.gps_data.toff_samples, vec![0.1]);
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

    #[test]
    fn test_gpsd_error() {
        let mut app = new_app();
        app.gps_data.connected = true;

        app.handle_gpsd_event(GpsdEvent::Error("connection refused".to_string()));
        assert!(!app.gps_data.connected);
        assert_eq!(app.gps_data.error_message, "connection refused");
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

    #[test]
    fn test_output_log_capped() {
        let mut dc = DeviceConfigState::new();
        for i in 0..(OUTPUT_LOG_MAX + 50) {
            dc.push_log(format!("line {i}"));
        }
        assert_eq!(dc.output_log.len(), OUTPUT_LOG_MAX);
        assert_eq!(dc.output_log[0], "line 50");
        assert_eq!(
            dc.output_log.last().unwrap(),
            &format!("line {}", OUTPUT_LOG_MAX + 49)
        );
    }

    // === Frame limiter ===

    #[test]
    fn test_frame_limiter_coalesces_data_redraws() {
        let mut f = FrameLimiter::new();
        let t0 = Instant::now();
        assert!(f.should_draw(t0)); // first frame
        f.drew(t0);
        assert!(!f.should_draw(t0));
        assert!(f.deadline().is_none());

        f.mark_dirty();
        assert!(!f.should_draw(t0 + Duration::from_millis(10)));
        assert_eq!(f.deadline(), Some(t0 + FRAME_INTERVAL));
        assert!(f.should_draw(t0 + FRAME_INTERVAL));
    }

    #[test]
    fn test_frame_limiter_urgent_draws_immediately() {
        let mut f = FrameLimiter::new();
        let t0 = Instant::now();
        f.drew(t0);
        f.mark_urgent();
        assert!(f.should_draw(t0 + Duration::from_millis(1)));
    }
}
