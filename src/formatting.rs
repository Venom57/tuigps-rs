/// Format a float with fallback for NaN
pub fn fmt(value: f64, decimals: usize, suffix: &str) -> String {
    if value.is_finite() {
        format!("{:.prec$}{}", value, suffix, prec = decimals)
    } else {
        "---".to_string()
    }
}

/// Format coordinate in DD, DMS, or DDM.
///
/// `axis` is `"lat"` (N/S) or anything else (E/W); `style` is `"dms"`,
/// `"ddm"`, or anything else for decimal degrees. Output examples:
///
/// - dd:  `51.5074000° N` (7 decimals)
/// - dms: `51° 30' 26.640" N` (seconds to 3 decimals)
/// - ddm: `51° 30.444000' N` (minutes to 6 decimals)
///
/// Rounding is done once on the smallest displayed unit and then split into
/// degrees/minutes/seconds, so a carry propagates (59.9999" shows as the next
/// minute, never as `60.000"`). A value that rounds to zero gets the positive
/// hemisphere letter. Non-finite input yields `---`.
pub fn fmt_coord(value: f64, axis: &str, style: &str) -> String {
    if !value.is_finite() {
        return "---".to_string();
    }

    let abs = value.abs();
    let hemisphere = |rounded_is_zero: bool| {
        let positive = value >= 0.0 || rounded_is_zero;
        match (axis, positive) {
            ("lat", true) => "N",
            ("lat", false) => "S",
            (_, true) => "E",
            (_, false) => "W",
        }
    };

    match style {
        "dms" => {
            // Whole milliarcseconds.
            let total = (abs * 3_600_000.0).round() as u64;
            let d = total / 3_600_000;
            let m = (total / 60_000) % 60;
            let ms = total % 60_000;
            format!(
                "{}° {:02}' {:02}.{:03}\" {}",
                d,
                m,
                ms / 1000,
                ms % 1000,
                hemisphere(total == 0)
            )
        }
        "ddm" => {
            // Whole micro-minutes.
            let total = (abs * 60_000_000.0).round() as u64;
            let d = total / 60_000_000;
            let um = total % 60_000_000;
            format!(
                "{}° {:02}.{:06}' {}",
                d,
                um / 1_000_000,
                um % 1_000_000,
                hemisphere(total == 0)
            )
        }
        _ => {
            let rounded_is_zero = (abs * 1e7).round() == 0.0;
            format!("{:.7}° {}", abs, hemisphere(rounded_is_zero))
        }
    }
}

/// Meters to international feet.
pub const M_TO_FT: f64 = 3.28084;
/// m/s to ft/min.
pub const MPS_TO_FPM: f64 = 196.850_394;

// Unit conventions (`unit` is the `UnitSystem::as_str()` value):
//
// | quantity           | metric | imperial | nautical |
// |--------------------|--------|----------|----------|
// | horizontal speed   | km/h   | mph      | kn       |
// | vertical speed     | m/s    | ft/min   | ft/min   |
// | altitude, distance | m      | ft       | ft       |
//
// Nautical follows aviation/marine practice: knots over ground, feet for
// altitude and small distances (error estimates, CEP), ft/min for climb.
// Unknown unit strings fall back to metric.

/// Convert m/s to display unit. Non-finite input (or a conversion that
/// overflows) yields `---`.
pub fn fmt_speed(mps: f64, unit: &str) -> String {
    fmt_speed_dec(mps, unit, 1)
}

/// [`fmt_speed`] with a caller-chosen number of decimals.
pub fn fmt_speed_dec(mps: f64, unit: &str, decimals: usize) -> String {
    let (factor, label) = match unit {
        "imperial" => (2.236936, "mph"),
        "nautical" => (1.943844, "kn"),
        _ => (3.6, "km/h"),
    };
    fmt(mps * factor, decimals, &format!(" {label}"))
}

/// Vertical speed: `m/s` with 2 decimals (metric) or `ft/min` with no
/// decimals (imperial and nautical). Non-finite input yields `---`.
pub fn fmt_climb(mps: f64, unit: &str) -> String {
    match unit {
        "imperial" | "nautical" => fmt(mps * MPS_TO_FPM, 0, " ft/min"),
        _ => fmt(mps, 2, " m/s"),
    }
}

/// Convert meters to display unit. Non-finite input (or a conversion that
/// overflows) yields `---`.
pub fn fmt_altitude(meters: f64, unit: &str) -> String {
    fmt_distance(meters, unit, 1)
}

/// A length in meters shown as `m` (metric) or `ft` (imperial, nautical)
/// with `decimals` decimals. Used for altitude, geoid separation, error
/// estimates, CEP and standard deviations. Non-finite input yields `---`.
pub fn fmt_distance(meters: f64, unit: &str, decimals: usize) -> String {
    let (factor, label) = distance_unit(unit);
    fmt(meters * factor, decimals, &format!(" {label}"))
}

