//! Exact-output tests for the formatting helpers (plan B13, T7).

use proptest::prelude::*;
use tuigps_rs::formatting::{fmt, fmt_altitude, fmt_coord, fmt_offset, fmt_speed, fmt_time_iso};

#[test]
fn coord_dd_exact() {
    assert_eq!(fmt_coord(51.5074, "lat", "dd"), "51.5074000° N");
    assert_eq!(fmt_coord(-33.8688, "lat", "dd"), "33.8688000° S");
    assert_eq!(fmt_coord(-0.1278, "lon", "dd"), "0.1278000° W");
    assert_eq!(fmt_coord(151.2093, "lon", "dd"), "151.2093000° E");
    assert_eq!(fmt_coord(179.999_999_99, "lon", "dd"), "180.0000000° E");
    // Unknown style falls back to decimal degrees.
    assert_eq!(fmt_coord(1.5, "lat", "bogus"), "1.5000000° N");
}

#[test]
fn coord_dms_exact() {
    assert_eq!(fmt_coord(51.5074, "lat", "dms"), "51° 30' 26.640\" N");
    assert_eq!(fmt_coord(-33.8688, "lat", "dms"), "33° 52' 07.680\" S");
    assert_eq!(fmt_coord(-0.1278, "lon", "dms"), "0° 07' 40.080\" W");
    assert_eq!(fmt_coord(-90.0, "lat", "dms"), "90° 00' 00.000\" S");
    assert_eq!(fmt_coord(180.0, "lon", "dms"), "180° 00' 00.000\" E");
}

#[test]
fn coord_ddm_exact() {
    assert_eq!(fmt_coord(51.5074, "lat", "ddm"), "51° 30.444000' N");
    assert_eq!(fmt_coord(-0.1278, "lon", "ddm"), "0° 07.668000' W");
    assert_eq!(fmt_coord(-180.0, "lon", "ddm"), "180° 00.000000' W");
}

#[test]
fn dms_seconds_carry_into_minutes_and_degrees() {
    // 10° 59' 59.9999" rounds to 11° 00' 00.000", never 10° 59' 60.000".
    let v = 10.0 + 59.0 / 60.0 + 59.9999 / 3600.0;
    assert_eq!(fmt_coord(v, "lat", "dms"), "11° 00' 00.000\" N");
    assert_eq!(fmt_coord(-v, "lon", "dms"), "11° 00' 00.000\" W");
    // Just below the rounding boundary stays put.
    let v = 10.0 + 59.0 / 60.0 + 59.9994 / 3600.0;
    assert_eq!(fmt_coord(v, "lat", "dms"), "10° 59' 59.999\" N");
    // Seconds carry into minutes only.
    let v = 10.0 + 14.0 / 60.0 + 59.99999 / 3600.0;
    assert_eq!(fmt_coord(v, "lat", "dms"), "10° 15' 00.000\" N");
}

#[test]
fn ddm_minutes_carry_into_degrees() {
    let v = 10.0 + 59.999_999_9 / 60.0;
    assert_eq!(fmt_coord(v, "lon", "ddm"), "11° 00.000000' E");
    let v = -(45.0 + 59.999_999_6 / 60.0);
    assert_eq!(fmt_coord(v, "lat", "ddm"), "46° 00.000000' S");
    let v = -(45.0 + 59.999_96 / 60.0);
    assert_eq!(fmt_coord(v, "lat", "ddm"), "45° 59.999960' S");
}

#[test]
fn coord_rounding_to_zero_uses_positive_hemisphere() {
    assert_eq!(fmt_coord(-1e-12, "lat", "dd"), "0.0000000° N");
    assert_eq!(fmt_coord(-1e-12, "lat", "dms"), "0° 00' 00.000\" N");
    assert_eq!(fmt_coord(-1e-12, "lon", "ddm"), "0° 00.000000' E");
    assert_eq!(fmt_coord(-0.0, "lon", "dd"), "0.0000000° E");
}

#[test]
fn coord_non_finite() {
    for style in ["dd", "dms", "ddm"] {
        assert_eq!(fmt_coord(f64::NAN, "lat", style), "---");
        assert_eq!(fmt_coord(f64::INFINITY, "lon", style), "---");
        assert_eq!(fmt_coord(f64::NEG_INFINITY, "lat", style), "---");
    }
}

#[test]
fn fmt_exact() {
    assert_eq!(fmt(1.23456, 2, " m"), "1.23 m");
    assert_eq!(fmt(0.996, 2, ""), "1.00");
    assert_eq!(fmt(-12.345_67, 3, "°"), "-12.346°");
    assert_eq!(fmt(42.0, 0, " sats"), "42 sats");
    assert_eq!(fmt(f64::NEG_INFINITY, 1, " m"), "---");
}

#[test]
fn speed_and_altitude_exact() {
    assert_eq!(fmt_speed(10.0, "metric"), "36.0 km/h");
    assert_eq!(fmt_speed(10.0, "imperial"), "22.4 mph");
    assert_eq!(fmt_speed(10.0, "nautical"), "19.4 kn");
    assert_eq!(fmt_speed(0.0, "metric"), "0.0 km/h");
    assert_eq!(fmt_speed(f64::INFINITY, "imperial"), "---");
    assert_eq!(fmt_altitude(100.0, "metric"), "100.0 m");
    assert_eq!(fmt_altitude(100.0, "imperial"), "328.1 ft");
    assert_eq!(fmt_altitude(100.0, "nautical"), "328.1 ft");
    assert_eq!(fmt_altitude(-12.34, "metric"), "-12.3 m");
    assert_eq!(fmt_altitude(f64::NEG_INFINITY, "metric"), "---");
    // Unit conversion overflowing to infinity must not print "inf".
    assert_eq!(fmt_speed(-1.2e308, "metric"), "---");
    assert_eq!(fmt_altitude(1.2e308, "imperial"), "---");
}

