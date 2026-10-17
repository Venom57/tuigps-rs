# tuigps-rs Review & Improvement Plan — 2026-10-17

Static review of the codebase covering runtime correctness/performance, user experience,
unit tests, and CI/CD. Produced by three parallel review passes, with key claims spot-checked
against the source.

> **Caveat:** The original review was static (no toolchain). Tooling verification was run
> afterwards with Rust 1.99.1 — see §0. Line numbers are as of commit `111225b`.

---

## 0. Tooling verification (Rust 1.99.1, 2026-10-17)

Note: toolchain lives in `~/.cargo/bin`; ensure it is on `PATH` in new shells.

| Check | Result |
|---|---|
| `cargo build` | ✅ Builds. **6 warnings**: unused import `ratatui::prelude::*` (`ui/dashboard.rs:1`); unused param `receipt_time` (`gpsd_client.rs:115`); `LogFormat::Csv` never constructed; `HoldResult` fields never read (`position_hold.rs:15+`); `raw_command` never read; `SatelliteInfo::freqid` never read |
| `cargo test` | ❌ **99 passed, 2 failed** (101 total) |
| `cargo clippy --all-targets` | ❌ **Fails** — deny-level `approx_constant` at `formatting.rs:111` (`3.14159` in a test). Plus ~21 warnings: 10× `field_reassign_with_default`, 2× redundant field names, collapsible `if`, items after test module, default-value lint, dead code |
| `cargo fmt --check` | ❌ **54 diffs across 19 files** (18 in `app.rs`, 7 in `gpsd_client.rs`) |

Confirmations / new findings from tooling:
- **B8 confirmed** — `test_timing_clock_sync_no_gps` fails.
- **B9 confirmed** — `cargo test` created `tuigps_YYYYMMDD_HHMMSS.gpx` in the repo root (hidden from `git status` by the `*.gpx` ignore rule). Removed manually after the run.
- **NEW B15** — `test_device_activate` panics: `there is no reactor running` at `app.rs:801` (`tokio::spawn` from a plain `#[test]`). Worse: if it *did* have a runtime it would shell out to the real `ubxtool` and send commands to attached hardware. Fix: inject a command runner / make the device action testable without spawning, and never execute `ubxtool` from tests.
- **B1 corroborated** — `extract_tpv` already takes `receipt_time` but never uses it (compiler warning): the TPV-receipt-time plumbing was started and not finished.
- **U11 corroborated** — `HoldResult` fields (mean/std/duration) are dead code: computed, never displayed.
- **P13 corroborated** — `LogFormat::Csv` is dead.
- **Quick-win batch:** `cargo fmt`, fix `approx_constant` (use `3.14159` → e.g. `1.23456`), resolve the 6 compiler warnings, then turn on `-D warnings` in CI.

---

## 1. Correctness bugs (fix first)

- [ ] **B1. Armed clock sync fires on wrong message / sets wrong time** — `app.rs:381-393`, `clock_sync.rs:14-44`
  - Fires on the next `Update` of *any* class (SKY/PPS/TOFF/DEVICE), not a fresh TPV as the comment claims.
  - `fix_age` uses `last_seen` (updated on every message, `gpsd_client.rs:105`) while `time` comes from the last TPV → clock set late by the TPV→other-message delay.
  - No `has_fix()` / mode ≥ 2 check.
  - `%S` formatting truncates sub-seconds → up to ~1 s error, always toward the past.
  - Fix: fire only when `time` changes (reuse `prev_time` check near `app.rs:339`), track TPV receipt time separately, require mode ≥ 2, pass fractional seconds.
- [ ] **B2. Clock sync blocks the UI and leaves NTP disabled** — `clock_sync.rs:8-10`
  - Up to 3 synchronous `std::process::Command::output()` calls on the event-loop task.
  - `timedatectl set-ntp false` runs unconditionally, error ignored, never re-enabled even if all methods fail.
  - A polkit prompt while in raw mode could hang.
  - Fix: `spawn_blocking`/`tokio::process`, `--no-ask-password`, restore NTP (or report it) on failure.
- [ ] **B3. GPX file invalid after normal quit** — `app.rs:1628`, `gps_logger.rs`
  - `q` breaks the loop without `logger.stop()`; `GpsLogger` has no `Drop` → closing `</trkseg></trk></gpx>` never written.
  - No flush per point (`gps_logger.rs:88-111`) → kill/SIGTERM loses up to 8 KiB of points.
  - GPX header lacks `xmlns` and required `creator` attribute (`gps_logger.rs:65`).
  - CSV writes literal `NaN` for missing values.
  - Fix: stop on exit, `impl Drop`, flush per point, valid header, empty CSV fields for NaN.
