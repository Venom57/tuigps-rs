# tuigps-rs

Terminal UI GPS monitoring tool using gpsd, written in Rust.

A complete reimplementation of [tuigps](https://github.com/smartin/tuigps) (Python/Textual) in Rust using Ratatui for the TUI and a small built-in client for the gpsd JSON protocol.

## Features

- **Real-time GPS monitoring** — position, velocity, fix quality, DOP values, error estimates
- **Satellite visualization** — ASCII polar sky plot, SNR bar chart, constellation breakdown, detailed satellite table
- **Precision timing** — PPS offset display, TOFF (time offset) measurement with statistics, armed single-shot TOFF
- **Device configuration** — u-blox receiver management via ubxtool (nav rate, power mode, PPS, constellations, serial speed)
- **Clock synchronization** — arm a one-shot set of the system clock from GPS time on the next fix (timedatectl, falling back to `sudo -n date`)
- **Position hold** — accumulate fixes with Welford's algorithm, compute CEP50/CEP95 statistics
- **GPS logging** — GPX track to a timestamped file in the working directory (a CSV writer exists in code but is not reachable from the UI)
- **NMEA viewer** — raw sentence stream with type filtering, pause/resume, color coding
- **Settings** — host/port, units (metric/imperial/nautical), coordinate format (DD/DMS/DDM); not persisted between runs

## Requirements

- Rust 1.88+ (2024 edition)
- Running `gpsd` instance (tested with gpsd 3.25+)
- Linux (clock sync uses `timedatectl` / `sudo date`; device config uses `ubxtool` and `gpsctl` from gpsd)

## Building

```bash
cargo build --release
```

### Cargo features

| Feature | Default | Description |
|---------|---------|-------------|
| `dbus`  | off     | D-Bus (`zbus`) clock sync. **Stub only** — not implemented yet; clock sync always falls back to `timedatectl` / `sudo date`. |

## Running

```bash
# Connect to local gpsd (default localhost:2947)
./target/release/tuigps-rs

# Connect to remote gpsd
./target/release/tuigps-rs --host 192.168.1.100 --port 2947
```

## Testing with simulated GPS

```bash
gpsfake -c 0.5 /usr/share/gpsd/sample.nmea
```

## Keyboard Shortcuts

### Global

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Quit |
| `s` | Open settings |
| `r` | Reconnect to gpsd |
| `u` | Cycle units (metric/imperial/nautical) |
| `m` | Open position in Google Maps (requires fix) |
| `l` | Toggle GPS logging (GPX) |
| `h` | Toggle position hold |
| `Tab` | Next tab |
| `Shift+Tab` | Previous tab |

Logging writes `tuigps_YYYYMMDD_HHMMSS.gpx` to the current directory.

### Satellites Tab

| Key | Action |
|-----|--------|
| `Up/Down` | Scroll satellite table |
| `PageUp/PageDown` | Scroll satellite table (fast) |

### Timing Tab

| Key | Action |
|-----|--------|
| `a` | Arm single-shot TOFF capture |
| `c` | Clear TOFF samples and armed data |
| `k` | Arm / disarm a one-shot clock sync on the next GPS fix |

When an armed sync fires it runs `timedatectl set-ntp false`, then tries
`timedatectl set-time`, then `sudo -n date -u -s`. There is no PPS-edge
sync path. The D-Bus method is a stub behind the non-default `dbus` feature.

### Device Tab

| Key | Action |
|-----|--------|
| `Up/Down` | Select control |
| `Left/Right` | Adjust value / toggle |
| `Enter` | Apply selected control |

### NMEA Tab

| Key | Action |
|-----|--------|
| `p` | Pause/resume NMEA stream |
| `f` | Cycle sentence filter (ALL/GGA/RMC/GSA/GSV/VTG/GLL/ZDA) |
| `c` | Clear NMEA buffer |
| `Up/Down` | Scroll |
| `PageUp/PageDown` | Scroll (fast) |

### Settings Overlay

| Key | Action |
|-----|--------|
| `Up/Down`, `Tab/Shift+Tab` | Move between fields |
| `Enter` | Edit Host/Port (Enter again to accept) / cycle Units, Coords |
| `Left` or `Right` | Cycle Units / Coords forward (both keys go forward) |
| `Esc` | Cancel edit / close settings |
| `Ctrl+S` | Close settings and reconnect |

Changes take effect immediately (host/port on the next reconnect) and are
not saved; restarting resets them to the CLI defaults.

## Tabs

1. **Dashboard** — grid layout with position, fix, velocity, sky plot, signal chart, errors, device, time
2. **Satellites** — constellation summary and detailed satellite table (scrollable)
3. **Timing** — PPS/TOFF details, armed measurements, TOFF statistics, clock sync
4. **Device** — u-blox configuration (nav rate, power, PPS, constellations, serial speed)
5. **NMEA** — raw NMEA sentence viewer with filtering, pause, color coding

## Development

```bash
cargo fmt --all            # format
cargo clippy --all-targets # lint (CI runs with -D warnings)
cargo test                 # unit + integration tests
cargo deny check           # licenses / advisories (needs cargo-deny)
```

The crate is a library (`src/lib.rs`) plus a thin binary (`src/main.rs`) so
integration tests under `tests/` can use the modules directly.

CI (`.github/workflows/ci.yml`) runs fmt, clippy, tests and a release build
for default / no-default / all features, an MSRV (1.88) check and
`cargo-deny`. Pushing a `v*` tag runs `.github/workflows/release.yml`, which
cross-compiles Linux x86_64 (gnu, musl), aarch64 and armv7 tarballs with
SHA-256 checksums and attaches them to a GitHub release.

## License

MIT
