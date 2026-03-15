# tuigps-rs Implementation Guide

Complete guide for reimplementing tuigps (Python/Textual) in Rust (Ratatui/Crossterm).

---

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Crate Dependencies](#crate-dependencies)
3. [Phase 1: Project Scaffolding](#phase-1-project-scaffolding)
4. [Phase 2: Data Model](#phase-2-data-model)
5. [Phase 3: gpsd Client](#phase-3-gpsd-client)
6. [Phase 4: Constants and Formatting](#phase-4-constants-and-formatting)
7. [Phase 5: Application Shell](#phase-5-application-shell)
8. [Phase 6: Dashboard Widgets](#phase-6-dashboard-widgets)
9. [Phase 7: Satellite Tab](#phase-7-satellite-tab)
10. [Phase 8: Timing Tab](#phase-8-timing-tab)
11. [Phase 9: Device Configuration Tab](#phase-9-device-configuration-tab)
12. [Phase 10: NMEA Viewer Tab](#phase-10-nmea-viewer-tab)
13. [Phase 11: Settings Overlay](#phase-11-settings-overlay)
14. [Phase 12: GPS Logger](#phase-12-gps-logger)
15. [Phase 13: Position Hold](#phase-13-position-hold)
16. [Phase 14: Clock Sync](#phase-14-clock-sync)
17. [Phase 15: Footer Status Bar](#phase-15-footer-status-bar)
18. [Implementation Notes](#implementation-notes)
19. [Testing Strategy](#testing-strategy)

---

## Architecture Overview

### Python (original)
```
gpsd ──TCP──▶ gpsd_client (daemon thread) ──call_from_thread──▶ Textual App ──▶ Widgets
```

### Rust (target)
```
gpsd ──TCP──▶ gpsd_task (tokio) ──mpsc channel──▶ Event Loop ──▶ Ratatui render
```

### Event Loop Design

The main loop processes three event sources:

```rust
loop {
    tokio::select! {
        // Terminal events (key press, resize)
        Some(event) = terminal_events.next() => { handle_input(event); }
        // GPS data updates from gpsd task
        Some(data) = gps_rx.recv() => { app.update_gps(data); }
        // 1Hz heartbeat for staleness detection
        _ = tick_interval.tick() => { app.tick(); }
    }
    // Render UI
    terminal.draw(|f| ui::render(f, &app))?;
}
```

### Module Dependency Graph

```
main.rs
  ├── app.rs (App state, event handling, tab management)
  │     ├── data_model.rs (GPSData, SatelliteInfo, etc.)
  │     ├── constants.rs (GNSS names, colors, ratings)
  │     ├── formatting.rs (NaN-safe display helpers)
  │     ├── gps_logger.rs (GPX/CSV file writer)
  │     ├── position_hold.rs (Welford accumulator)
  │     └── clock_sync.rs (system clock adjustment)
  ├── gpsd_client.rs (async TCP reader, JSON parser)
  └── ui/ (all rendering functions)
        ├── mod.rs (top-level layout: tabs + footer)
        ├── dashboard.rs (3x3 grid)
        ├── position.rs, fix.rs, velocity.rs, ...
        ├── settings.rs (modal overlay)
        └── status_bar.rs (footer)
```

---

## Crate Dependencies

### Cargo.toml

```toml
[package]
name = "tuigps-rs"
version = "0.1.0"
edition = "2024"

[dependencies]
# TUI
ratatui = "0.29"
crossterm = "0.28"

# Async runtime
tokio = { version = "1", features = ["full"] }

# Serialization (gpsd JSON)
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Date/time
chrono = { version = "0.4", features = ["serde"] }

# CLI arguments
clap = { version = "4", features = ["derive"] }

# Linux system calls (ioctl for PPS, clock_settime)
nix = { version = "0.29", features = ["ioctl", "time"] }

# D-Bus for clock sync
zbus = { version = "5", optional = true }

# Open URLs in browser
open = "5"

# Error handling
anyhow = "1"
thiserror = "2"

# Logging (optional, for debug)
tracing = "0.1"
tracing-subscriber = "0.3"

[features]
default = ["dbus"]
dbus = ["dep:zbus"]
```

---

## Phase 1: Project Scaffolding

### 1.1 Initialize Project

```bash
cargo init tuigps-rs
```

### 1.2 Create Directory Structure

```
src/
├── main.rs
├── app.rs
├── gpsd_client.rs
├── data_model.rs
├── constants.rs
├── formatting.rs
├── gps_logger.rs
├── position_hold.rs
├── clock_sync.rs
└── ui/
    ├── mod.rs
    ├── dashboard.rs
    ├── position.rs
    ├── fix.rs
    ├── velocity.rs
    ├── sky_plot.rs
    ├── signal_chart.rs
    ├── error_panel.rs
    ├── device_panel.rs
    ├── time_panel.rs
    ├── constellation.rs
    ├── satellite_table.rs
    ├── nmea_viewer.rs
    ├── status_bar.rs
    ├── device_config.rs
    └── settings.rs
```

### 1.3 main.rs Skeleton

```rust
use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;
use tokio::sync::mpsc;

mod app;
mod constants;
mod data_model;
mod formatting;
mod gpsd_client;
mod gps_logger;
mod position_hold;
mod clock_sync;
mod ui;

#[derive(Parser)]
#[command(name = "tuigps-rs", about = "Terminal GPS monitor")]
struct Cli {
    #[arg(long, default_value = "localhost")]
    host: String,
    #[arg(long, default_value_t = 2947)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run app
    let result = app::run(&mut terminal, &cli.host, cli.port).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}
```

---

## Phase 2: Data Model

Port `data_model.py` to `src/data_model.rs`.

### 2.1 Enums

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FixMode {
    Unknown = 0,
    NoFix = 1,
    Fix2D = 2,
    Fix3D = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FixStatus {
    Unknown = 0,
    Gps = 1,
    Dgps = 2,
    RtkFix = 3,
    RtkFloat = 4,
    Dr = 5,
    GnssDr = 6,
    TimeOnly = 7,
    Simulated = 8,
    PpsFix = 9,
}
```

Implement `From<u8>` for both enums with a default/fallback for unknown values.

### 2.2 Data Structs

```rust
#[derive(Debug, Clone)]
pub struct SatelliteInfo {
    pub prn: u16,
    pub gnssid: u8,
    pub svid: u16,
    pub elevation: f64,    // NaN if unknown
    pub azimuth: f64,      // NaN if unknown
    pub snr: f64,          // NaN if unknown
    pub used: bool,
    pub sigid: u8,
    pub health: u8,        // 0 = unknown, 1 = healthy, 2 = unhealthy
    pub freqid: Option<i8>,
}

#[derive(Debug, Clone, Default)]
pub struct DOPValues {
    pub hdop: f64,
    pub vdop: f64,
    pub pdop: f64,
    pub gdop: f64,
    pub tdop: f64,
    pub xdop: f64,
    pub ydop: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ErrorEstimates {
    pub eph: f64,
    pub epv: f64,
    pub ept: f64,
    pub eps: f64,
    pub epd: f64,
    pub epc: f64,
    pub epx: f64,
    pub epy: f64,
    pub sep: f64,
}

#[derive(Debug, Clone, Default)]
pub struct PPSData {
    pub real_sec: i64,
    pub real_nsec: i64,
    pub clock_sec: i64,
    pub clock_nsec: i64,
    pub precision: i32,
    pub qerr: i64,
}

#[derive(Debug, Clone, Default)]
pub struct TOFFData {
    pub real_sec: i64,
    pub real_nsec: i64,
    pub clock_sec: i64,
    pub clock_nsec: i64,
}

#[derive(Debug, Clone, Default)]
pub struct DeviceInfo {
    pub path: String,
    pub driver: String,
    pub subtype: String,
    pub bps: u32,
    pub cycle: f64,
    pub mincycle: f64,
    pub activated: String,
    pub native: u8,
}

#[derive(Debug, Clone, Default)]
pub struct VersionInfo {
    pub release: String,
    pub proto_major: u32,
    pub proto_minor: u32,
}
```

### 2.3 Main GPSData Struct

```rust
#[derive(Debug, Clone)]
pub struct GPSData {
    // Connection state
    pub connected: bool,
    pub last_seen: f64,       // unix timestamp
    pub error_message: String,

    // TPV
    pub latitude: f64,
    pub longitude: f64,
    pub alt_hae: f64,
    pub alt_msl: f64,
    pub geoid_sep: f64,
    pub speed: f64,
    pub track: f64,
    pub climb: f64,
    pub time: String,          // ISO 8601
    pub leapseconds: i32,
    pub mode: FixMode,
    pub status: FixStatus,
    pub magtrack: f64,
    pub magvar: f64,

    // ECEF
    pub ecefx: f64,
    pub ecefy: f64,
    pub ecefz: f64,
    pub ecefvx: f64,
    pub ecefvy: f64,
    pub ecefvz: f64,

    // Composites
    pub dop: DOPValues,
    pub errors: ErrorEstimates,
    pub pps: PPSData,
    pub toff: TOFFData,
    pub device: DeviceInfo,
    pub version: VersionInfo,

    // TOFF accumulation
    pub toff_samples: Vec<f64>,      // circular buffer, max 20
    pub toff_armed_offset: f64,
    pub toff_armed_gps_time: String,
    pub toff_armed_sys_time: f64,

    // Satellites
    pub satellites: Vec<SatelliteInfo>,
    pub satellites_used: u32,
}
```

Implement `Default` for `GPSData` with all floats set to `f64::NAN`, strings empty, vectors empty, `mode = FixMode::Unknown`, `status = FixStatus::Unknown`.

### 2.4 Computed Methods

```rust
impl GPSData {
    /// Returns (visible, used) counts per gnssid
    pub fn constellation_counts(&self) -> HashMap<u8, (u32, u32)> { ... }

    pub fn has_fix(&self) -> bool {
        matches!(self.mode, FixMode::Fix2D | FixMode::Fix3D)
            && self.latitude.is_finite()
            && self.longitude.is_finite()
    }

    /// PPS offset in microseconds
    pub fn pps_offset_us(&self) -> f64 { ... }
}
```

---

## Phase 3: gpsd Client

Port `gpsd_client.py` to `src/gpsd_client.rs`.

### 3.1 Connection Architecture

```rust
use tokio::net::TcpStream;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

pub enum GpsdEvent {
    Update(GPSData),
    Error(String),
    Nmea(String),
}

pub async fn gpsd_task(
    host: String,
    port: u16,
    tx: mpsc::Sender<GpsdEvent>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        match connect_and_read(&host, port, &tx, &mut shutdown).await {
            Ok(()) => break,  // clean shutdown
            Err(e) => {
                let _ = tx.send(GpsdEvent::Error(e.to_string())).await;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {},
                    _ = shutdown.changed() => break,
                }
            }
        }
    }
}
```

### 3.2 Protocol Handling

gpsd speaks a JSON-based protocol over TCP. On connect, send the WATCH command:

```rust
async fn connect_and_read(...) -> Result<()> {
    let stream = TcpStream::connect((host, port)).await?;
    let (reader, mut writer) = stream.into_split();

    // Send WATCH command
    let watch = r#"?WATCH={"enable":true,"json":true,"pps":true,"timing":true,"nmea":true}"#;
    writer.write_all(watch.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut data = GPSData::default();
    data.connected = true;

    loop {
        line.clear();
        tokio::select! {
            result = reader.read_line(&mut line) => {
                let n = result?;
                if n == 0 { return Err(anyhow!("gpsd connection closed")); }
                let receipt_time = std::time::SystemTime::now()
                    .duration_since(UNIX_EPOCH)?.as_secs_f64();
                process_message(&line, &mut data, receipt_time)?;
                let _ = tx.send(GpsdEvent::Update(data.clone())).await;
            }
            _ = shutdown.changed() => return Ok(()),
        }
    }
}
```

### 3.3 Message Parsing

Parse each JSON line by its `"class"` field:

```rust
fn process_message(line: &str, data: &mut GPSData, receipt_time: f64) -> Result<()> {
    // Check for NMEA (starts with $ or !)
    if line.starts_with('$') || line.starts_with('!') {
        // Send as NMEA event
        return Ok(());
    }

    let msg: serde_json::Value = serde_json::from_str(line)?;
    let class = msg["class"].as_str().unwrap_or("");

    match class {
        "TPV" => extract_tpv(&msg, data, receipt_time),
        "SKY" => extract_sky(&msg, data),
        "PPS" => extract_pps(&msg, data),
        "TOFF" => extract_toff(&msg, data),
        "DEVICES" => extract_devices(&msg, data),
        "DEVICE" => extract_device(&msg, data),
        "VERSION" => extract_version(&msg, data),
        _ => {} // ignore unknown classes
    }

    data.last_seen = receipt_time;
    Ok(())
}
```

### 3.4 TPV Extraction (most complex)

```rust
fn extract_tpv(msg: &Value, data: &mut GPSData, receipt_time: f64) {
    data.mode = FixMode::from(msg["mode"].as_u64().unwrap_or(0) as u8);
    data.status = FixStatus::from(msg["status"].as_u64().unwrap_or(0) as u8);

    // Extract all float fields (NaN if absent)
    data.latitude = json_f64(msg, "lat");
    data.longitude = json_f64(msg, "lon");
    data.alt_hae = json_f64(msg, "altHAE");
    data.alt_msl = json_f64(msg, "altMSL");
    data.geoid_sep = json_f64(msg, "geoidSep");
    data.speed = json_f64(msg, "speed");
    data.track = json_f64(msg, "track");
    data.climb = json_f64(msg, "climb");
    data.magtrack = json_f64(msg, "magtrack");
    data.magvar = json_f64(msg, "magvar");
    // ... error estimates, ECEF, leapseconds, time string

    // TOFF computation from GPS time vs receipt_time
    if let Some(time_str) = msg["time"].as_str() {
        data.time = time_str.to_string();
        if let Ok(gps_time) = chrono::DateTime::parse_from_rfc3339(time_str) {
            let gps_epoch = gps_time.timestamp() as f64
                + gps_time.timestamp_subsec_nanos() as f64 / 1e9;
            let offset = gps_epoch - receipt_time;

            // Accumulate in circular buffer (max 20)
            if data.toff_samples.len() >= 20 {
                data.toff_samples.remove(0);
            }
            data.toff_samples.push(offset);
        }
    }
}

/// Helper: extract f64 from JSON, returns NaN if missing
fn json_f64(msg: &Value, key: &str) -> f64 {
    msg[key].as_f64().unwrap_or(f64::NAN)
}
```

### 3.5 SKY Extraction

```rust
fn extract_sky(msg: &Value, data: &mut GPSData) {
    // DOP values
    data.dop.hdop = json_f64(msg, "hdop");
    data.dop.vdop = json_f64(msg, "vdop");
    data.dop.pdop = json_f64(msg, "pdop");
    data.dop.gdop = json_f64(msg, "gdop");
    data.dop.tdop = json_f64(msg, "tdop");
    data.dop.xdop = json_f64(msg, "xdop");
    data.dop.ydop = json_f64(msg, "ydop");

    // Satellites (only replace if array present and non-empty)
    if let Some(sats) = msg["satellites"].as_array() {
        if !sats.is_empty() {
            data.satellites = sats.iter().map(|s| SatelliteInfo {
                prn: s["PRN"].as_u64().unwrap_or(0) as u16,
                gnssid: s["gnssid"].as_u64().unwrap_or(0) as u8,
                svid: s["svid"].as_u64().unwrap_or(0) as u16,
                elevation: json_f64(s, "el"),
                azimuth: json_f64(s, "az"),
                snr: json_f64(s, "ss"),
                used: s["used"].as_bool().unwrap_or(false),
                sigid: s["sigid"].as_u64().unwrap_or(0) as u8,
                health: s["health"].as_u64().unwrap_or(0) as u8,
                freqid: s["freqid"].as_i64().map(|v| v as i8),
            }).collect();
            data.satellites_used = data.satellites.iter()
                .filter(|s| s.used).count() as u32;
        }
    }
}
```

### 3.6 PPS, TOFF, Device, Version Extraction

Straightforward field mapping from JSON. PPS example:

```rust
fn extract_pps(msg: &Value, data: &mut GPSData) {
    data.pps.real_sec = msg["real_sec"].as_i64().unwrap_or(0);
    data.pps.real_nsec = msg["real_nsec"].as_i64().unwrap_or(0);
    data.pps.clock_sec = msg["clock_sec"].as_i64().unwrap_or(0);
    data.pps.clock_nsec = msg["clock_nsec"].as_i64().unwrap_or(0);
    data.pps.precision = msg["precision"].as_i64().unwrap_or(0) as i32;
    data.pps.qerr = msg["qErr"].as_i64().unwrap_or(0);
}
```

### 3.7 Armed TOFF

The gpsd client needs an `AtomicBool` or channel to receive "arm TOFF" requests:

```rust
pub struct GpsdClient {
    toff_armed: Arc<AtomicBool>,
    // ...
}
```

In TPV extraction, after computing offset:
```rust
if self.toff_armed.compare_exchange(true, false, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
    data.toff_armed_offset = offset;
    data.toff_armed_gps_time = time_str.to_string();
    data.toff_armed_sys_time = receipt_time;
}
```

---

## Phase 4: Constants and Formatting

### 4.1 Constants (`constants.rs`)

```rust
use ratatui::style::Color;

pub const GNSS_NAMES: &[(u8, &str)] = &[
    (0, "GPS"), (1, "SBAS"), (2, "Galileo"), (3, "BeiDou"),
    (4, "IMES"), (5, "QZSS"), (6, "GLONASS"), (7, "NavIC"),
];

pub const GNSS_SHORT: &[(u8, &str)] = &[
    (0, "GP"), (1, "SB"), (2, "GA"), (3, "BD"),
    (4, "IM"), (5, "QZ"), (6, "GL"), (7, "IR"),
];

pub const GNSS_COLORS: &[(u8, Color)] = &[
    (0, Color::Green),       // GPS
    (1, Color::Yellow),      // SBAS
    (2, Color::Blue),        // Galileo
    (3, Color::Red),         // BeiDou
    (4, Color::Magenta),     // IMES
    (5, Color::LightCyan),   // QZSS
    (6, Color::Cyan),        // GLONASS
    (7, Color::LightMagenta),// NavIC
];

pub fn gnss_name(id: u8) -> &'static str { ... }
pub fn gnss_short(id: u8) -> &'static str { ... }
pub fn gnss_color(id: u8) -> Color { ... }

// Fix mode display
pub fn mode_name(mode: FixMode) -> &'static str {
    match mode {
        FixMode::Unknown => "Unknown",
        FixMode::NoFix => "No Fix",
        FixMode::Fix2D => "2D Fix",
        FixMode::Fix3D => "3D Fix",
    }
}

pub fn mode_color(mode: FixMode) -> Color {
    match mode {
        FixMode::Unknown | FixMode::NoFix => Color::Red,
        FixMode::Fix2D => Color::Yellow,
        FixMode::Fix3D => Color::Green,
    }
}

// Fix status display - 10 variants with names and colors
pub fn status_name(status: FixStatus) -> &'static str { ... }
pub fn status_color(status: FixStatus) -> Color { ... }

// DOP ratings
pub fn dop_rating(value: f64) -> (&'static str, Color) {
    if !value.is_finite() { return ("---", Color::DarkGray); }
    match value {
        v if v < 1.0 => ("Ideal", Color::Green),
        v if v < 2.0 => ("Excellent", Color::Green),
        v if v < 5.0 => ("Good", Color::Yellow),
        v if v < 10.0 => ("Moderate", Color::Yellow),
        v if v < 20.0 => ("Fair", Color::Red),
        _ => ("Poor", Color::Red),
    }
}

// 16-point compass
pub const COMPASS_POINTS: &[&str] = &[
    "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE",
    "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW",
];

pub fn bearing_to_compass(degrees: f64) -> &'static str {
    if !degrees.is_finite() { return "---"; }
    let idx = ((degrees + 11.25) % 360.0 / 22.5) as usize;
    COMPASS_POINTS[idx.min(15)]
}
```

### 4.2 Formatting (`formatting.rs`)

```rust
/// Format a float with fallback for NaN
pub fn fmt(value: f64, decimals: usize, suffix: &str) -> String {
    if value.is_finite() {
        format!("{:.prec$}{}", value, suffix, prec = decimals)
    } else {
        "---".to_string()
    }
}

/// Format coordinate in DD, DMS, or DDM
pub fn fmt_coord(value: f64, axis: &str, style: &str) -> String {
    if !value.is_finite() { return "---".to_string(); }

    let dir = match axis {
        "lat" => if value >= 0.0 { "N" } else { "S" },
        _ => if value >= 0.0 { "E" } else { "W" },
    };
    let abs = value.abs();

    match style {
        "dms" => {
            let d = abs as u32;
            let m = ((abs - d as f64) * 60.0) as u32;
            let s = (abs - d as f64 - m as f64 / 60.0) * 3600.0;
            format!("{}° {:02}' {:06.3}\" {}", d, m, s, dir)
        }
        "ddm" => {
            let d = abs as u32;
            let m = (abs - d as f64) * 60.0;
            format!("{}° {:09.6}' {}", d, m, dir)
        }
        _ => format!("{:.7}° {}", abs, dir), // "dd"
    }
}

/// Convert m/s to display unit
pub fn fmt_speed(mps: f64, unit: &str) -> String {
    if !mps.is_finite() { return "---".to_string(); }
    match unit {
        "imperial" => format!("{:.1} mph", mps * 2.236936),
        "nautical" => format!("{:.1} kn", mps * 1.943844),
        _ => format!("{:.1} km/h", mps * 3.6),
    }
}

/// Convert meters to display unit
pub fn fmt_altitude(meters: f64, unit: &str) -> String {
    if !meters.is_finite() { return "---".to_string(); }
    match unit {
        "imperial" | "nautical" => format!("{:.1} ft", meters * 3.28084),
        _ => format!("{:.1} m", meters),
    }
}

/// Parse ISO 8601 time string into (date, time) parts
pub fn fmt_time_iso(iso: &str) -> (String, String) {
    if iso.is_empty() { return ("---".to_string(), "---".to_string()); }
    if let Some(pos) = iso.find('T') {
        let date = &iso[..pos];
        let time = iso[pos+1..].trim_end_matches('Z');
        (date.to_string(), time.to_string())
    } else {
        (iso.to_string(), "---".to_string())
    }
}

/// Format time offset intelligently (ns/us/ms/s)
pub fn fmt_offset(offset_sec: f64) -> String {
    if !offset_sec.is_finite() { return "---".to_string(); }
    let abs = offset_sec.abs();
    let sign = if offset_sec < 0.0 { "-" } else { "+" };
    if abs < 1e-6 {
        format!("{}{:.1} ns", sign, abs * 1e9)
    } else if abs < 1e-3 {
        format!("{}{:.3} us", sign, abs * 1e6)
    } else if abs < 1.0 {
        format!("{}{:.3} ms", sign, abs * 1e3)
    } else {
        format!("{}{:.3} s", sign, abs)
    }
}
```

---

## Phase 5: Application Shell

### 5.1 App State (`app.rs`)

```rust
pub enum ActiveTab {
    Dashboard,
    Satellites,
    Timing,
    Device,
    Nmea,
}

pub enum UnitSystem {
    Metric,
    Imperial,
    Nautical,
}

pub enum CoordFormat {
    DD,
    DMS,
    DDM,
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
    pub nmea_buffer: VecDeque<String>,  // max 1000
    pub nmea_paused: bool,
    pub nmea_filter: String,            // "" = all, or "GGA", "RMC", etc.

    // Device config state
    pub device_config_log: Vec<String>,

    // Clock sync
    pub armed_clock_set: bool,
    pub armed_toff: Arc<AtomicBool>,
}
```

### 5.2 Event Loop (`app.rs`)

```rust
pub async fn run(terminal: &mut Terminal<impl Backend>, host: &str, port: u16) -> Result<()> {
    let mut app = App::new(host.to_string(), port);

    // Channel for gpsd events
    let (tx, mut rx) = mpsc::channel(100);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Spawn gpsd task
    let gpsd_handle = tokio::spawn(gpsd_client::gpsd_task(
        host.to_string(), port, tx.clone(), shutdown_rx,
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
```

### 5.3 Input Handling

```rust
impl App {
    pub fn handle_event(&mut self, event: crossterm::event::Event) {
        if self.show_settings {
            self.handle_settings_input(event);
            return;
        }

        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press { return; }
            match key.code {
                KeyCode::Char('q') => self.should_quit = true,
                KeyCode::Char('t') => self.cycle_theme(),
                KeyCode::Char('s') => self.show_settings = true,
                KeyCode::Char('r') => self.reconnect(),
                KeyCode::Char('u') => self.cycle_units(),
                KeyCode::Char('m') => self.open_maps(),
                KeyCode::Char('l') => self.toggle_logging(),
                KeyCode::Char('h') => self.toggle_hold(),
                KeyCode::Tab => self.next_tab(),
                KeyCode::BackTab => self.prev_tab(),
                // Tab-specific keys handled per active tab
                _ => self.handle_tab_input(key),
            }
        }
    }
}
```

---

## Phase 6: Dashboard Widgets

### 6.1 Layout (`ui/dashboard.rs`)

Use Ratatui's `Layout` to create a 3x3 grid:

```rust
pub fn render_dashboard(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Ratio(3, 10),  // row 1
        Constraint::Ratio(4, 10),  // row 2
        Constraint::Ratio(3, 10),  // row 3
    ]).split(area);

    let row1 = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ]).split(rows[0]);

    let row2 = Layout::horizontal([
        Constraint::Ratio(2, 3),  // sky plot (2-col span)
        Constraint::Ratio(1, 3),  // signal chart
    ]).split(rows[1]);

    let row3 = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ]).split(rows[2]);

    position::render(f, row1[0], app);
    fix::render(f, row1[1], app);
    velocity::render(f, row1[2], app);
    sky_plot::render(f, row2[0], app);
    signal_chart::render(f, row2[1], app);
    error_panel::render(f, row3[0], app);
    device_panel::render(f, row3[1], app);
    time_panel::render(f, row3[2], app, false);
}
```

### 6.2 Position Panel (`ui/position.rs`)

```rust
pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Position ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let data = &app.gps_data;
    let cf = &app.coord_format;
    let unit = &app.units;

    let mut lines = vec![
        Line::from(vec![
            Span::raw("Lat: "),
            Span::styled(
                fmt_coord(data.latitude, "lat", cf.as_str()),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(vec![
            Span::raw("Lon: "),
            Span::styled(
                fmt_coord(data.longitude, "lon", cf.as_str()),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(format!("Alt HAE: {}", fmt_altitude(data.alt_hae, unit.as_str()))),
        Line::from(format!("Alt MSL: {}", fmt_altitude(data.alt_msl, unit.as_str()))),
        Line::from(format!("Geoid:   {}", fmt(data.geoid_sep, 1, " m"))),
    ];

    // If position hold active, append statistics
    if let Some(hold) = &app.position_hold {
        if let Some(result) = hold.result() {
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::raw("CEP50: "),
                Span::styled(
                    format!("{:.2} m", result.cep50),
                    cep_color(result.cep50),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::raw("CEP95: "),
                Span::styled(
                    format!("{:.2} m", result.cep95),
                    cep_color(result.cep95),
                ),
            ]));
        }
    }

    f.render_widget(Paragraph::new(lines), inner);
}
```

### 6.3 Fix Panel (`ui/fix.rs`)

Display fix mode with color, status, satellite counts, and DOP values with color-coded ratings.

```rust
pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Fix ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let data = &app.gps_data;
    let mut lines = vec![];

    // Mode line with color
    lines.push(Line::from(vec![
        Span::raw("Mode:   "),
        Span::styled(mode_name(data.mode), Style::default().fg(mode_color(data.mode)).bold()),
    ]));

    // Status line with color
    lines.push(Line::from(vec![
        Span::raw("Status: "),
        Span::styled(status_name(data.status), Style::default().fg(status_color(data.status))),
    ]));

    // Satellites
    lines.push(Line::from(format!(
        "Sats:   {}/{}",
        data.satellites_used,
        data.satellites.len()
    )));

    lines.push(Line::raw(""));

    // DOP values with ratings
    for (label, value) in [
        ("HDOP", data.dop.hdop), ("VDOP", data.dop.vdop),
        ("PDOP", data.dop.pdop), ("GDOP", data.dop.gdop),
    ] {
        let (rating, color) = dop_rating(value);
        lines.push(Line::from(vec![
            Span::raw(format!("{}: ", label)),
            Span::styled(fmt(value, 1, ""), Style::default().fg(color)),
            Span::styled(format!(" ({})", rating), Style::default().fg(color).dim()),
        ]));
    }

    f.render_widget(Paragraph::new(lines), inner);
}
```

### 6.4 Velocity Panel (`ui/velocity.rs`)

Display speed (unit-converted), track with compass direction, magnetic track, climb, magnetic variation.

### 6.5 Sky Plot (`ui/sky_plot.rs`)

This is the most complex widget. ASCII polar projection of satellite positions.

```rust
pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Sky ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let w = inner.width as f64;
    let h = inner.height as f64;
    let cx = w / 2.0;
    let cy = h / 2.0;
    let radius = cx.min(cy * 2.0) - 1.0; // aspect ratio: chars are ~2:1

    // Create a buffer of styled characters
    let mut canvas: Vec<Vec<(char, Style)>> = vec![
        vec![(' ', Style::default()); inner.width as usize];
        inner.height as usize
    ];

    // Draw elevation rings (0°, 30°, 60°)
    draw_ring(&mut canvas, cx, cy, radius, 1.0, '·');       // 0° (horizon)
    draw_ring(&mut canvas, cx, cy, radius, 2.0/3.0, '·');   // 30°
    draw_ring(&mut canvas, cx, cy, radius, 1.0/3.0, '·');   // 60°

    // Draw crosshairs
    draw_hline(&mut canvas, cy as usize, '·');
    draw_vline(&mut canvas, cx as usize, '·');

    // Cardinal labels
    plot_char(&mut canvas, cx, 0.0, 'N', Style::default().fg(Color::White).bold());
    plot_char(&mut canvas, cx, h - 1.0, 'S', Style::default().fg(Color::White).bold());
    plot_char(&mut canvas, 0.0, cy, 'W', Style::default().fg(Color::White).bold());
    plot_char(&mut canvas, w - 1.0, cy, 'E', Style::default().fg(Color::White).bold());

    // Plot satellites
    for sat in &app.gps_data.satellites {
        if !sat.elevation.is_finite() || !sat.azimuth.is_finite() { continue; }
        let r = (90.0 - sat.elevation) / 90.0 * radius;
        let az_rad = sat.azimuth.to_radians();
        let x = cx + r * az_rad.sin() / 2.0; // /2 for aspect ratio
        let y = cy - r * az_rad.cos();

        let color = gnss_color(sat.gnssid);
        let (ch, style) = if sat.used {
            ('#', Style::default().fg(color).bold())
        } else {
            ('o', Style::default().fg(color))
        };
        plot_char(&mut canvas, x, y, ch, style);
    }

    // Render canvas to buffer
    for (row_idx, row) in canvas.iter().enumerate() {
        let spans: Vec<Span> = row.iter().map(|(ch, style)| {
            Span::styled(ch.to_string(), *style)
        }).collect();
        let line = Line::from(spans);
        let y = inner.y + row_idx as u16;
        if y < inner.y + inner.height {
            f.render_widget(Paragraph::new(vec![line]),
                Rect::new(inner.x, y, inner.width, 1));
        }
    }
}
```

### 6.6 Signal Chart (`ui/signal_chart.rs`)

Horizontal bar chart of satellite SNR values:

```rust
pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Signal ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let data = &app.gps_data;
    let mut sats: Vec<&SatelliteInfo> = data.satellites.iter()
        .filter(|s| s.snr.is_finite())
        .collect();

    // Sort: used first, then by SNR descending
    sats.sort_by(|a, b| {
        b.used.cmp(&a.used)
            .then(b.snr.partial_cmp(&a.snr).unwrap_or(std::cmp::Ordering::Equal))
    });

    let max_snr = 55.0;
    let bar_width = inner.width.saturating_sub(8) as f64; // leave room for label

    for (i, sat) in sats.iter().enumerate().take(inner.height as usize) {
        let label = format!("{:>2}{:>3}", gnss_short(sat.gnssid), sat.svid);
        let filled = (sat.snr / max_snr * bar_width).round() as usize;
        let empty = bar_width as usize - filled.min(bar_width as usize);

        let color = gnss_color(sat.gnssid);
        let used_marker = if sat.used { "+" } else { " " };

        let line = Line::from(vec![
            Span::styled(used_marker, Style::default().fg(Color::White)),
            Span::styled(&label, Style::default().fg(color)),
            Span::raw(" "),
            Span::styled("█".repeat(filled), Style::default().fg(color)),
            Span::styled("░".repeat(empty), Style::default().fg(Color::DarkGray)),
        ]);

        f.render_widget(Paragraph::new(vec![line]),
            Rect::new(inner.x, inner.y + i as u16, inner.width, 1));
    }
}
```

### 6.7 Error Panel (`ui/error_panel.rs`)

Display error estimates (EPH, EPV, EPT, EPS, EPD, EPC) with `+-` prefix and units.

### 6.8 Device Panel (`ui/device_panel.rs`)

Display device path, driver, baud rate, cycle time, gpsd version.

### 6.9 Time Panel (`ui/time_panel.rs`)

Two modes:
- **Basic** (dashboard): GPS date, time, time error, leap seconds
- **Detailed** (timing tab, `show_pps=true`): PPS offset with quality badge, TOFF current/armed/stats, TDOP

---

## Phase 7: Satellite Tab

### 7.1 Constellation Panel (`ui/constellation.rs`)

Two-column layout showing per-constellation used/visible counts, colored by GNSS. Total row at bottom.

### 7.2 Satellite Table (`ui/satellite_table.rs`)

Use Ratatui's `Table` widget:

```rust
pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let mut rows: Vec<Row> = app.gps_data.satellites.iter()
        .map(|sat| {
            let color = gnss_color(sat.gnssid);
            let snr_color = if sat.snr.is_finite() {
                if sat.snr > 30.0 { Color::Green }
                else if sat.snr > 20.0 { Color::Yellow }
                else { Color::Red }
            } else { Color::DarkGray };

            Row::new(vec![
                Cell::from(gnss_name(sat.gnssid)).style(Style::default().fg(color)),
                Cell::from(format!("{}", sat.prn)),
                Cell::from(format!("{}", sat.svid)),
                Cell::from(fmt(sat.elevation, 0, "°")),
                Cell::from(fmt(sat.azimuth, 0, "°")),
                Cell::from(fmt(sat.snr, 1, "")).style(Style::default().fg(snr_color)),
                Cell::from(if sat.used { "*" } else { "" })
                    .style(Style::default().fg(Color::Green)),
                Cell::from(format!("{}", sat.sigid)),
                Cell::from(if sat.health == 1 { "OK".to_string() } else { format!("{}", sat.health) })
                    .style(if sat.health == 1 { Style::default().fg(Color::Green) }
                           else { Style::default().fg(Color::Red) }),
            ])
        }).collect();

    // Sort by gnssid then PRN
    rows.sort_by(/* gnssid, prn */);

    let header = Row::new(["GNSS", "PRN", "SV", "El", "Az", "SNR", "U", "Sig", "Health"])
        .style(Style::default().bold());

    let widths = [
        Constraint::Length(8), Constraint::Length(5), Constraint::Length(4),
        Constraint::Length(5), Constraint::Length(5), Constraint::Length(5),
        Constraint::Length(2), Constraint::Length(4), Constraint::Length(7),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::bordered().title(" Satellites "));

    f.render_widget(table, area);
}
```

---

## Phase 8: Timing Tab

### 8.1 Layout

Vertical layout:
1. TimePanel (detailed mode with PPS/TOFF)
2. TOFF control buttons (Arm TOFF, Clear TOFF)
3. DevicePanel

### 8.2 TOFF Controls

Render as simple text buttons. Handle specific keys when on Timing tab (e.g., `a` to arm, `c` to clear).

### 8.3 TOFF Statistics

Compute from `gps_data.toff_samples`:
```rust
fn toff_stats(samples: &[f64]) -> Option<(f64, f64, f64, f64)> {
    if samples.is_empty() { return None; }
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    let variance = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    let std = variance.sqrt();
    let min = samples.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    Some((mean, std, min, max))
}
```

---

## Phase 9: Device Configuration Tab

### 9.1 Architecture

The device config tab runs `ubxtool` and `gpsctl` commands via `tokio::process::Command`. Output is captured and displayed in a scrollable log area.

### 9.2 State

```rust
pub struct DeviceConfigState {
    pub platform_model: usize,      // index into model list
    pub nav_rate: usize,            // index into rate list
    pub power_mode: usize,          // index into power mode list
    pub serial_speed: usize,        // index into baud rate list
    pub pps_frequency: usize,
    pub pps_duration: usize,
    pub gnss_enabled: [bool; 6],    // GPS, GLONASS, Galileo, BeiDou, SBAS, QZSS
    pub raw_command: String,
    pub output_log: Vec<String>,    // scrollable output
    pub selected_control: usize,    // which control is focused
    pub proto_version: String,      // "18" default for u-blox 8
}
```

### 9.3 Command Execution

```rust
async fn run_ubxtool(args: &[&str], proto_ver: &str) -> Result<String> {
    let output = Command::new("ubxtool")
        .arg("-P").arg(proto_ver)
        .args(args)
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string()
       + &String::from_utf8_lossy(&output.stderr))
}
```

### 9.4 Controls to Implement

- Platform model: `ubxtool -p MODEL,<model_id>`
- Nav rate: `ubxtool -z CFG-RATE-MEAS,<ms>`
- Power mode: `ubxtool -p PMS,<mode>`
- Serial speed: `ubxtool -z CFG-PRT-1-BAUDRATE,<baud>` + `gpsctl -s <baud> <device>`
- PPS config: Pack TP5 binary message
- Constellation toggles: `ubxtool -e/-d <constellation>`
- Save config: `ubxtool -p SAVE`
- Cold boot: `ubxtool -p COLDBOOT`
- Clock sync: Uses `clock_sync` module

### 9.5 Interaction Model

Navigate with arrow keys, activate with Enter. Tab-specific keybindings when Device tab is active.

---

## Phase 10: NMEA Viewer Tab

### 10.1 State

```rust
pub struct NmeaViewerState {
    pub buffer: VecDeque<String>,   // max 1000
    pub paused: bool,
    pub pause_buffer: VecDeque<String>,
    pub filter: String,             // "" = all, "GGA", "RMC", etc.
    pub scroll_offset: usize,
}
```

### 10.2 NMEA Sentence Handling

From gpsd client, NMEA sentences arrive as raw strings. Extract type:

```rust
fn nmea_type(sentence: &str) -> &str {
    // "$GPGGA,..." -> "GGA" (skip $ + 2-char talker ID)
    if sentence.len() > 6 && sentence.starts_with('$') {
        &sentence[3..6]
    } else {
        "???"
    }
}
```

### 10.3 Color Coding

```rust
fn nmea_color(sentence_type: &str) -> Color {
    match sentence_type {
        "GGA" => Color::Green,
        "RMC" => Color::Blue,
        "GSA" => Color::Yellow,
        "GSV" => Color::LightYellow,
        "VTG" => Color::Cyan,
        "GLL" => Color::Magenta,
        "ZDA" => Color::LightCyan,
        "TXT" => Color::DarkGray,
        _ => Color::White,
    }
}
```

### 10.4 Rendering

Scrollable list of colored NMEA sentences. Controls: `p` pause/resume, `c` clear, `f` cycle filter.

---

## Phase 11: Settings Overlay

### 11.1 Modal Overlay

Render as a centered popup over the current tab content:

```rust
pub fn render_settings(f: &mut Frame, area: Rect, app: &App) {
    // Semi-transparent overlay
    let overlay = Block::default()
        .style(Style::default().bg(Color::Black));
    f.render_widget(Clear, area);

    // Centered dialog (60x20)
    let dialog_area = centered_rect(60, 20, area);
    let block = Block::bordered()
        .title(" Settings ")
        .style(Style::default().bg(Color::DarkGray));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    // Fields: Host, Port, Units, Coord Format
    // Navigation: Tab between fields, Enter to edit, Esc to cancel, Ctrl+S to apply
}
```

### 11.2 Settings Fields

- Host (text input)
- Port (text input)
- Units (cycle: metric/imperial/nautical)
- Coordinate format (cycle: DD/DMS/DDM)

---

## Phase 12: GPS Logger

### 12.1 GPX Format

```rust
pub struct GpsLogger {
    file: Option<BufWriter<File>>,
    format: LogFormat,
    point_count: u32,
    last_time: String,
    active: bool,
}

pub enum LogFormat {
    Gpx,
    Csv,
}
```

### 12.2 GPX Writing

```rust
fn start_gpx(&mut self) -> Result<()> {
    let filename = format!("tuigps_{}.gpx",
        chrono::Local::now().format("%Y%m%d_%H%M%S"));
    let mut file = BufWriter::new(File::create(&filename)?);
    writeln!(file, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
    writeln!(file, r#"<gpx version="1.0">"#)?;
    writeln!(file, r#"<trk><trkseg>"#)?;
    self.file = Some(file);
    Ok(())
}

fn log_point_gpx(&mut self, data: &GPSData) -> Result<()> {
    if !data.has_fix() { return Ok(()); }
    if data.time == self.last_time { return Ok(()); } // deduplicate
    self.last_time = data.time.clone();

    if let Some(ref mut file) = self.file {
        writeln!(file, r#"<trkpt lat="{}" lon="{}">"#, data.latitude, data.longitude)?;
        if data.alt_msl.is_finite() {
            writeln!(file, "<ele>{}</ele>", data.alt_msl)?;
        }
        writeln!(file, "<time>{}</time>", data.time)?;
        writeln!(file, "</trkpt>")?;
        self.point_count += 1;
    }
    Ok(())
}

fn stop_gpx(&mut self) -> Result<()> {
    if let Some(ref mut file) = self.file {
        writeln!(file, "</trkseg></trk></gpx>")?;
        file.flush()?;
    }
    self.file = None;
    Ok(())
}
```

---

## Phase 13: Position Hold

### 13.1 Welford's Algorithm

```rust
pub struct PositionHold {
    count: u64,
    mean_lat: f64,
    mean_lon: f64,
    mean_alt: f64,
    m2_lat: f64,
    m2_lon: f64,
    m2_alt: f64,
    start_time: Instant,
}

pub struct HoldResult {
    pub mean_lat: f64,
    pub mean_lon: f64,
    pub mean_alt: f64,
    pub std_lat: f64,
    pub std_lon: f64,
    pub std_alt: f64,
    pub cep50: f64,
    pub cep95: f64,
    pub count: u64,
    pub duration: Duration,
}

impl PositionHold {
    pub fn add_fix(&mut self, lat: f64, lon: f64, alt: f64) {
        self.count += 1;
        let n = self.count as f64;

        // Welford update for each dimension
        let delta_lat = lat - self.mean_lat;
        self.mean_lat += delta_lat / n;
        self.m2_lat += delta_lat * (lat - self.mean_lat);

        // Same for lon, alt...
    }

    pub fn result(&self) -> Option<HoldResult> {
        if self.count < 2 { return None; }
        let n = self.count as f64;
        let std_lat = (self.m2_lat / (n - 1.0)).sqrt();
        let std_lon = (self.m2_lon / (n - 1.0)).sqrt();
        let std_alt = (self.m2_alt / (n - 1.0)).sqrt();

        // Convert to meters
        const M_PER_DEG_LAT: f64 = 110540.0;
        let m_per_deg_lon = 111320.0 * self.mean_lat.to_radians().cos();

        let std_north = std_lat * M_PER_DEG_LAT;
        let std_east = std_lon * m_per_deg_lon;

        let cep50 = 0.5887 * (std_north + std_east);
        let cep95 = 2.146 * cep50;

        Some(HoldResult { /* ... */ })
    }
}
```

---

## Phase 14: Clock Sync

### 14.1 GPS Time Clock Sync (`clock_sync.rs`)

```rust
pub fn set_clock_from_gps(gps_time_str: &str, last_seen: f64) -> Result<String> {
    // Disable NTP first
    let _ = Command::new("timedatectl").args(["set-ntp", "false"]).output();

    // Parse GPS time and compensate for age
    let gps_time = DateTime::parse_from_rfc3339(gps_time_str)?;
    let fix_age = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64() - last_seen;
    let adjusted = gps_time + chrono::Duration::milliseconds((fix_age * 1000.0) as i64);

    // Method 1: D-Bus SetTime
    #[cfg(feature = "dbus")]
    if try_dbus_set_time(adjusted).is_ok() {
        return Ok(format!("Clock set via D-Bus (fix age: {:.1}s)", fix_age));
    }

    // Method 2: timedatectl
    let time_str = adjusted.format("%Y-%m-%d %H:%M:%S").to_string();
    if Command::new("timedatectl").args(["set-time", &time_str]).output()?.status.success() {
        return Ok(format!("Clock set via timedatectl (fix age: {:.1}s)", fix_age));
    }

    // Method 3: sudo date
    let utc_str = adjusted.format("%Y-%m-%dT%H:%M:%S").to_string();
    Command::new("sudo").args(["-n", "date", "-u", "-s", &utc_str]).output()?;
    Ok(format!("Clock set via sudo date (fix age: {:.1}s)", fix_age))
}
```

### 14.2 PPS Clock Sync

Uses Linux PPS ioctl to capture pulse edge timing:

```rust
use nix::ioctl_readwrite;

#[repr(C)]
struct PpsKtime {
    sec: i64,
    nsec: i32,
    flags: u32,
}

#[repr(C)]
struct PpsKinfo {
    assert_tu: PpsKtime,
    clear_tu: PpsKtime,
    assert_sequence: u32,
    clear_sequence: u32,
    current_mode: i32,
}

// PPS_FETCH ioctl
ioctl_readwrite!(pps_fetch, b'p', 0xa1, PpsKinfo);

pub fn sync_from_pps(device_path: &str) -> Result<String> {
    // Open /dev/pps0 (or whatever)
    // Call pps_fetch ioctl to get assert edge timestamp
    // Compute offset between target GPS second and kernel CLOCK_REALTIME at pulse
    // Apply via D-Bus or sudo date
}
```

---

## Phase 15: Footer Status Bar

### 15.1 Connection Status (`ui/status_bar.rs`)

Render a single-line footer:

```rust
pub fn render_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let data = &app.gps_data;

    let mut spans = vec![];

    // Key bindings (abbreviated)
    for (key, action) in &[
        ("q", "quit"), ("t", "theme"), ("s", "settings"),
        ("r", "reconnect"), ("u", "units"), ("m", "maps"),
        ("l", "log"), ("h", "hold"),
    ] {
        spans.push(Span::styled(
            format!(" {} ", key),
            Style::default().fg(Color::White).bg(Color::DarkGray),
        ));
        spans.push(Span::raw(format!("{} ", action)));
    }

    // Activity badges
    if app.logger.as_ref().map_or(false, |l| l.active) {
        spans.push(Span::styled(" REC ", Style::default().fg(Color::White).bg(Color::Red)));
        spans.push(Span::raw(format!(" {} pts ", app.logger.as_ref().unwrap().point_count)));
    }
    if app.position_hold.is_some() {
        spans.push(Span::styled(" HOLD ", Style::default().fg(Color::Black).bg(Color::Cyan)));
    }

    // GPS status (right-aligned)
    let status_span = if !data.connected {
        Span::styled("DISCONNECTED", Style::default().fg(Color::Red))
    } else {
        let age = SystemTime::now().duration_since(UNIX_EPOCH)
            .unwrap().as_secs_f64() - data.last_seen;
        if age > 10.0 {
            Span::styled(format!("STALE ({:.0}s)", age), Style::default().fg(Color::Yellow))
        } else {
            // Constellation breakdown: "4GP+2GA"
            let counts = data.constellation_counts();
            let parts: Vec<String> = counts.iter()
                .filter(|(_, (_, used))| *used > 0)
                .map(|(id, (_, used))| format!("{}{}", used, gnss_short(*id)))
                .collect();
            Span::styled(parts.join("+"), Style::default().fg(Color::Green))
        }
    };
    spans.push(status_span);

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
```

---

## Implementation Notes

### NaN Handling
- Initialize all `f64` fields to `f64::NAN`
- Always check `.is_finite()` before display or computation
- Display "---" for NaN values

### Thread Safety
- gpsd runs in a tokio task, communicates via `mpsc::channel`
- Device config commands run in spawned tokio tasks
- Clock sync (armed mode) must execute on the gpsd task for minimum latency — use a separate channel to send clock sync requests to the gpsd task

### Terminal Restoration
- Always restore terminal in a panic handler or `Drop` impl:
```rust
let original_hook = std::panic::take_hook();
std::panic::set_hook(Box::new(move |info| {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    original_hook(info);
}));
```

### Color Themes
Ratatui doesn't have built-in themes like Textual. Options:
- Define a `Theme` struct with named colors
- Cycle between light/dark by swapping the theme struct
- Use terminal's own palette for basic theming

### Scrolling
For scrollable content (NMEA viewer, device config log, satellite table):
- Track `scroll_offset` in app state
- Use `List` or `Table` with `.offset()` or manual slice
- Handle Up/Down/PageUp/PageDown keys

### Performance
- `GPSData.clone()` is called on every update — keep satellite vectors reasonable
- Render only the active tab's widgets
- NMEA buffer capped at 1000 entries

---

## Testing Strategy

### Unit Tests
- `data_model.rs`: Default values, enum conversions, `has_fix()`, `constellation_counts()`
- `formatting.rs`: All formatters with normal values, NaN, edge cases
- `constants.rs`: `bearing_to_compass()`, `dop_rating()`
- `position_hold.rs`: Welford accumulation, CEP computation
- `gpsd_client.rs`: JSON message parsing (mock gpsd JSON strings)

### Integration Tests
- Connect to `gpsfake -c 0.5 /usr/share/gpsd/sample.nmea`
- Verify data flows through to `GPSData`
- Verify reconnection after disconnect

### Manual Testing
```bash
# Start simulated GPS
gpsfake -c 0.5 /usr/share/gpsd/sample.nmea

# Run app
cargo run

# Verify each tab renders correctly
# Test keyboard shortcuts
# Test settings overlay
# Test logging (check output file)
# Test position hold (accumulate for 30s, check CEP)
```

---

## Implementation Order (Recommended)

1. **Scaffold** — Cargo.toml, main.rs, terminal setup/teardown
2. **Data model** — all structs and enums
3. **Constants + formatting** — pure functions, easy to test
4. **gpsd client** — TCP connection, JSON parsing, channel
5. **App shell** — event loop, tab switching, keybindings
6. **Dashboard tab** — all 8 widgets (start with position, fix, velocity)
7. **Sky plot + signal chart** — visual widgets
8. **Footer status bar** — connection status
9. **Satellite tab** — constellation panel + table
10. **Timing tab** — PPS/TOFF display + controls
11. **NMEA viewer** — raw stream display
12. **Settings overlay** — modal dialog
13. **GPS logger** — GPX/CSV output
14. **Position hold** — Welford + CEP
15. **Device config** — ubxtool integration
16. **Clock sync** — D-Bus/timedatectl/PPS ioctl
