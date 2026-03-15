use anyhow::Result;
use chrono::DateTime;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn set_clock_from_gps(gps_time_str: &str, last_seen: f64) -> Result<String> {
    // Disable NTP first
    let _ = Command::new("timedatectl")
        .args(["set-ntp", "false"])
        .output();

    // Parse GPS time and compensate for age
    let gps_time = DateTime::parse_from_rfc3339(gps_time_str)?;
    let fix_age = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs_f64()
        - last_seen;
    let adjusted = gps_time + chrono::Duration::milliseconds((fix_age * 1000.0) as i64);

    // Method 1: D-Bus SetTime
    #[cfg(feature = "dbus")]
    if try_dbus_set_time(adjusted).is_ok() {
        return Ok(format!("Clock set via D-Bus (fix age: {:.1}s)", fix_age));
    }

    // Method 2: timedatectl
    let time_str = adjusted.format("%Y-%m-%d %H:%M:%S").to_string();
    if Command::new("timedatectl")
        .args(["set-time", &time_str])
        .output()?
        .status
        .success()
    {
        return Ok(format!(
            "Clock set via timedatectl (fix age: {:.1}s)",
            fix_age
        ));
    }

    // Method 3: sudo date
    let utc_str = adjusted.format("%Y-%m-%dT%H:%M:%S").to_string();
    Command::new("sudo")
        .args(["-n", "date", "-u", "-s", &utc_str])
        .output()?;
    Ok(format!(
        "Clock set via sudo date (fix age: {:.1}s)",
        fix_age
    ))
}

#[cfg(feature = "dbus")]
fn try_dbus_set_time(
    _adjusted: DateTime<chrono::FixedOffset>,
) -> Result<()> {
    // TODO: Implement D-Bus clock setting via zbus
    Err(anyhow::anyhow!("D-Bus clock sync not yet implemented"))
}
