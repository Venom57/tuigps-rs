# tuigps-rs

Terminal UI GPS monitoring tool using gpsd, written in Rust.

A complete reimplementation of [tuigps](https://github.com/smartin/tuigps) (Python/Textual) in Rust using Ratatui for the TUI and a small built-in client for the gpsd JSON protocol.

## Features

- **Real-time GPS monitoring** — position, velocity, fix quality, DOP values, error estimates
- **Satellite visualization** — ASCII polar sky plot, SNR bar chart, constellation breakdown, detailed satellite table
- **Precision timing** — PPS offset display, TOFF (time offset) measurement with statistics, armed single-shot TOFF
- **Device configuration** — u-blox receiver management via ubxtool (nav rate, power mode, PPS, constellations, serial speed)
- **Clock synchronization** — arm a one-shot set of the system clock from GPS time on the next fix (timedatectl, falling back to `sudo -n date`)
- **Position hold** — accumulate fixes with Welford's algorithm; a Hold panel shows mean position, sample count, duration, σN/σE and CEP50/CEP95 (exact bivariate-normal radius)
- **GPS logging** — GPX 1.1 track, CSV (one row per fix), or both side by side, to timestamped files in a configurable directory; flushed per point, finalized even on abnormal exit, existing files are never overwritten
- **NMEA viewer** — raw sentence stream with type filtering, pause/resume, scroll-back that stays anchored, color coding
- **Settings** — host/port, units (metric/imperial/nautical), coordinate format (DD/DMS/DDM), saved to a config file
- **Status at a glance** — connection badge always visible, fix age, panels marked NO DATA / STALE when data stops, `?` help overlay
- **Accessibility** — constellation letters on the sky plot (not color alone), default-foreground text for light themes, [`NO_COLOR`](https://no-color.org) support

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

# Use a specific config file
./target/release/tuigps-rs --config ./my-config.toml
```

### Configuration

Settings are loaded from `$XDG_CONFIG_HOME/tuigps-rs/config.toml`
(default `~/.config/tuigps-rs/config.toml`) and written when you apply
settings (`Ctrl+S` or `[ Save ]`). A missing or invalid file falls back to
defaults with a status message. `--host` / `--port` override the file.

```toml
host = "localhost"
port = 2947
units = "metric"        # metric | imperial | nautical
coord_format = "dd"     # dd | dms | ddm
log_format = "gpx"      # gpx | csv | both
log_dir = "."           # existing directory; "~/" = home
```

`--log-format <gpx|csv|both>` and `--log-dir <DIR>` override the file too.

### Logging

`l` starts and stops a logging session. Files are named
`tuigps_YYYYMMDD_HHMMSS.gpx` / `.csv` (with `both`, the two files share the
name); a `_1`, `_2`, … suffix is added rather than overwrite anything. A point
is written once per new TPV with a 2D/3D fix. The footer shows
`REC GPX+CSV 42 pts` while logging. Format and directory changes made during
a session apply to the next one.

CSV columns (always SI units, regardless of the display unit setting; fields
gpsd did not report are left empty, never `NaN`):

| Column | Meaning |
|---|---|
| `time` | gpsd TPV time, ISO 8601 UTC |
| `latitude`, `longitude` | WGS84 decimal degrees |
| `alt_msl` | altitude above mean sea level, m |
| `speed` | ground speed, m/s |
| `track` | course over ground, ° true |
| `climb` | vertical speed, m/s |
| `alt_hae` | altitude above the WGS84 ellipsoid, m |
| `mode` | `2d` or `3d` |
| `status` | `gps`, `dgps`, `rtk_fix`, `rtk_float`, `dr`, `gnss_dr`, `time_only`, `simulated`, `pps` (empty if unknown) |
| `sats_used`, `sats_visible` | satellite counts |
| `hdop`, `vdop`, `pdop` | dilution of precision |
| `eph`, `epv` | estimated horizontal / vertical error, m |

New columns are only ever appended, so scripts reading the leading columns
keep working.

### Units

| | metric | imperial | nautical |
|---|---|---|---|
| Speed, EPS | km/h | mph | kn |
| Climb, EPC | m/s | ft/min | ft/min |
| Altitude, geoid, EPH/EPV/EPX/EPY/SEP, CEP, σ | m | ft | ft |

`u` cycles units for the current session only; use Settings to save them.

Set `NO_COLOR=1` to disable colors (badges switch to reverse video).

## Testing with simulated GPS

```bash
gpsfake -c 0.5 /usr/share/gpsd/sample.nmea
```

## Keyboard Shortcuts

Press `?` at any time for a help overlay generated from the same key table the
app uses, so it always matches the current tab.

### Global

| Key | Action |
|-----|--------|
| `?` | Toggle help |
| `1`–`5` | Jump to tab |
| `Tab` / `Shift+Tab` | Next / previous tab |
| `q` / `Ctrl+C` | Quit (`Ctrl+C` works everywhere, including text input) |
| `Esc` | Close overlay / dismiss prompt |
| `r` | Reconnect to gpsd |
| `s` | Settings |
| `u` | Cycle units (not saved) |
| `m` | Open position in Google Maps (requires fix) |
| `l` | Start/stop logging (format and directory from Settings; see [Logging](#logging)) |
| `h` | Start/stop position hold |

### Satellites Tab

| Key | Action |
|-----|--------|
| `↑/↓` | Scroll one line |
| `PgUp/PgDn` | Scroll one page |
| `Home/End` | First / last satellite |

### Timing Tab

| Key | Action |
|-----|--------|
| `a` | Arm one-shot TOFF capture |
| `c` | Clear TOFF data |
| `k` | Arm / disarm a one-shot clock sync on the next fix |

An armed clock sync fires only on a fresh TPV with a 2D/3D fix, compensates
for the fix age (refusing fixes older than 60 s) and keeps sub-second
precision. It runs off the UI thread: `timedatectl set-ntp false`, then
`timedatectl set-time`, then `sudo -n date -u -s` (all non-interactive). If
every method fails, NTP is re-enabled. There is no PPS-edge sync path; the
D-Bus method is a stub behind the non-default `dbus` feature.

### Device Tab

| Key | Action |
|-----|--------|
| `↑/↓` | Select control |
| `←/→` | Change value (`*` = pending, `✓` = applied) |
| `Enter` | Apply — asks for confirmation first |
| `y` / other key | Confirm / cancel |

Baud-rate changes, disabling a constellation and saving to flash show a
warning before confirming. "Applied" means the command was sent; the
receiver's actual configuration is not read back.

### NMEA Tab

| Key | Action |
|-----|--------|
| `p` | Pause / resume the stream |
| `f` | Cycle sentence filter |
| `c` | Clear buffer |
| `↑/↓`, `PgUp/PgDn` | Scroll back / forward |
| `Home/End` | Oldest / follow newest |

The title shows FOLLOWING, SCROLLED +N or PAUSED.

### Settings Overlay

| Key | Action |
|-----|--------|
| `↑/↓`, `Tab/Shift+Tab` | Move between fields |
| `Enter` | Edit host/port/log dir, cycle units/coords/log format, or activate `[ Save ]` |
| `←/→` | Cycle value backward / forward |
| `Ctrl+S` | Apply and save (reconnects if host/port changed) |
| `Esc` | Cancel the current edit; press again to discard changes and close |

The port accepts digits only (1–65535). The log directory must already exist.

## Tabs

1. **Dashboard** — grid layout with position (incl. fix age), fix, velocity (or Hold stats while position hold is on), sky plot, signal chart, errors, device, time
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

Optional pre-commit hook (fmt + clippy): `git config core.hooksPath .githooks`.

The crate is a library (`src/lib.rs`) plus a thin binary (`src/main.rs`) so
integration tests under `tests/` can use the modules directly.

CI (`.github/workflows/ci.yml`) runs fmt, clippy, tests and a release build
for default / no-default / all features, an MSRV (1.88) check and
`cargo-deny`. Pushing a `v*` tag runs `.github/workflows/release.yml`, which
cross-compiles Linux x86_64 (gnu, musl), aarch64 and armv7 tarballs with
SHA-256 checksums and attaches them to a GitHub release.

## License

MIT
