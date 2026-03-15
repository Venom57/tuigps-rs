/// Format a float with fallback for NaN
pub fn fmt(value: f64, decimals: usize, suffix: &str) -> String {
    if value.is_finite() {
        format!("{:.prec$}{}", value, suffix, prec = decimals)
    } else {
        "---".to_string()
    }
}

/// Format coordinate in DD, DMS, or DDM
pub fn fmt_coord(value: f64, axis: &str, style: &str) -> String {
    if !value.is_finite() {
        return "---".to_string();
    }

    let dir = match axis {
        "lat" => {
            if value >= 0.0 {
                "N"
            } else {
                "S"
            }
        }
        _ => {
            if value >= 0.0 {
                "E"
            } else {
                "W"
            }
        }
    };
    let abs = value.abs();

    match style {
        "dms" => {
            let d = abs as u32;
            let m = ((abs - d as f64) * 60.0) as u32;
            let s = (abs - d as f64 - m as f64 / 60.0) * 3600.0;
            format!("{}° {:02}' {:06.3}\" {}", d, m, s, dir)
        }
        "ddm" => {
            let d = abs as u32;
            let m = (abs - d as f64) * 60.0;
            format!("{}° {:09.6}' {}", d, m, dir)
        }
        _ => format!("{:.7}° {}", abs, dir), // "dd"
    }
}

/// Convert m/s to display unit
pub fn fmt_speed(mps: f64, unit: &str) -> String {
    if !mps.is_finite() {
        return "---".to_string();
    }
    match unit {
        "imperial" => format!("{:.1} mph", mps * 2.236936),
        "nautical" => format!("{:.1} kn", mps * 1.943844),
        _ => format!("{:.1} km/h", mps * 3.6),
    }
}

/// Convert meters to display unit
pub fn fmt_altitude(meters: f64, unit: &str) -> String {
    if !meters.is_finite() {
        return "---".to_string();
    }
    match unit {
        "imperial" | "nautical" => format!("{:.1} ft", meters * 3.28084),
        _ => format!("{:.1} m", meters),
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

/// Format time offset intelligently (ns/us/ms/s)
pub fn fmt_offset(offset_sec: f64) -> String {
    if !offset_sec.is_finite() {
        return "---".to_string();
    }
    let abs = offset_sec.abs();
    let sign = if offset_sec < 0.0 { "-" } else { "+" };
    if abs < 1e-6 {
        format!("{}{:.1} ns", sign, abs * 1e9)
    } else if abs < 1e-3 {
        format!("{}{:.3} us", sign, abs * 1e6)
    } else if abs < 1.0 {
        format!("{}{:.3} ms", sign, abs * 1e3)
    } else {
        format!("{}{:.3} s", sign, abs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fmt_finite() {
        assert_eq!(fmt(3.14159, 2, " m"), "3.14 m");
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
