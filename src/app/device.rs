//! Device tab: u-blox controls, y/n confirmation and command execution
//! (`ubxtool` / `gpsctl`).

use crossterm::event::{KeyCode, KeyModifiers};
use std::process::Output;
use std::time::Duration;

use super::App;

/// Maximum number of lines kept in the Device tab output log.
pub const OUTPUT_LOG_MAX: usize = 500;

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

impl App {
    /// Move the Device tab selection up or down, stopping at the ends.
    pub(super) fn device_config_select(&mut self, up: bool) {
        let dc = &mut self.device_config;
        if up {
            dc.selected_control = dc.selected_control.saturating_sub(1);
        } else if dc.selected_control < DEVICE_CONTROL_COUNT - 1 {
            dc.selected_control += 1;
        }
    }

    pub(super) fn device_config_adjust(&mut self, dir: i32) {
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
    pub(super) fn device_config_activate(&mut self) {
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
    pub(super) fn handle_device_confirm(&mut self, key: crossterm::event::KeyEvent) {
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
    pub(super) fn dispatch_device_commands(&mut self) {
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

    fn run_ubxtool(&self, args: &[String], proto_ver: &str) {
        let tx = self.cmd_tx.clone();
        let full_args: Vec<String> = ["-P".to_string(), proto_ver.to_string()]
            .into_iter()
            .chain(args.iter().cloned())
            .collect();
        let cmd_str = format!("$ ubxtool {}", full_args.join(" "));
        let _ = tx.try_send(cmd_str);

        tokio::spawn(async move {
            let msg = match run_command("ubxtool", &full_args).await {
                Ok(output) => {
                    let text = combined_output(&output);
                    if text.is_empty() {
                        "(no output)".to_string()
                    } else {
                        text
                    }
                }
                Err(CmdError::Spawn(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    "Error: ubxtool not found (install gpsd-clients)".to_string()
                }
                Err(CmdError::Spawn(e)) => format!("Error: {}", e),
                Err(CmdError::Timeout) => "Error: command timed out (10s)".to_string(),
            };
            let _ = tx.send(msg).await;
        });
    }

    fn run_baud_rate_change(&self, speed: u32, device: &str, proto_ver: &str) {
        let tx = self.cmd_tx.clone();
        let proto = proto_ver.to_string();
        let device = device.to_string();
        let speed_str = speed.to_string();

        tokio::spawn(async move {
            // Step 1: Tell receiver to switch baud rate
            let ubx_args = ["-P".to_string(), proto, "-S".to_string(), speed_str.clone()];
            match run_command("ubxtool", &ubx_args).await {
                Ok(output) => {
                    let text = combined_output(&output);
                    if !text.is_empty() {
                        let _ = tx.send(text).await;
                    }
                }
                Err(CmdError::Spawn(e)) => {
                    let _ = tx.send(format!("ubxtool error: {}", e)).await;
                    return;
                }
                Err(CmdError::Timeout) => {
                    let _ = tx.send("ubxtool timed out (10s)".to_string()).await;
                    return;
                }
            }

            // Step 2: Tell gpsd the new speed
            let mut gpsctl_args = vec!["-s".to_string(), speed_str.clone()];
            if !device.is_empty() {
                gpsctl_args.push(device.clone());
            }
            let msg = match run_command("gpsctl", &gpsctl_args).await {
                Ok(output) if output.status.success() => {
                    let dev_name = if device.is_empty() {
                        "default device"
                    } else {
                        &device
                    };
                    format!("Baud rate set to {} on {}", speed_str, dev_name)
                }
                Ok(output) => format!(
                    "gpsctl error: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
                Err(CmdError::Spawn(e)) => format!("gpsctl error: {}", e),
                Err(CmdError::Timeout) => "gpsctl timed out (10s)".to_string(),
            };
            let _ = tx.send(msg).await;
        });
    }
}

/// Time limit for each external device command.
const DEVICE_CMD_TIMEOUT: Duration = Duration::from_secs(10);

/// Why an external device command produced no output.
enum CmdError {
    /// The process could not be started (e.g. not installed).
    Spawn(std::io::Error),
    /// The process did not finish within [`DEVICE_CMD_TIMEOUT`].
    Timeout,
}

/// Run `program` to completion, capturing its output, within
/// [`DEVICE_CMD_TIMEOUT`].
async fn run_command(program: &str, args: &[String]) -> Result<Output, CmdError> {
    match tokio::time::timeout(
        DEVICE_CMD_TIMEOUT,
        // kill_on_drop: if the timeout fires, dropping the future kills the
        // child instead of leaving it running against the receiver
        tokio::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(CmdError::Spawn(e)),
        Err(_) => Err(CmdError::Timeout),
    }
}

/// Trimmed stdout and stderr, joined by a newline when both are non-empty.
fn combined_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    [stdout.trim(), stderr.trim()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ActiveTab;
    use crate::app::test_util::*;

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

    #[cfg(unix)]
    #[test]
    fn test_combined_output_joins_trimmed_streams() {
        use std::os::unix::process::ExitStatusExt;
        let output = |stdout: &str, stderr: &str| Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        };
        assert_eq!(combined_output(&output(" out \n", "\nerr\n")), "out\nerr");
        assert_eq!(combined_output(&output("out\n", "  ")), "out");
        assert_eq!(combined_output(&output("", "err")), "err");
        assert_eq!(combined_output(&output("\n", "")), "");
    }
}
