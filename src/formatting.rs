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
