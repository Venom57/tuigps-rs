# tuigps-rs

Terminal UI GPS monitoring tool using gpsd, written in Rust.

## Tech Stack
- **Rust 2024 edition** (1.75+)
- **Ratatui** — TUI framework (successor to tui-rs)
- **Crossterm** — terminal backend for Ratatui
- **gpsd_proto** or custom JSON parser — gpsd protocol client
- **tokio** — async runtime for gpsd connection and subprocess management
- **serde/serde_json** — JSON deserialization of gpsd messages
- Threaded architecture: gpsd reader thread communicates with TUI via `mpsc` channels

## Project Structure
- `src/main.rs` — entry point, CLI arg parsing, terminal setup/teardown
- `src/app.rs` — application state, event loop, tab management, keybindings
- `src/gpsd_client.rs` — async gpsd connection with auto-reconnect
- `src/data_model.rs` — structs for GPS state (GPSData, SatelliteInfo, DOPValues, etc.)
- `src/constants.rs` — GNSS names, colors, status maps, DOP ratings
- `src/formatting.rs` — NaN-safe formatting helpers
- `src/gps_logger.rs` — GPX/CSV file logging
- `src/position_hold.rs` — Welford's algorithm for position averaging
- `src/clock_sync.rs` — system clock sync via D-Bus/timedatectl/sudo
- `src/ui/` — rendering functions per tab/widget
  - `src/ui/mod.rs` — top-level layout dispatch
  - `src/ui/dashboard.rs` — 3x3 grid layout
  - `src/ui/position.rs` — lat/lon/alt display
  - `src/ui/fix.rs` — fix mode/status/DOP
  - `src/ui/velocity.rs` — speed/track/climb
  - `src/ui/sky_plot.rs` — ASCII polar satellite plot
  - `src/ui/signal_chart.rs` — SNR bar chart
  - `src/ui/error_panel.rs` — error estimates
  - `src/ui/device_panel.rs` — device info
  - `src/ui/time_panel.rs` — GPS/PPS/TOFF timing
  - `src/ui/constellation.rs` — per-constellation summary
  - `src/ui/satellite_table.rs` — detailed satellite table
  - `src/ui/nmea_viewer.rs` — raw NMEA stream
  - `src/ui/status_bar.rs` — footer connection status
  - `src/ui/device_config.rs` — u-blox configuration UI
  - `src/ui/settings.rs` — settings overlay

## Key Patterns
- All GPS data flows through a single `GPSData` struct protected by `Arc<Mutex<>>` or sent via channel
- UI rendering is pure: `fn render_widget(data: &GPSData, area: Rect, buf: &mut Buffer)`
- NaN represented as `f64::NAN`; always check `.is_finite()` before display
- gpsd client runs in a tokio task with auto-reconnect (2s delay)
- Channel-based communication: `mpsc::Sender<GPSData>` from gpsd task to main event loop
- Event loop processes: terminal events (crossterm), gpsd updates (channel), tick timer (1Hz heartbeat)

## Commands
- `cargo build --release` — build
- `cargo run` — run (debug)
- `cargo test` — run tests
- `cargo clippy` — lint
- `cargo fmt` — format

## Testing with simulated GPS
```bash
gpsfake -c 0.5 /usr/share/gpsd/sample.nmea
```

## Key Crates
- `ratatui` — TUI widgets and layout
- `crossterm` — terminal events and raw mode
- `tokio` — async runtime
- `serde`, `serde_json` — gpsd JSON parsing
- `chrono` — date/time parsing and formatting
- `clap` — CLI argument parsing
- `nix` — ioctl for PPS, clock operations
- `zbus` or `dbus` — D-Bus for clock sync (optional)
- `open` — browser launching (Google Maps)

## Git Commits
- Do NOT include `Co-Authored-By` lines in commit messages