- [ ] **B4. Position hold counts the same fix repeatedly** — `app.rs:401-409`
  - Adds a sample on every gpsd update (SKY/PPS/TOFF too). If TPVs stop while PPS continues, stale fix keeps accumulating and CEP shrinks artificially.
  - Fix: gate on TPV `time` changing (as logger does at `gps_logger.rs:92`).
- [ ] **B5. Position hold altitude stats use wrong count** — `position_hold.rs:57-71`
  - Divides by lat/lon count `n` even when altitude samples were skipped (NaN). All-NaN altitude reports 0.0.
  - Fix: separate `count_alt`.
- [ ] **B6. Position hold breaks at antimeridian** — averaging 179.9 and −179.9 gives ~0°. Fix: average longitude as unit vectors or unwrap relative to first fix.
- [ ] **B7. Ctrl+C ignores modifiers** — `app.rs:296`
  - Does not quit; on NMEA tab clears buffer, on Timing tab clears TOFF samples.
  - Fix: handle `Ctrl+C` globally as quit before tab dispatch.
- [ ] **B8. Failing unit test** — `test_timing_clock_sync_no_gps` (`app.rs:1268`) expects "No GPS time" but `k` now sets "Clock sync ARMED…" (`app.rs:659`). Broken by commit `98be9ba`.
- [ ] **B9. Test writes a real file into the repo** — `test_toggle_logging` creates `tuigps_YYYYMMDD_HHMMSS.gpx` in CWD and asserts nothing. Logger dir/filename is hard-coded.
- [ ] **B10. Partial SKY wipes DOPs; empty satellites never clear** — `gpsd_client.rs` (`extract_sky`, ~`:168`)
  - Absent DOP fields set to NaN → flicker with u-blox partial SKY messages. Only overwrite when present.
  - Empty `satellites` array is skipped → stale satellites shown forever.
- [ ] **B11. Stale events overwrite reconnect state** — `app.rs:~1610`
  - After `abort()`, `Update`s still queued in `rx` from the old task overwrite "Reconnecting…" / `connected=false`.
  - Fix: tag events with a connection generation, or drain the channel.
- [ ] **B12. Staleness check defeated by PPS** — `app.rs:433-442`: `last_seen` refreshed by PPS/DEVICE/VERSION, so lost TPVs aren't flagged stale. Track `last_tpv`.
- [ ] **B13. DMS/DDM rounding** — `formatting.rs:36-44` can display `60.000"` instead of carrying.
- [ ] **B15. `test_device_activate` panics (no tokio runtime) and would run real `ubxtool`** — `app.rs:801`. See §0.
- [ ] **B14. `nmea_type` byte slicing** — `nmea_viewer.rs:96` slices `[3..6]`, panics on multibyte input; `$PUBX` etc. typed as garbage.

## 2. Runtime / performance

- [ ] **P1. Redraw on every select! arm** — `app.rs:1626`
  - Every NMEA line, JSON message, keypress, and command-output line triggers a full frame. ~20–30 fps at 1 Hz with NMEA on; 200+ at 10 Hz nav rate.
  - Fix: dirty flag, drain `rx` with `try_recv` before draw, cap at ~10–20 fps.
- [ ] **P2. Full `GPSData` deep clone per JSON line** — `gpsd_client.rs:75` (satellite `Vec` + strings). Bounded(100) channel means slow draws (ssh) back-pressure into the TCP socket; gpsd may drop slow clients.
- [ ] **P3. Per-class events from gpsd client (architectural fix)** — send `Tpv/Sky/Pps/Toff/Device…` events (or deltas / `Arc` snapshots) instead of whole snapshots. Fixes B1, B4, B11, B12, P1, P2 together.
- [ ] **P4. Per-frame allocation churn in UI**
  - Sky plot allocates a `String`/`Span` per cell (`sky_plot.rs:142-153`) → use `buf.set_string`/cells directly.
  - NMEA viewer re-filters and clones up to 1000 lines per frame (`nmea_viewer.rs:29-39,73`).
  - Satellite sort and `constellation_counts` HashMap rebuilt every frame.
