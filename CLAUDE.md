# tuigps-rs

Terminal UI GPS monitoring tool using gpsd, written in Rust.

## Tech Stack
- **Rust 2024 edition**, MSRV 1.88 (`rust-version` in Cargo.toml; let-chains need 1.88)
- **Ratatui** — TUI framework (successor to tui-rs)
- **Crossterm** — terminal backend for Ratatui
- Custom gpsd JSON client built on `serde_json::Value` (no serde derive)
- **tokio** — async runtime for gpsd connection and subprocess management (features: rt-multi-thread, macros, net, io-util, time, sync, process)
- gpsd reader runs as a tokio task and talks to the TUI via `mpsc` channels

## Project Structure
- `src/lib.rs` — library crate root (all modules live here)
- `src/main.rs` — thin binary: CLI arg parsing, terminal setup/teardown
- `tests/` — integration tests against the library crate
- `src/app.rs` — application state, event loop, tab management, keybindings
- `src/gpsd_client.rs` — async gpsd connection with auto-reconnect
- `src/data_model.rs` — structs for GPS state (GPSData, SatelliteInfo, DOPValues, etc.)
- `src/constants.rs` — GNSS names, colors, status maps, DOP ratings
- `src/formatting.rs` — NaN-safe formatting helpers
- `src/gps_logger.rs` — GPX/CSV file logging (only GPX is reachable from the UI)
- `src/position_hold.rs` — Welford's algorithm for position averaging
- `src/clock_sync.rs` — system clock sync via timedatectl, falling back to `sudo -n date` (D-Bus path is a stub behind the non-default `dbus` feature)
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
- `cargo deny check` — license/advisory/source checks (`deny.toml`)

CI (`.github/workflows/ci.yml`) runs fmt, clippy and tests with `-D warnings` for
default / `--no-default-features` / `--all-features`, plus an MSRV 1.88 check and
cargo-deny. Run the same gate locally before committing:
`cargo fmt --check && RUSTFLAGS=-Dwarnings cargo clippy --all-targets && cargo test`.
Tagging `v*` triggers `.github/workflows/release.yml` (cross-compiled Linux tarballs).

## Testing with simulated GPS
```bash
gpsfake -c 0.5 /usr/share/gpsd/sample.nmea
```

## Key Crates
- `ratatui` — TUI widgets and layout
- `crossterm` — terminal events and raw mode
- `tokio` — async runtime
- `serde_json` — gpsd JSON parsing (`Value` only)
- `chrono` — date/time parsing and formatting
- `clap` — CLI argument parsing
- `futures` — `StreamExt` for crossterm's `EventStream`
- `anyhow` — error handling
- `zbus` — optional, behind the non-default `dbus` feature (stub)
- `open` — browser launching (Google Maps)

## Git Commits
- Do NOT include `Co-Authored-By` lines in commit messages