/// Conversion factor from meters and label for lengths in `unit`.
pub fn distance_unit(unit: &str) -> (f64, &'static str) {
    match unit {
        "imperial" | "nautical" => (M_TO_FT, "ft"),
        _ => (1.0, "m"),
    }
}

/// Two lengths sharing one unit label, e.g. `0.52/0.48 m`; a missing
/// value shows as `---` in its slot, both missing as plain `---`.
pub fn fmt_distance_pair(a_m: f64, b_m: f64, unit: &str, decimals: usize) -> String {
    let (factor, label) = distance_unit(unit);
    let (a, b) = (a_m * factor, b_m * factor);
    if !a.is_finite() && !b.is_finite() {
        return "---".to_string();
    }
    format!("{}/{} {label}", fmt(a, decimals, ""), fmt(b, decimals, ""))
}

/// Prefix an already formatted value with `±`, leaving the `---`
/// placeholder alone (never `±---`).
pub fn plus_minus(value: String) -> String {
    if value == "---" {
        value
    } else {
        format!("\u{00b1}{value}")
    }
}

/// Elapsed time as `MM:SS` below one hour, `HH:MM:SS` from one hour on
/// (hours keep growing past 99). Sub-second parts are truncated.
pub fn fmt_duration(d: std::time::Duration) -> String {
    let total = d.as_secs();
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h == 0 {
        format!("{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

/// Age in seconds of the last TPV fix, given its local receipt time and
/// the current time (both unix seconds). `None` before the first TPV
/// (`last_tpv` is 0) or for non-finite input. Small negative ages (clock
/// stepped backwards) are clamped to 0.
pub fn fix_age(last_tpv: f64, now: f64) -> Option<f64> {
    if !last_tpv.is_finite() || !now.is_finite() || last_tpv <= 0.0 {
        return None;
    }
    Some((now - last_tpv).max(0.0))
}

/// Compact age: `0.4s` below 10 s, `12s` below a minute, `5m03s` below an
/// hour, then `2h05m`. `None` or non-finite yields `---`.
pub fn fmt_age(age: Option<f64>) -> String {
    let Some(age) = age.filter(|a| a.is_finite()) else {
        return "---".to_string();
    };
    let age = age.max(0.0);
    if age < 9.95 {
        return format!("{age:.1}s");
    }
    let secs = age.round() as u64;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs / 60) % 60)
    }
}

/// Block title for a panel with `lines` lines of content in a bordered
/// area `area_height` rows tall: ` Name ` when everything fits, ` Name … `
/// when lower-priority lines were cut off.
pub fn panel_title(name: &str, lines: usize, area_height: u16) -> String {
    let visible = area_height.saturating_sub(2) as usize;
    if lines > visible {
        format!(" {name} \u{2026} ")
    } else {
        format!(" {name} ")
    }
}

/// Parse ISO 8601 time string into (date, time) parts
pub fn fmt_time_iso(iso: &str) -> (String, String) {
    if iso.is_empty() {
        return ("---".to_string(), "---".to_string());
    }
    if let Some(pos) = iso.find('T') {
        let date = &iso[..pos];
        let time = iso[pos + 1..].trim_end_matches('Z');
        (date.to_string(), time.to_string())
    } else {
        (iso.to_string(), "---".to_string())
    }
}

