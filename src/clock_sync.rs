//! System clock sync from a GPS fix.
//!
//! The command sequence is built as data ([`Cmd`]) and executed through a
//! [`CommandRunner`], so the logic can be unit-tested with a mock runner
//! without ever touching the system clock. The real runner is only used from
//! [`sync_clock_async`], which runs it on a blocking thread so the UI event
//! loop never waits on a subprocess.

use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use std::fmt;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// Everything needed to set the clock from a fix.
#[derive(Debug, Clone, PartialEq)]
pub struct ClockSyncRequest {
    /// RFC 3339 time of the fix, as reported in the TPV.
    pub gps_time: String,
    /// Local unix time at which that TPV was received.
    pub tpv_receipt: f64,
}

/// One external command, as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub program: &'static str,
    pub args: Vec<String>,
}

impl Cmd {
    fn new(program: &'static str, args: &[&str]) -> Self {
        Self {
            program,
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl fmt::Display for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.program, self.args.join(" "))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdOutput {
    pub success: bool,
    pub stderr: String,
}

/// Executes a [`Cmd`]. Tests use a mock; production uses [`SystemRunner`].
pub trait CommandRunner {
    fn run(&mut self, cmd: &Cmd) -> std::io::Result<CmdOutput>;
}

/// Runs commands for real, with stdin detached so nothing can prompt.
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&mut self, cmd: &Cmd) -> std::io::Result<CmdOutput> {
        let output = Command::new(cmd.program)
            .args(&cmd.args)
            .stdin(Stdio::null())
            .output()?;
        Ok(CmdOutput {
            success: output.status.success(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

/// `timedatectl set-ntp <enable>` without any interactive polkit prompt.
pub fn ntp_cmd(enable: bool) -> Cmd {
    let value = if enable { "true" } else { "false" };
    Cmd::new("timedatectl", &["--no-ask-password", "set-ntp", value])
}

/// `timedatectl set-time` takes local time; fractional seconds are kept.
pub fn timedatectl_set_time_cmd(target: DateTime<Utc>) -> Cmd {
    let local = target.with_timezone(&Local);
    let arg = local.format("%Y-%m-%d %H:%M:%S%.6f").to_string();
    Cmd::new("timedatectl", &["--no-ask-password", "set-time", &arg])
}

/// `sudo -n date -u -s` takes UTC; fractional seconds are kept.
pub fn sudo_date_cmd(target: DateTime<Utc>) -> Cmd {
    let arg = target.format("%Y-%m-%d %H:%M:%S%.6f").to_string();
    Cmd::new("sudo", &["-n", "date", "-u", "-s", &arg])
}

/// GPS fix time advanced by the age of the fix (`now - tpv_receipt`).
/// Returns the target time and the fix age in seconds.
pub fn target_time(gps_time: &str, tpv_receipt: f64, now: f64) -> Result<(DateTime<Utc>, f64)> {
    let gps = DateTime::parse_from_rfc3339(gps_time)?.with_timezone(&Utc);
    let fix_age = now - tpv_receipt;
    if !fix_age.is_finite() || !(0.0..60.0).contains(&fix_age) {
        anyhow::bail!("fix age {fix_age:.3}s out of range; refusing to set clock");
    }
    let adjusted = gps + chrono::Duration::microseconds((fix_age * 1e6).round() as i64);
    Ok((adjusted, fix_age))
}

pub fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn describe(result: &std::io::Result<CmdOutput>) -> String {
    match result {
        Ok(out) if out.stderr.is_empty() => "failed".to_string(),
        Ok(out) => out.stderr.clone(),
        Err(e) => e.to_string(),
    }
}

fn succeeded(result: &std::io::Result<CmdOutput>) -> bool {
    matches!(result, Ok(out) if out.success)
}

/// Set the system clock from `req`, trying each method in turn.
///
/// `now` is sampled right before each set-time attempt so the fix-age
/// compensation includes time spent in earlier attempts.
/// If NTP was disabled but no method set the clock, NTP is re-enabled.
pub fn sync_clock<R: CommandRunner>(
    runner: &mut R,
    req: &ClockSyncRequest,
    now: impl Fn() -> f64,
) -> Result<String, String> {
    // Validate before touching anything.
    target_time(&req.gps_time, req.tpv_receipt, now()).map_err(|e| e.to_string())?;

    // timedatectl set-time refuses to run while NTP is active.
    let ntp_disabled = succeeded(&runner.run(&ntp_cmd(false)));
    let mut errors = Vec::new();

    #[cfg(feature = "dbus")]
    if let Ok((target, fix_age)) = target_time(&req.gps_time, req.tpv_receipt, now())
        && try_dbus_set_time(target.fixed_offset()).is_ok()
    {
        return Ok(format!("Clock set via D-Bus (fix age: {fix_age:.3}s)"));
    }

    type Builder = fn(DateTime<Utc>) -> Cmd;
    let methods: [(&str, Builder); 2] = [
        ("timedatectl", timedatectl_set_time_cmd),
        ("sudo date", sudo_date_cmd),
    ];
    for (name, build) in methods {
        let (target, fix_age) =
            target_time(&req.gps_time, req.tpv_receipt, now()).map_err(|e| e.to_string())?;
        let result = runner.run(&build(target));
        if succeeded(&result) {
            let ntp_note = if ntp_disabled { ", NTP disabled" } else { "" };
            return Ok(format!(
                "Clock set via {name} to {} UTC (fix age: {fix_age:.3}s{ntp_note})",
                target.format("%H:%M:%S%.3f")
            ));
        }
        errors.push(format!("{name}: {}", describe(&result)));
    }

    let ntp_state = if !ntp_disabled {
        String::new()
    } else if succeeded(&runner.run(&ntp_cmd(true))) {
        "; NTP re-enabled".to_string()
    } else {
        "; WARNING: NTP left disabled (re-enable failed)".to_string()
    };
    Err(format!(
        "All clock sync methods failed ({}){ntp_state}",
        errors.join("; ")
    ))
}

/// Run [`sync_clock`] with the real runner on a blocking thread.
pub async fn sync_clock_async(req: ClockSyncRequest) -> Result<String, String> {
    tokio::task::spawn_blocking(move || sync_clock(&mut SystemRunner, &req, unix_now))
        .await
        .unwrap_or_else(|e| Err(format!("clock sync task failed: {e}")))
}

#[cfg(feature = "dbus")]
fn try_dbus_set_time(_adjusted: DateTime<chrono::FixedOffset>) -> Result<()> {
    // TODO: Implement D-Bus clock setting via zbus
    Err(anyhow::anyhow!("D-Bus clock sync not yet implemented"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDateTime, TimeZone};
    use std::collections::VecDeque;

    /// Records commands and replies from a script. Never executes anything.
    struct MockRunner {
        calls: Vec<Cmd>,
        replies: VecDeque<bool>,
    }

    impl MockRunner {
        fn new(replies: &[bool]) -> Self {
            Self {
                calls: Vec::new(),
                replies: replies.iter().copied().collect(),
            }
        }
    }

    impl CommandRunner for MockRunner {
        fn run(&mut self, cmd: &Cmd) -> std::io::Result<CmdOutput> {
            self.calls.push(cmd.clone());
            let success = self.replies.pop_front().unwrap_or(false);
            Ok(CmdOutput {
                success,
                stderr: if success {
                    String::new()
                } else {
                    format!("{} denied", cmd.program)
                },
            })
        }
    }

    const GPS_TIME: &str = "2024-01-15T12:30:00.000Z";
    // 2024-01-15T12:30:00Z
    const GPS_EPOCH: f64 = 1_705_321_800.0;

    fn req() -> ClockSyncRequest {
        ClockSyncRequest {
            gps_time: GPS_TIME.to_string(),
            tpv_receipt: GPS_EPOCH + 0.1,
        }
    }

    fn now_fn() -> impl Fn() -> f64 {
        || GPS_EPOCH + 0.1 + 0.25 // fix is 250 ms old
    }

    #[test]
    fn test_target_time_compensates_fix_age_with_subseconds() {
        let (t, age) = target_time(GPS_TIME, 100.0, 100.25).unwrap();
        assert!((age - 0.25).abs() < 1e-9);
        assert_eq!(
            t.format("%Y-%m-%d %H:%M:%S%.6f").to_string(),
            "2024-01-15 12:30:00.250000"
        );
    }

    #[test]
    fn test_target_time_rejects_bad_input() {
        assert!(target_time("garbage", 0.0, 1.0).is_err());
        assert!(target_time(GPS_TIME, 100.0, 99.0).is_err()); // negative age
        assert!(target_time(GPS_TIME, 100.0, 1000.0).is_err()); // ancient fix
        assert!(target_time(GPS_TIME, f64::NAN, 1.0).is_err());
    }

    #[test]
    fn test_command_formats_keep_fraction() {
        let target = Utc.with_ymd_and_hms(2024, 1, 15, 12, 30, 0).unwrap()
            + chrono::Duration::microseconds(123_456);
        assert_eq!(
            sudo_date_cmd(target).args,
            vec!["-n", "date", "-u", "-s", "2024-01-15 12:30:00.123456"]
        );
        let td = timedatectl_set_time_cmd(target);
        assert_eq!(td.program, "timedatectl");
        assert_eq!(&td.args[..2], ["--no-ask-password", "set-time"]);
        // Local time string round-trips to the same instant, sub-second intact
        let naive = NaiveDateTime::parse_from_str(&td.args[2], "%Y-%m-%d %H:%M:%S%.f").unwrap();
        let back = Local.from_local_datetime(&naive).single().unwrap();
        assert_eq!(back.with_timezone(&Utc), target);
    }

    #[test]
    fn test_ntp_cmd_no_ask_password() {
        assert_eq!(
            ntp_cmd(false).to_string(),
            "timedatectl --no-ask-password set-ntp false"
        );
        assert_eq!(
            ntp_cmd(true).to_string(),
            "timedatectl --no-ask-password set-ntp true"
        );
    }

    #[test]
    fn test_sync_succeeds_via_timedatectl() {
        let mut runner = MockRunner::new(&[true, true]);
        let msg = sync_clock(&mut runner, &req(), now_fn()).unwrap();
        assert!(msg.contains("timedatectl"), "{msg}");
        assert!(msg.contains("12:30:00.250"), "{msg}");
        assert!(msg.contains("NTP disabled"), "{msg}");
        assert_eq!(runner.calls.len(), 2);
        assert_eq!(runner.calls[0], ntp_cmd(false));
        assert_eq!(&runner.calls[1].args[1], "set-time");
    }

    #[test]
    fn test_sync_falls_back_to_sudo_date() {
        let mut runner = MockRunner::new(&[true, false, true]);
        let msg = sync_clock(&mut runner, &req(), now_fn()).unwrap();
        assert!(msg.contains("sudo date"), "{msg}");
        assert_eq!(runner.calls.len(), 3);
        assert_eq!(runner.calls[2].args[4], "2024-01-15 12:30:00.250000");
    }

    #[test]
    fn test_sync_failure_reenables_ntp() {
        let mut runner = MockRunner::new(&[true, false, false, true]);
        let err = sync_clock(&mut runner, &req(), now_fn()).unwrap_err();
        assert!(err.contains("All clock sync methods failed"), "{err}");
        assert!(err.contains("NTP re-enabled"), "{err}");
        assert_eq!(runner.calls.last().unwrap(), &ntp_cmd(true));
    }

    #[test]
    fn test_sync_failure_reports_ntp_left_disabled() {
        let mut runner = MockRunner::new(&[true, false, false, false]);
        let err = sync_clock(&mut runner, &req(), now_fn()).unwrap_err();
        assert!(err.contains("NTP left disabled"), "{err}");
    }

    #[test]
    fn test_sync_failure_without_ntp_change_does_not_touch_ntp() {
        let mut runner = MockRunner::new(&[false, false, false]);
        let err = sync_clock(&mut runner, &req(), now_fn()).unwrap_err();
        assert!(!err.contains("NTP"), "{err}");
        assert_eq!(runner.calls.len(), 3);
    }

    #[test]
    fn test_sync_invalid_request_runs_nothing() {
        let mut runner = MockRunner::new(&[]);
        let bad = ClockSyncRequest {
            gps_time: "nope".to_string(),
            tpv_receipt: GPS_EPOCH,
        };
        assert!(sync_clock(&mut runner, &bad, now_fn()).is_err());
        assert!(runner.calls.is_empty());
    }
}
