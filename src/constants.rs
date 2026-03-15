use ratatui::style::Color;

use crate::data_model::{FixMode, FixStatus};

pub const GNSS_NAMES: &[(u8, &str)] = &[
    (0, "GPS"),
    (1, "SBAS"),
    (2, "Galileo"),
    (3, "BeiDou"),
    (4, "IMES"),
    (5, "QZSS"),
    (6, "GLONASS"),
    (7, "NavIC"),
];

pub const GNSS_SHORT: &[(u8, &str)] = &[
    (0, "GP"),
    (1, "SB"),
    (2, "GA"),
    (3, "BD"),
    (4, "IM"),
    (5, "QZ"),
    (6, "GL"),
    (7, "IR"),
];

pub const GNSS_COLORS: &[(u8, Color)] = &[
    (0, Color::Green),
    (1, Color::Yellow),
    (2, Color::Blue),
    (3, Color::Red),
    (4, Color::Magenta),
    (5, Color::LightCyan),
    (6, Color::Cyan),
    (7, Color::LightMagenta),
];

pub fn gnss_name(id: u8) -> &'static str {
    GNSS_NAMES
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, n)| *n)
        .unwrap_or("Unknown")
}

pub fn gnss_short(id: u8) -> &'static str {
    GNSS_SHORT
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, n)| *n)
        .unwrap_or("??")
}

pub fn gnss_color(id: u8) -> Color {
    GNSS_COLORS
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, c)| *c)
        .unwrap_or(Color::White)
}

pub fn mode_name(mode: FixMode) -> &'static str {
    match mode {
        FixMode::Unknown => "Unknown",
        FixMode::NoFix => "No Fix",
        FixMode::Fix2D => "2D Fix",
        FixMode::Fix3D => "3D Fix",
    }
}

pub fn mode_color(mode: FixMode) -> Color {
    match mode {
        FixMode::Unknown | FixMode::NoFix => Color::Red,
        FixMode::Fix2D => Color::Yellow,
        FixMode::Fix3D => Color::Green,
    }
}

pub fn status_name(status: FixStatus) -> &'static str {
    match status {
        FixStatus::Unknown => "Unknown",
        FixStatus::Gps => "GPS",
        FixStatus::Dgps => "DGPS",
        FixStatus::RtkFix => "RTK Fix",
        FixStatus::RtkFloat => "RTK Float",
        FixStatus::Dr => "Dead Reckoning",
        FixStatus::GnssDr => "GNSS+DR",
        FixStatus::TimeOnly => "Time Only",
        FixStatus::Simulated => "Simulated",
        FixStatus::PpsFix => "PPS Fix",
    }
}

pub fn status_color(status: FixStatus) -> Color {
    match status {
        FixStatus::Unknown => Color::DarkGray,
        FixStatus::Gps => Color::Green,
        FixStatus::Dgps => Color::LightGreen,
        FixStatus::RtkFix => Color::Cyan,
        FixStatus::RtkFloat => Color::LightCyan,
        FixStatus::Dr => Color::Yellow,
        FixStatus::GnssDr => Color::Yellow,
        FixStatus::TimeOnly => Color::Blue,
        FixStatus::Simulated => Color::Magenta,
        FixStatus::PpsFix => Color::Green,
    }
}

pub fn dop_rating(value: f64) -> (&'static str, Color) {
    if !value.is_finite() {
        return ("---", Color::DarkGray);
    }
    match value {
        v if v < 1.0 => ("Ideal", Color::Green),
        v if v < 2.0 => ("Excellent", Color::Green),
        v if v < 5.0 => ("Good", Color::Yellow),
        v if v < 10.0 => ("Moderate", Color::Yellow),
        v if v < 20.0 => ("Fair", Color::Red),
        _ => ("Poor", Color::Red),
    }
}

pub const COMPASS_POINTS: &[&str] = &[
    "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
    "NW", "NNW",
];

pub fn bearing_to_compass(degrees: f64) -> &'static str {
    if !degrees.is_finite() {
        return "---";
    }
    let idx = ((degrees + 11.25) % 360.0 / 22.5) as usize;
    COMPASS_POINTS[idx.min(15)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gnss_name_known() {
        assert_eq!(gnss_name(0), "GPS");
        assert_eq!(gnss_name(2), "Galileo");
        assert_eq!(gnss_name(6), "GLONASS");
    }

    #[test]
    fn test_gnss_name_unknown() {
        assert_eq!(gnss_name(99), "Unknown");
    }

    #[test]
    fn test_gnss_short() {
        assert_eq!(gnss_short(0), "GP");
        assert_eq!(gnss_short(3), "BD");
    }

    #[test]
    fn test_mode_name() {
        assert_eq!(mode_name(FixMode::Fix3D), "3D Fix");
        assert_eq!(mode_name(FixMode::NoFix), "No Fix");
    }

    #[test]
    fn test_dop_rating_ideal() {
        let (rating, _) = dop_rating(0.5);
        assert_eq!(rating, "Ideal");
    }

    #[test]
    fn test_dop_rating_excellent() {
        let (rating, _) = dop_rating(1.5);
        assert_eq!(rating, "Excellent");
    }

    #[test]
    fn test_dop_rating_poor() {
        let (rating, _) = dop_rating(25.0);
        assert_eq!(rating, "Poor");
    }

    #[test]
    fn test_dop_rating_nan() {
        let (rating, _) = dop_rating(f64::NAN);
        assert_eq!(rating, "---");
    }

    #[test]
    fn test_bearing_north() {
        assert_eq!(bearing_to_compass(0.0), "N");
        assert_eq!(bearing_to_compass(5.0), "N");
    }

    #[test]
    fn test_bearing_east() {
        assert_eq!(bearing_to_compass(90.0), "E");
    }

    #[test]
    fn test_bearing_south() {
        assert_eq!(bearing_to_compass(180.0), "S");
    }

    #[test]
    fn test_bearing_west() {
        assert_eq!(bearing_to_compass(270.0), "W");
    }

    #[test]
    fn test_bearing_nan() {
        assert_eq!(bearing_to_compass(f64::NAN), "---");
    }

    #[test]
    fn test_bearing_360() {
        assert_eq!(bearing_to_compass(359.0), "N");
    }
}
