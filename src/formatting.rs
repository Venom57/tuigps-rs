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

/// Convert m/s to display unit. Non-finite input (or a conversion that
/// overflows) yields `---`.
pub fn fmt_speed(mps: f64, unit: &str) -> String {
    let (factor, label) = match unit {
        "imperial" => (2.236936, "mph"),
        "nautical" => (1.943844, "kn"),
        _ => (3.6, "km/h"),
    };
    fmt(mps * factor, 1, &format!(" {label}"))
}

/// Convert meters to display unit. Non-finite input (or a conversion that
/// overflows) yields `---`.
pub fn fmt_altitude(meters: f64, unit: &str) -> String {
    match unit {
        "imperial" | "nautical" => fmt(meters * 3.28084, 1, " ft"),
        _ => fmt(meters, 1, " m"),
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
}
