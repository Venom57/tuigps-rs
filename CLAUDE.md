# tuigps-rs

Terminal UI GPS monitoring tool using gpsd, written in Rust.

## Tech Stack
- **Rust 2024 edition**, MSRV 1.88 (`rust-version` in Cargo.toml; let-chains need 1.88)
- **Ratatui** — TUI framework (successor to tui-rs)
- **Crossterm** — terminal backend for Ratatui
- Custom gpsd JSON client built on `serde_json::Value`; `serde` derive + `toml` only for the config file
- **tokio** — async runtime for gpsd connection and subprocess management (features: rt-multi-thread, macros, net, io-util, time, sync, process)
- gpsd reader runs as a tokio task and talks to the TUI via `mpsc` channels

## Project Structure
- `src/lib.rs` — library crate root (all modules live here)
- `src/main.rs` — thin binary: CLI arg parsing, terminal setup/teardown
- `src/config.rs` — TOML config file (`$XDG_CONFIG_HOME/tuigps-rs/config.toml`) and CLI precedence
- `src/app/` — application state and behavior
  - `mod.rs` — App struct, tab/unit/coord enums, key dispatch (`run_action`), tick, logging/hold/maps toggles; re-exports the public API
  - `keys.rs` — the single key-binding table (drives dispatch, help overlay, status hints), `lookup_binding`
  - `settings.rs` — `SettingsField`, settings dialog input, apply/cancel, `validate_port`
  - `device.rs` — Device tab state, y/n confirmation, queued `DeviceCommand`s, ubxtool/gpsctl execution, `build_tp5_cmd`
  - `nmea.rs` — NMEA buffer, pause buffer, filter, scroll/follow
  - `scroll.rs` — `Scroll`, page size, satellite-table scrolling
  - `timing.rs` — `TimingState` (TOFF samples/capture), armed clock sync
  - `gpsd.rs` — applying gpsd events, per-TPV hooks, staleness, reconnect generations
  - `event_loop.rs` — `run()`, `FrameLimiter` (redraw cap), the select loop
  - `test_util.rs` — shared unit-test helpers (cfg(test))
- `src/gpsd_client.rs` — async gpsd connection with auto-reconnect; `GpsdMessage::parse`/`apply` per message class
- `src/data_model.rs` — structs for GPS state (GPSData, SatelliteInfo, DOPValues, etc.)
- `src/constants.rs` — GNSS names, colors, status maps, DOP ratings
- `src/formatting.rs` — NaN-safe formatting helpers
- `src/gps_logger.rs` — GPX/CSV file logging; `LogMode` (gpx/csv/both) selects the files, `CSV_HEADER` documents the columns (append-only)
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
  - `src/ui/help.rs` — `?` help overlay (generated from `app/keys.rs`)
- `tests/` — integration tests against the library crate (fixtures in `tests/fixtures/`):
  render smoke/display, gpsd fixtures and mock-server I/O, app state, interaction, logger, position hold, formatting, property tests

## Key Patterns
- The gpsd task sends `TaggedEvent`s (connection generation + `GpsdEvent`: `Connected`, `Message(GpsdMessage)`, `Error`, `Nmea`) over an `mpsc` channel; the app applies each message to its own `GPSData` in place. Events from an older generation (pre-reconnect) are dropped.
- Per-TPV logic (logging, position hold, armed clock sync, TOFF) runs only on a TPV with a new `time`; staleness is measured from `last_tpv`, not any message
- UI rendering is pure: `fn render(f: &mut Frame, area: Rect, app: &App)` — no side effects in `src/ui/`
- Side effects are queued as data and executed by the event loop (`pending_device_cmds`, `pending_clock_sync`); clock sync builds `Cmd`s run through a `CommandRunner` so tests use a mock. Tests must never execute ubxtool/gpsctl/timedatectl/date/sudo
- NaN represented as `f64::NAN`; always check `.is_finite()` before display (formatting helpers return `---`)
- gpsd client runs in a tokio task with auto-reconnect (2s delay), bounded line reads (64 KiB)
- Event loop processes: terminal events (crossterm), gpsd updates (channel, drained before each frame), command output, tick timer (1Hz heartbeat); gpsd-driven redraws are capped at ~20 fps, keys and ticks redraw immediately

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
- `serde` + `toml` — config file
- `chrono` — date/time parsing and formatting
- `clap` — CLI argument parsing
- `futures` — `StreamExt` for crossterm's `EventStream`
- `anyhow` — error handling
- `zbus` — optional, behind the non-default `dbus` feature (stub)
- `open` — browser launching (Google Maps)

## Git Commits
- Do NOT include `Co-Authored-By` lines in commit messages