- [ ] **P5. Unbounded / fragile line reading** — `gpsd_client.rs:59` `read_line` grows without limit on newline-less input; invalid UTF-8 forces disconnect + 2 s reconnect. Use `read_until` with size cap + `from_utf8_lossy`.
- [ ] **P6. Unknown/invalid JSON still sends a clone and triggers redraw** — `gpsd_client.rs:74-76,90`.
- [ ] **P7. Silently swallowed errors** — logger start/write (`app.rs:397,483,488`), `open::that` (`:476`), crossterm `Err`/`None` (`:1587`). If the event stream ends, `q` can never be received. Surface via `set_status`.
- [ ] **P8. Dependency trimming** (`Cargo.toml`)
  - Unused: `nix`, `thiserror`, `tracing`, `tracing-subscriber`.
  - Unused features: `serde` derive (only `serde_json::Value` used), chrono `serde`.
  - `zbus` is a *default* feature but `try_dbus_set_time` is a TODO stub (`clock_sync.rs:59-65`) — make non-default or implement.
  - tokio `"full"` → `rt-multi-thread, macros, net, io-util, time, sync, process`.
- [ ] **P9. Release profile** — add `[profile.release]` with `lto = "thin"`, `codegen-units = 1`, `strip = true` (consider `panic = "abort"` only after verifying teardown doesn't rely on unwinding).
- [ ] **P10. Terminal restore gaps** — `main.rs:37-58`: failure between `enable_raw_mode` and `Terminal::new` skips restore; panic in spawned task restores terminal but app keeps running on a broken screen. Consider `ratatui::init()`/`restore()`.
- [ ] **P11. Log filename collisions** — 1 s resolution + `File::create` truncates (`gps_logger.rs:58-63`). Toggling twice within a second overwrites.
- [ ] **P12. Device output log `Vec` grows forever** — cap it.
- [ ] **P13. Dead code** — `armed_toff: Arc<AtomicBool>` (`app.rs:226`) only used on one task → plain `bool`; `LogFormat::Csv` unreachable (`toggle_logging` hardcodes GPX).

## 3. User experience

### High
- [ ] **U1. Connection badge truncated off footer** — `status_bar.rs:13-27,116-150`. ~65 cols of global hints render first; at 80 cols on NMEA/Device tabs, CONNECTED/DISCONNECTED/STALE never appears. Fix: right-anchored chunk for status + REC/HOLD; move long hints to a `?` help overlay.
- [ ] **U2. Stale values shown at full brightness when disconnected** — `GpsdEvent::Error` (`app.rs:411-414`) and reconnect only clear `connected`. Dim panels / show "NO DATA" when disconnected or stale; show fix age.
- [ ] **U3. Settings help text invisible** — `settings.rs:12` background `DarkGray`; hints/descriptions also `DarkGray` (`:79-91,130,154-155`). Negates the last commit's work.
- [ ] **U4. Dangerous device actions without confirmation** — `app.rs:697,741-789`
  - Enter immediately runs ubxtool (baud change can sever link, disable GNSS, save to flash).
  - Controls initialize from hard-coded defaults (`app.rs:176-188`: 9600 baud, all constellations `[x]`), not the receiver's actual config.
  - `[x]`/`[ ]` flip before apply → pending vs applied indistinguishable.
  - Fix: query current config, pending marker, two-step confirm for destructive ops.
- [ ] **U5. Mouse capture enabled but unused** — `main.rs:46`. Blocks terminal text selection (can't copy coordinates). Remove, or implement click-to-tab + wheel scroll.

### Medium
- [ ] **U6. Dashboard clips data at 80×24 with no indication** — fixed 3/5/2 ratios (`dashboard.rs:8-12`). Fix panel loses PDOP/GDOP; Position loses MSL/geoid/hold CEP (`position.rs:36-53`); Errors shows 2 of 7 rows. Use `Length`/`Min` constraints + "terminal too small" notice.
- [ ] **U7. Misleading placeholders** — PPS offset `+0.0 ns` with no PPS (`time_panel.rs:35`, `data_model.rs:278`); device cycle `0.0s`/`NaNs` (`device_panel.rs:14`); leap seconds `0`; satellite health 0 (unknown) shown as red "0" (`satellite_table.rs:44-53`).
- [ ] **U8. Units only partially applied** — climb always m/s (`velocity.rs:30`); geoid, CEP, error estimates always meters (`position.rs:33,43,49`, `error_panel.rs`).
- [ ] **U9. Settings not persisted** — add `~/.config/tuigps/config.toml` (host, port, units, coord format, theme).
- [ ] **U10. Unbounded scroll offsets** — satellite Down / NMEA Up (`app.rs:620-631,669-677`) count past end; render clamps, so user must press the opposite key many times. NMEA offset is bottom-relative, so view drifts as lines arrive while scrolled.
- [ ] **U11. Missing action feedback** — `l` logging gives no start/stop/error message, filename never shown (`app.rs:480-492`); `m` silently does nothing without fix (`:470-478`); `h` off silently discards hold stats — mean/count/duration/std computed but never displayed (`:494-500`).
- [ ] **U12. Color accessibility** — GPS green vs BeiDou red (`constants.rs:28-31`); sky plot distinguishes constellations by color only, no legend; red/yellow/green SNR/DOP/CEP; Galileo dark blue hard to read on black; `Color::White` values vanish on light themes; no `NO_COLOR` / monochrome; STALE badge white-on-yellow (`status_bar.rs:126`).

### Low
- [ ] **U13.** Settings: Left and Right both cycle forward (`app.rs:574-580`); Port accepts letters, invalid value silently ignored; Esc after editing host keeps new value without reconnect.
- [ ] **U14.** Status message duplicated on Timing tab (`mod.rs:124-130`).
- [ ] **U15.** Satellite table subtracts 4 rows for chrome but uses 3 (`satellite_table.rs:13`); sort fixed to GNSS/PRN.
- [ ] **U16.** No `1`–`5` tab shortcuts, no `?` help, Esc does nothing in main view.

### Feature ideas
- `?` help overlay + compact footer.
- Fix age / last-fix timestamp; grey out stale panels.
- Copy position to clipboard (OSC 52); MGRS/UTM coordinate formats.
- Position-hold panel: mean position, count, duration, live CEP circle/scatter, export.
- Sortable satellite table (SNR / elevation / used); sky-plot constellation glyphs + legend.
- SNR and satellite-count sparklines; DOP/accuracy trend.
- NMEA viewer: free-text search, talker filter, save to file.
- `--no-color` / `NO_COLOR`, light theme.
- Mouse: click tabs, scroll lists.

## 4. Tests

### Current state (~101 unit tests, all in `src/`; no `tests/`, no `lib.rs`)

| Module | Tests | Gaps |
|---|---|---|
| `app.rs` | 40 | Device-config UI, clock arm flow, maps key, tick/stale, gpsd event ingestion |
| `formatting.rs` | 23 | Mostly `contains()` not exact strings; no rounding-boundary cases |
| `constants.rs` | 14 | Negative / >360° bearings |
| `data_model.rs` | 10 | — |
| `gpsd_client.rs` | 9 | TOFF, DEVICES, WATCH, ERROR; partial/empty SKY; wrong-type fields; reconnect loop |
| `position_hold.rs` | 5 | NaN altitude, antimeridian, known-variance check |
| `gps_logger.rs`, `clock_sync.rs`, `main.rs`, all `ui/*` | **0** | Everything |

### Plan
- [ ] **T0.** Split crate into `src/lib.rs` + thin `src/main.rs` so `tests/` can import modules.
- [ ] **T1.** Fix B8 (failing test) and B9 (test writes file; make logger output dir injectable, use `tempfile`).
- [ ] **T2. Render smoke tests** with `ratatui::backend::TestBackend`: `ui::render` and each panel at 0×0, 1×1, 10×3, 20×5, 40×12, 80×24, 200×60 with default and fully-populated `GPSData` (NaNs, 0 sats, 64 sats, SNR > 55, NMEA overflow). Then `insta` snapshots at 80×24.
- [ ] **T3. gpsd fixtures** `tests/fixtures/*.jsonl` (real gpsd 3.2x): TPV, SKY (full/partial/empty), TOFF, PPS, DEVICES (0/1/2), WATCH, VERSION, ERROR, plus malformed (truncated, wrong types, huge PRN, 1 MB line, CRLF). Assert exact field values.
- [ ] **T4. Mock gpsd server** (`#[tokio::test]`): `TcpListener` on `127.0.0.1:0`, verify `?WATCH`, stream fixtures, close → expect `Error` → reconnect after 2 s (`tokio::time::pause()`) → clean shutdown.
- [ ] **T5. Logger output**: parse GPX with `quick-xml` after stop *and* drop; CSV via `csv` crate (header + N rows, empty fields not `NaN`, duplicate timestamps deduped).
- [ ] **T6. position_hold numerics**: two-pass reference for mean/std, B5/B6 regressions, CEP50 vs synthetic Gaussian.
- [ ] **T7. `proptest`**: formatting never panics on any f64; `process_message` never panics on arbitrary input; renders never panic for any `Rect` < 300×100.
- [ ] **T8. App state**: feed `GpsdEvent::Update`/`Nmea` into `App`, check staleness, NMEA buffer cap, hold accumulation, clock-sync arm flow, Ctrl+C.
- [ ] Add `[dev-dependencies]`: `insta`, `proptest`, `tempfile`, `quick-xml`, `csv`.

## 5. CI/CD & tooling

No CI exists (no `.github/`, no `.gitlab-ci.yml`). This is how B8 shipped.

- [ ] **C1. `.github/workflows/ci.yml`**

```yaml
name: CI
on: [push, pull_request]
env: { CARGO_TERM_COLOR: always, RUSTFLAGS: -Dwarnings }
jobs:
  check:
    runs-on: ubuntu-latest
    strategy:
      matrix: { features: ["", "--no-default-features"] }
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: "rustfmt, clippy" }
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all --check
      - run: cargo clippy --all-targets ${{ matrix.features }} -- -D warnings
      - run: cargo test --all-targets ${{ matrix.features }}
      - run: cargo build --release ${{ matrix.features }}
  msrv:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.88
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --all-targets --locked
  deny:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: EmbarkStudios/cargo-deny-action@v2
```

- [ ] **C2. `.github/workflows/release.yml`** — tag-triggered `cross` builds (Raspberry Pi is a likely target):

```yaml
name: Release
on: { push: { tags: ["v*"] } }
permissions: { contents: write }
jobs:
  build:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        target: [x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu,
                 armv7-unknown-linux-gnueabihf, x86_64-unknown-linux-musl]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with: { key: "${{ matrix.target }}" }
      - run: cargo install cross --locked
      - run: cross build --release --locked --target ${{ matrix.target }}
      - run: tar czf tuigps-${{ github.ref_name }}-${{ matrix.target }}.tar.gz -C target/${{ matrix.target }}/release tuigps-rs
      - uses: softprops/action-gh-release@v2
        with: { files: "*.tar.gz" }
```
  Alternatives: `cargo-dist`; optional `cargo-deb` for Pi users.

- [ ] **C3. Cargo.toml metadata** — `rust-version = "1.88"`, `description`, `license`, `repository`, `readme`, `keywords`, `categories`.
- [ ] **C4.** `rust-toolchain.toml` (stable + rustfmt + clippy).
- [ ] **C5.** `[lints]` table (e.g. `clippy::pedantic` warn, `unwrap_used` warn outside tests).
- [ ] **C6.** `deny.toml` (`cargo deny init`, allow-list licenses).
- [ ] **C7.** `.github/dependabot.yml` for `cargo` + `github-actions`.
- [ ] **C8.** Optional pre-commit hook (`fmt --check` + `clippy`).
- [ ] **C9.** `.gitignore`: `*.csv` would hide CSV test fixtures — add `!tests/fixtures/**`; misleading "Cargo.lock optional" comment (lockfile should stay committed for a binary).

## 6. Documentation drift

- [ ] **D1.** README and CLAUDE.md say "Rust 1.75+". Edition 2024 needs 1.85; let-chains (e.g. `gpsd_client.rs:206`) need **1.88**.
- [ ] **D2.** README advertises CSV logging — unreachable (GPX hardcoded, `app.rs:487`).
- [ ] **D3.** README claims clock sync "from GPS time or PPS pulse edge (D-Bus, …)" — no PPS-edge path; D-Bus is a stub.
- [ ] **D4.** README documents `k` as "Sync system clock" — it toggles arm/disarm.
- [ ] **D5.** Left/Right "cycle value" wording hides that both go forward; Tab/Shift+Tab in settings undocumented; settings implied persistent but aren't.

## 7. Architecture

- [ ] **A1. Split `app.rs`** (1636 lines, ~600 tests) into:
  - `app/device_config.rs` — `DeviceConfigState`, constants, `run_ubxtool`, `run_baud_rate_change` (duplicated output handling), `build_tp5_cmd` (~300 lines)
  - `app/settings.rs` — `SettingsField` + input handler
  - `app/nmea.rs` — buffers, pause/filter
  - `app/timing.rs` — TOFF + clock arming
  - `event_loop.rs` — `run`
  - Tests move with their modules.
- [ ] **A2.** Move app-managed `toff_*` fields out of `GPSData` (`data_model.rs:200-203`) into an App-owned `TimingState`; removes the take/restore workaround at `app.rs:319-332`.
- [ ] **A3.** Per-class gpsd events (see P3).

---

## Execution order

1. ~~**Verify with real tooling**~~ — done, see §0.
2. **Green baseline + CI** — `cargo fmt`, fix clippy `approx_constant` + warnings, fix B8, B9, B15, then add C1 with `-D warnings`.
3. **Safety & data integrity** — B1, B2, B3, B7.
4. **Event model refactor** — P3/A3 (fixes B4, B11, B12, P1, P2), then frame cap.
5. **UX quick wins** — U1, U3, U5, U2.
6. **Test infrastructure** — T0, T2, T3, then A1 split.
7. **Cleanup** — P8, P9, D1–D5, remaining Medium/Low items.