/// Format time offset intelligently (ns/us/ms/s).
///
/// The unit is chosen on the *rounded* magnitude so values just below a unit
/// boundary display as e.g. `+1.000 us`, never `+1000.0 ns`. A value that
/// rounds to zero is shown with a `+` sign.
pub fn fmt_offset(offset_sec: f64) -> String {
    if !offset_sec.is_finite() {
        return "---".to_string();
    }
    let abs = offset_sec.abs();
    // Format in the smallest unit whose *formatted* magnitude stays below
    // 1000; checking the formatted text avoids float/decimal rounding skew.
    let units: [(f64, usize, &str); 4] = [
        (1e9, 1, "ns"),
        (1e6, 3, "us"),
        (1e3, 3, "ms"),
        (1.0, 3, "s"),
    ];
    let mut text = String::new();
    let mut unit = "s";
    for (scale, decimals, name) in units {
        text = format!("{:.*}", decimals, abs * scale);
        unit = name;
        if text.parse::<f64>().is_ok_and(|v| v < 1000.0) {
            break;
        }
    }
    let is_zero = text.bytes().all(|b| b == b'0' || b == b'.');
    let sign = if offset_sec < 0.0 && !is_zero {
        "-"
    } else {
        "+"
    };
    format!("{sign}{text} {unit}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fmt_finite() {
        assert_eq!(fmt(1.23456, 2, " m"), "1.23 m");
        assert_eq!(fmt(0.0, 1, "°"), "0.0°");
    }

    #[test]
    fn test_fmt_nan() {
        assert_eq!(fmt(f64::NAN, 2, " m"), "---");
        assert_eq!(fmt(f64::INFINITY, 1, ""), "---");
    }

    #[test]
    fn test_fmt_coord_dd() {
        let s = fmt_coord(51.5074, "lat", "dd");
        assert!(s.contains("51.5074"));
        assert!(s.contains("N"));
    }

    #[test]
    fn test_fmt_coord_south() {
        let s = fmt_coord(-33.8688, "lat", "dd");
        assert!(s.contains("S"));
    }

    #[test]
    fn test_fmt_coord_west() {
        let s = fmt_coord(-0.1278, "lon", "dd");
        assert!(s.contains("W"));
    }

    #[test]
    fn test_fmt_coord_dms() {
        let s = fmt_coord(51.5074, "lat", "dms");
        assert!(s.contains("51°"));
        assert!(s.contains("N"));
    }

    #[test]
    fn test_fmt_coord_ddm() {
        let s = fmt_coord(51.5074, "lat", "ddm");
        assert!(s.contains("51°"));
        assert!(s.contains("N"));
    }

    #[test]
    fn test_fmt_coord_nan() {
        assert_eq!(fmt_coord(f64::NAN, "lat", "dd"), "---");
    }

    #[test]
    fn test_fmt_speed_metric() {
        assert_eq!(fmt_speed(10.0, "metric"), "36.0 km/h");
    }

    #[test]
    fn test_fmt_speed_imperial() {
        let s = fmt_speed(10.0, "imperial");
        assert!(s.contains("mph"));
    }

    #[test]
    fn test_fmt_speed_nautical() {
        let s = fmt_speed(10.0, "nautical");
        assert!(s.contains("kn"));
    }

    #[test]
    fn test_fmt_speed_nan() {
        assert_eq!(fmt_speed(f64::NAN, "metric"), "---");
    }

    #[test]
    fn test_fmt_altitude_metric() {
        assert_eq!(fmt_altitude(100.0, "metric"), "100.0 m");
    }

    #[test]
    fn test_fmt_altitude_imperial() {
        let s = fmt_altitude(100.0, "imperial");
        assert!(s.contains("ft"));
    }

    #[test]
    fn test_fmt_altitude_nan() {
        assert_eq!(fmt_altitude(f64::NAN, "metric"), "---");
    }

    #[test]
    fn test_fmt_time_iso_valid() {
        let (d, t) = fmt_time_iso("2024-01-15T12:30:00.000Z");
        assert_eq!(d, "2024-01-15");
        assert_eq!(t, "12:30:00.000");
    }

    #[test]
    fn test_fmt_time_iso_empty() {
        let (d, t) = fmt_time_iso("");
        assert_eq!(d, "---");
        assert_eq!(t, "---");
    }

    #[test]
    fn test_fmt_offset_nanoseconds() {
        let s = fmt_offset(0.5e-9);
        assert!(s.contains("ns"));
    }

    #[test]
    fn test_fmt_offset_microseconds() {
        let s = fmt_offset(50e-6);
        assert!(s.contains("us"));
    }

    #[test]
    fn test_fmt_offset_milliseconds() {
        let s = fmt_offset(0.05);
        assert!(s.contains("ms"));
    }

    #[test]
    fn test_fmt_offset_seconds() {
        let s = fmt_offset(1.5);
        assert!(s.contains("s"));
        assert!(!s.contains("ms"));
    }

    #[test]
    fn test_fmt_offset_negative() {
        let s = fmt_offset(-0.05);
        assert!(s.starts_with('-'));
    }

    #[test]
    fn test_fmt_offset_nan() {
        assert_eq!(fmt_offset(f64::NAN), "---");
    }

    #[test]
    fn test_fmt_speed_dec() {
        assert_eq!(fmt_speed_dec(1.0, "metric", 2), "3.60 km/h");
        assert_eq!(fmt_speed_dec(1.0, "imperial", 2), "2.24 mph");
        assert_eq!(fmt_speed_dec(1.0, "nautical", 2), "1.94 kn");
        assert_eq!(fmt_speed_dec(f64::NAN, "nautical", 2), "---");
    }

    #[test]
    fn test_fmt_climb_units() {
        assert_eq!(fmt_climb(1.234, "metric"), "1.23 m/s");
        assert_eq!(fmt_climb(-0.5, "metric"), "-0.50 m/s");
        assert_eq!(fmt_climb(1.0, "imperial"), "197 ft/min");
        assert_eq!(fmt_climb(-2.54, "nautical"), "-500 ft/min");
        assert_eq!(fmt_climb(1.0, "bogus"), "1.00 m/s");
        assert_eq!(fmt_climb(f64::NAN, "metric"), "---");
        assert_eq!(fmt_climb(f64::NAN, "imperial"), "---");
        assert_eq!(fmt_climb(f64::INFINITY, "nautical"), "---");
    }

    #[test]
    fn test_fmt_distance_units() {
        assert_eq!(fmt_distance(2.5, "metric", 2), "2.50 m");
        assert_eq!(fmt_distance(2.5, "imperial", 2), "8.20 ft");
        assert_eq!(fmt_distance(100.0, "nautical", 1), "328.1 ft");
        assert_eq!(fmt_distance(-47.0, "metric", 1), "-47.0 m");
        assert_eq!(fmt_distance(1.0, "", 0), "1 m");
        assert_eq!(fmt_distance(f64::NAN, "metric", 2), "---");
        assert_eq!(fmt_distance(f64::NAN, "imperial", 2), "---");
        assert_eq!(fmt_distance(f64::MAX, "imperial", 1), "---");
    }

    #[test]
    fn test_fmt_altitude_nautical_is_feet() {
        assert_eq!(fmt_altitude(100.0, "nautical"), "328.1 ft");
        assert_eq!(fmt_altitude(100.0, "imperial"), "328.1 ft");
    }

    #[test]
    fn test_fmt_distance_pair() {
        assert_eq!(fmt_distance_pair(0.52, 0.481, "metric", 2), "0.52/0.48 m");
        assert_eq!(fmt_distance_pair(1.0, 2.0, "imperial", 1), "3.3/6.6 ft");
        assert_eq!(
            fmt_distance_pair(1.0, f64::NAN, "nautical", 1),
            "3.3/--- ft"
        );
        assert_eq!(fmt_distance_pair(f64::NAN, f64::NAN, "metric", 1), "---");
    }

    #[test]
    fn test_plus_minus() {
        assert_eq!(plus_minus("2.50 m".into()), "\u{00b1}2.50 m");
        assert_eq!(plus_minus(fmt(f64::NAN, 2, " m")), "---");
        assert_eq!(plus_minus(fmt_distance(f64::NAN, "imperial", 1)), "---");
    }

    #[test]
    fn test_fmt_duration() {
        use std::time::Duration;
        assert_eq!(fmt_duration(Duration::ZERO), "00:00");
        assert_eq!(fmt_duration(Duration::from_millis(59_999)), "00:59");
        assert_eq!(fmt_duration(Duration::from_secs(754)), "12:34");
        assert_eq!(fmt_duration(Duration::from_secs(3599)), "59:59");
        assert_eq!(fmt_duration(Duration::from_secs(3600)), "01:00:00");
        assert_eq!(fmt_duration(Duration::from_secs(45_296)), "12:34:56");
        assert_eq!(fmt_duration(Duration::from_secs(360_000)), "100:00:00");
    }

    #[test]
    fn test_fix_age() {
        assert_eq!(fix_age(0.0, 1000.0), None);
        assert_eq!(fix_age(f64::NAN, 1000.0), None);
        assert_eq!(fix_age(1000.0, f64::NAN), None);
        assert_eq!(fix_age(1000.0, 1000.5), Some(0.5));
        assert_eq!(fix_age(1000.0, 999.0), Some(0.0));
    }

    #[test]
    fn test_fmt_age() {
        assert_eq!(fmt_age(None), "---");
        assert_eq!(fmt_age(Some(f64::NAN)), "---");
        assert_eq!(fmt_age(Some(0.0)), "0.0s");
        assert_eq!(fmt_age(Some(0.44)), "0.4s");
        assert_eq!(fmt_age(Some(-1.0)), "0.0s");
        assert_eq!(fmt_age(Some(9.94)), "9.9s");
        assert_eq!(fmt_age(Some(9.96)), "10s");
        assert_eq!(fmt_age(Some(59.4)), "59s");
        assert_eq!(fmt_age(Some(59.6)), "1m00s");
        assert_eq!(fmt_age(Some(303.0)), "5m03s");
        assert_eq!(fmt_age(Some(3599.4)), "59m59s");
        assert_eq!(fmt_age(Some(7500.0)), "2h05m");
    }

    #[test]
    fn test_panel_title() {
        assert_eq!(panel_title("Errors", 7, 9), " Errors ");
        assert_eq!(panel_title("Errors", 7, 8), " Errors \u{2026} ");
        assert_eq!(panel_title("Errors", 0, 0), " Errors ");
        assert_eq!(panel_title("Errors", 1, 2), " Errors \u{2026} ");
    }
}
