# tuigps-rs

Terminal UI GPS monitoring tool using gpsd, written in Rust.

A complete reimplementation of [tuigps](https://github.com/smartin/tuigps) (Python/Textual) in Rust using Ratatui for the TUI and gpsd_client for gpsd communication.

## Features

- **Real-time GPS monitoring** — position, velocity, fix quality, DOP values, error estimates
- **Satellite visualization** — ASCII polar sky plot, SNR bar chart, constellation breakdown, detailed satellite table
- **Precision timing** — PPS offset display, TOFF (time offset) measurement with statistics, armed single-shot TOFF
- **Device configuration** — u-blox receiver management via ubxtool (nav rate, power mode, PPS, constellations, serial speed)
- **Clock synchronization** — set system clock from GPS time or PPS pulse edge (D-Bus, timedatectl, sudo fallback)
- **Position hold** — accumulate fixes with Welford's algorithm, compute CEP50/CEP95 statistics
- **GPS logging** — GPX and CSV format with timestamped filenames
- **NMEA viewer** — raw sentence stream with type filtering, pause/resume, color coding
- **Settings** — configurable host/port, units (metric/imperial/nautical), coordinate format (DD/DMS/DDM)

## Requirements

- Rust 1.75+ (2024 edition)
- Running `gpsd` instance (tested with gpsd 3.25+)
- Linux (for PPS ioctl and clock sync features)

## Building

```bash
cargo build --release
```

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
| `q` | Quit |
| `s` | Open settings |
| `r` | Reconnect to gpsd |
| `u` | Cycle units (metric/imperial/nautical) |
| `m` | Open position in Google Maps (requires fix) |
| `l` | Toggle GPS logging (GPX) |
| `h` | Toggle position hold |
| `Tab` | Next tab |
| `Shift+Tab` | Previous tab |

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
| `k` | Sync system clock from GPS time |

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
| `Up/Down` | Navigate fields |
| `Enter` | Edit text field / cycle value |
| `Left/Right` | Cycle value (Units, Coords) |
| `Esc` | Close settings / cancel edit |
| `Ctrl+S` | Apply settings and reconnect |

## Tabs

1. **Dashboard** — grid layout with position, fix, velocity, sky plot, signal chart, errors, device, time
2. **Satellites** — constellation summary and detailed satellite table (scrollable)
3. **Timing** — PPS/TOFF details, armed measurements, TOFF statistics, clock sync
4. **Device** — u-blox configuration (nav rate, power, PPS, constellations, serial speed)
5. **NMEA** — raw NMEA sentence viewer with filtering, pause, color coding

## License

MIT