#[test]
fn time_iso_exact() {
    assert_eq!(
        fmt_time_iso("2024-01-15T12:30:00.000Z"),
        ("2024-01-15".to_string(), "12:30:00.000".to_string())
    );
    assert_eq!(
        fmt_time_iso("2024-01-15"),
        ("2024-01-15".to_string(), "---".to_string())
    );
    assert_eq!(fmt_time_iso(""), ("---".to_string(), "---".to_string()));
}

#[test]
fn offset_exact() {
    assert_eq!(fmt_offset(0.5e-9), "+0.5 ns");
    assert_eq!(fmt_offset(-0.5e-9), "-0.5 ns");
    assert_eq!(fmt_offset(0.0), "+0.0 ns");
    assert_eq!(fmt_offset(50e-6), "+50.000 us");
    assert_eq!(fmt_offset(0.05), "+50.000 ms");
    assert_eq!(fmt_offset(-2.5e-3), "-2.500 ms");
    assert_eq!(fmt_offset(1.5), "+1.500 s");
    assert_eq!(fmt_offset(f64::NAN), "---");
}

#[test]
fn offset_unit_boundaries_carry() {
    // Values that would round up to 1000 in the smaller unit move up a unit.
    assert_eq!(fmt_offset(999.96e-9), "+1.000 us");
    assert_eq!(fmt_offset(999.9996e-6), "+1.000 ms");
    assert_eq!(fmt_offset(0.999_999_6), "+1.000 s");
    assert_eq!(fmt_offset(-999.96e-9), "-1.000 us");
    // Just below the boundary stays in the smaller unit.
    assert_eq!(fmt_offset(999.94e-9), "+999.9 ns");
    // Tiny negatives that round to zero are not shown as "-0.0".
    assert_eq!(fmt_offset(-1e-12), "+0.0 ns");
}

/// Parse `D° MM' SS.sss" H` into (deg, min, sec, hemisphere).
fn parse_dms(s: &str) -> (u64, u64, f64, char) {
    let parts: Vec<&str> = s.split(' ').collect();
    assert_eq!(parts.len(), 4, "{s}");
    let d = parts[0].trim_end_matches('°').parse().unwrap();
    let m = parts[1].trim_end_matches('\'').parse().unwrap();
    let sec = parts[2].trim_end_matches('"').parse().unwrap();
    (d, m, sec, parts[3].chars().next().unwrap())
}

/// Parse `D° MM.mmmmmm' H` into (deg, min, hemisphere).
fn parse_ddm(s: &str) -> (u64, f64, char) {
    let parts: Vec<&str> = s.split(' ').collect();
    assert_eq!(parts.len(), 3, "{s}");
    let d = parts[0].trim_end_matches('°').parse().unwrap();
    let m = parts[1].trim_end_matches('\'').parse().unwrap();
    (d, m, parts[2].chars().next().unwrap())
}

proptest! {
    #[test]
    fn helpers_never_panic(v in proptest::num::f64::ANY, decimals in 0usize..20) {
        let outputs = [
            fmt(v, decimals, " x"),
            fmt_coord(v, "lat", "dd"),
            fmt_coord(v, "lon", "dms"),
            fmt_coord(v, "lat", "ddm"),
            fmt_speed(v, "metric"),
            fmt_speed(v, "imperial"),
            fmt_speed(v, "nautical"),
            fmt_altitude(v, "metric"),
            fmt_altitude(v, "imperial"),
            fmt_offset(v),
        ];
        for out in outputs {
            prop_assert!(!out.contains("NaN") && !out.contains("inf"), "{}", out);
            if !v.is_finite() {
                prop_assert_eq!(out, "---");
            }
        }
    }

    #[test]
    fn time_iso_never_panics(s in ".*") {
        let _ = fmt_time_iso(&s);
    }

    #[test]
    fn dms_fields_in_range_and_round_trip(v in -180.0f64..=180.0) {
        let (d, m, sec, h) = parse_dms(&fmt_coord(v, "lon", "dms"));
        prop_assert!(m < 60);
        prop_assert!(sec < 60.0, "sec = {}", sec);
        let back = d as f64 + m as f64 / 60.0 + sec / 3600.0;
        prop_assert!((back - v.abs()).abs() <= 0.0005 / 3600.0 + 1e-12);
        if v.abs() > 1e-6 {
            prop_assert_eq!(h, if v > 0.0 { 'E' } else { 'W' });
        }
    }

    #[test]
    fn ddm_fields_in_range_and_round_trip(v in -90.0f64..=90.0) {
        let (d, m, h) = parse_ddm(&fmt_coord(v, "lat", "ddm"));
        prop_assert!(m < 60.0, "min = {}", m);
        let back = d as f64 + m / 60.0;
        prop_assert!((back - v.abs()).abs() <= 0.000_000_5 / 60.0 + 1e-12);
        if v.abs() > 1e-6 {
            prop_assert_eq!(h, if v > 0.0 { 'N' } else { 'S' });
        }
    }

    #[test]
    fn offset_magnitude_below_1000_unless_seconds(v in -1e3f64..1e3) {
        let out = fmt_offset(v);
        let (num, unit) = out.split_once(' ').unwrap();
        let mag: f64 = num[1..].parse().unwrap();
        if unit != "s" {
            prop_assert!(mag < 1000.0, "{}", out);
        }
    }
}
