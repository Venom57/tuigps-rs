//! gpsd protocol fixtures (plan T3): realistic gpsd 3.2x JSON lines from
//! `tests/fixtures/*.jsonl` are parsed with `GpsdMessage::parse`, applied to
//! a `GPSData`, and checked field by field. Malformed input must never panic
//! and must never corrupt state that is already there.

use tuigps_rs::data_model::{FixMode, FixStatus, GPSData};
use tuigps_rs::gpsd_client::{GpsdEvent, GpsdMessage, MsgClass, classify_line};

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Lines of an LF-terminated fixture.
fn lines(name: &str) -> Vec<String> {
    fixture(name).lines().map(str::to_string).collect()
}

/// Parse and apply one line; returns the message class if it was accepted.
fn apply(data: &mut GPSData, line: &str, receipt: f64) -> Option<MsgClass> {
    let msg = GpsdMessage::parse(line, receipt)?;
    msg.apply(data);
    Some(msg.class)
}

#[track_caller]
fn assert_close(actual: f64, expected: f64) {
    let tol = 1e-9 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tol,
        "expected {expected}, got {actual}"
    );
}

/// Debug rendering of the whole state; NaN prints consistently, so equal
/// snapshots mean nothing changed.
fn snapshot(data: &GPSData) -> String {
    format!("{data:?}")
}

/// A state with every message family applied, used as the baseline for
/// "malformed input must not corrupt" checks.
fn populated() -> GPSData {
    let mut data = GPSData::default();
    // VERSION + one-device DEVICES, 3D TPV, full SKY, PPS + TOFF
    for (name, count) in [
        ("connect.jsonl", 3),
        ("tpv.jsonl", 1),
        ("sky.jsonl", 1),
        ("timing.jsonl", 2),
    ] {
        for line in lines(name).iter().take(count) {
            apply(&mut data, line, 1000.0).expect("fixture line parses");
        }
    }
    data
}

// === VERSION / DEVICES / WATCH / DEVICE ===

#[test]
fn connect_sequence() {
    let l = lines("connect.jsonl");
    let mut d = GPSData::default();

    assert_eq!(apply(&mut d, &l[0], 1.0), Some(MsgClass::Version));
    assert_eq!(d.version.release, "3.25");
    assert_eq!(d.version.proto_major, 3);
    assert_eq!(d.version.proto_minor, 15);
    assert_eq!(d.last_seen, 1.0);
    assert_eq!(d.last_tpv, 0.0, "only TPV sets last_tpv");

    // No devices: nothing to take device info from
    assert_eq!(apply(&mut d, &l[1], 2.0), Some(MsgClass::Devices));
    assert_eq!(d.device.path, "");
    assert_eq!(d.device.bps, 0);

    // One device
    assert_eq!(apply(&mut d, &l[2], 3.0), Some(MsgClass::Devices));
    assert_eq!(d.device.path, "/dev/ttyACM0");
    assert_eq!(d.device.driver, "u-blox");
    assert_eq!(d.device.subtype, "SW ROM CORE 3.01 (107888),HW 00080000");
    assert_eq!(d.device.activated, "2024-05-12T14:03:27.418Z");
    assert_eq!(d.device.bps, 9600);
    assert_eq!(d.device.native, 1);
    assert_close(d.device.cycle, 1.0);
    assert_close(d.device.mincycle, 0.02);

    // Two devices: the first one is shown
    assert_eq!(apply(&mut d, &l[3], 4.0), Some(MsgClass::Devices));
    assert_eq!(d.device.path, "/dev/ttyUSB0");
    assert_eq!(d.device.driver, "SiRF");
    assert_eq!(d.device.subtype, "GSW3.5.0_3.5.00.00-SDK-3EP2.01");
    assert_eq!(d.device.bps, 4800);
    assert_close(d.device.mincycle, 0.10);

    // WATCH is accepted but carries no state
    let before = snapshot(&d);
    assert_eq!(apply(&mut d, &l[4], 4.0), Some(MsgClass::Watch));
    assert_eq!(snapshot(&d), before);

    // DEVICE notification (driver switch + speed change)
    assert_eq!(apply(&mut d, &l[5], 5.0), Some(MsgClass::Device));
    assert_eq!(d.device.driver, "u-blox");
    assert_eq!(d.device.bps, 115200);
    assert_eq!(d.device.native, 0);
    assert_close(d.device.cycle, 0.20);
    assert_eq!(d.device.activated, "2024-05-12T14:03:30.512Z");
    // ...absent subtype is kept
    assert_eq!(d.device.subtype, "GSW3.5.0_3.5.00.00-SDK-3EP2.01");

    // Partial DEVICE (subtype only) keeps everything else
    assert_eq!(apply(&mut d, &l[6], 6.0), Some(MsgClass::Device));
    assert_eq!(d.device.subtype, "SW EXT CORE 1.00 (3fda8e),HW 00190000");
    assert_eq!(d.device.driver, "u-blox");
    assert_eq!(d.device.bps, 115200);
    assert_eq!(d.last_seen, 6.0);
}

// === TPV ===

#[test]
fn tpv_3d_fix_all_fields() {
    let l = lines("tpv.jsonl");
    let mut d = GPSData::default();
    assert_eq!(apply(&mut d, &l[0], 1715522701.25), Some(MsgClass::Tpv));

    assert_eq!(d.mode, FixMode::Fix3D);
    assert_eq!(d.status, FixStatus::Dgps);
    assert_eq!(d.time, "2024-05-12T14:05:01.000Z");
    assert_eq!(d.leapseconds, 18);
    assert_close(d.latitude, 51.477928333);
    assert_close(d.longitude, -0.001545);
    assert_close(d.alt_hae, 91.345);
    assert_close(d.alt_msl, 45.561);
    assert_close(d.geoid_sep, 45.784);
    assert_close(d.speed, 0.012);
    assert_close(d.track, 123.4567);
    assert_close(d.climb, -0.003);
    assert_close(d.magtrack, 124.0122);
    assert_close(d.magvar, 0.6);

    let e = &d.errors;
    assert_close(e.eph, 1.85);
    assert_close(e.epv, 2.345);
    assert_close(e.ept, 0.005);
    assert_close(e.eps, 0.25);
    assert_close(e.epd, 12.3456);
    assert_close(e.epc, 4.69);
    assert_close(e.epx, 1.042);
    assert_close(e.epy, 1.329);
    assert_close(e.sep, 3.1);

    assert_close(d.ecefx, 3980581.91);
    assert_close(d.ecefy, -111.27);
    assert_close(d.ecefz, 4966824.52);
    assert_close(d.ecefvx, 0.01);
    assert_close(d.ecefvy, -0.02);
    assert_close(d.ecefvz, 0.0);

    assert!(d.has_fix());
    assert_eq!(d.last_tpv, 1715522701.25);
    assert_eq!(d.last_seen, 1715522701.25);
}

#[test]
fn tpv_2d_no_fix_mode0_and_optional_fields() {
    let l = lines("tpv.jsonl");
    let mut d = GPSData::default();
    apply(&mut d, &l[0], 1.0);

    // 2D fix: no altitude, no status; absent fields reset to NaN
    apply(&mut d, &l[1], 2.0);
    assert_eq!(d.mode, FixMode::Fix2D);
    assert_eq!(d.status, FixStatus::Unknown);
    assert_close(d.latitude, -33.856784);
    assert_close(d.longitude, 151.215297);
    assert!(d.alt_hae.is_nan() && d.alt_msl.is_nan() && d.climb.is_nan());
    assert!(d.ecefx.is_nan() && d.magtrack.is_nan());
    assert_close(d.errors.eph, 22.9);
    assert!(d.errors.epv.is_nan());
    assert!(d.has_fix());

    // No fix: time and leap seconds only
    apply(&mut d, &l[2], 3.0);
    assert_eq!(d.mode, FixMode::NoFix);
    assert_eq!(d.time, "2024-05-12T14:05:03.000Z");
    assert_eq!(d.leapseconds, 18);
    assert!(d.latitude.is_nan() && d.longitude.is_nan());
    assert!(!d.has_fix());
    assert_close(d.errors.ept, 0.005);

    // Mode 0 (unknown): everything numeric unset
    apply(&mut d, &l[3], 4.0);
    assert_eq!(d.mode, FixMode::Unknown);
    assert_eq!(d.leapseconds, 0);
    assert!(d.errors.ept.is_nan());
    assert!(!d.has_fix());
    assert_eq!(d.last_tpv, 4.0);

    // RTK fixed, negative ellipsoidal height, sub-second time
    apply(&mut d, &l[4], 5.0);
    assert_eq!(d.status, FixStatus::RtkFix);
    assert_close(d.alt_hae, -12.5);
    assert_close(d.alt_msl, 20.25);
    assert_eq!(d.time, "2024-05-12T14:05:04.250Z");
}

// === SKY ===

#[test]
fn sky_full_mixed_constellations() {
    let l = lines("sky.jsonl");
    let mut d = GPSData::default();
    assert_eq!(apply(&mut d, &l[0], 1.0), Some(MsgClass::Sky));

    assert_close(d.dop.xdop, 0.54);
    assert_close(d.dop.ydop, 0.61);
    assert_close(d.dop.vdop, 1.02);
    assert_close(d.dop.tdop, 0.58);
    assert_close(d.dop.hdop, 0.81);
    assert_close(d.dop.gdop, 1.48);
    assert_close(d.dop.pdop, 1.30);

    assert_eq!(d.satellites.len(), 8);
    assert_eq!(d.satellites_used, 5);
    let prns: Vec<u16> = d.satellites.iter().map(|s| s.prn).collect();
    assert_eq!(prns, [2, 12, 131, 68, 79, 305, 419, 193]);
    let gnss: Vec<u8> = d.satellites.iter().map(|s| s.gnssid).collect();
    assert_eq!(gnss, [0, 0, 1, 6, 6, 2, 3, 5]);

    let glo = &d.satellites[3];
    assert_eq!((glo.svid, glo.sigid, glo.health), (4, 0, 1));
    assert_eq!(glo.freqid, Some(13));
    assert_close(glo.elevation, 33.0);
    assert_close(glo.azimuth, 76.0);
    assert_close(glo.snr, 31.0);
    assert!(glo.used);
    assert_eq!(d.satellites[4].freqid, Some(0), "freqid 0 is a value");
    assert_eq!(d.satellites[0].freqid, None, "freqid only for GLONASS");

    let gal = &d.satellites[5];
    assert_eq!((gal.svid, gal.sigid), (5, 1));

    let qzss = &d.satellites[7];
    assert_eq!(qzss.health, 2);
    assert_close(qzss.elevation, -1.0);
    assert!(!qzss.used);
    assert_close(d.satellites[1].snr, 0.0);

    let counts = d.constellation_counts();
    assert_eq!(counts[&0], (2, 1));
    assert_eq!(counts[&6], (2, 2));
    assert_eq!(counts[&1], (1, 0));
}

#[test]
fn sky_partial_and_empty() {
    let l = lines("sky.jsonl");
    let mut d = GPSData::default();
    apply(&mut d, &l[0], 1.0);

    // DOP-only SKY: satellites survive
    apply(&mut d, &l[1], 2.0);
    assert_close(d.dop.hdop, 0.92);
    assert_close(d.dop.pdop, 1.43);
    assert_close(d.dop.xdop, 0.60);
    assert_eq!(d.satellites.len(), 8);
    assert_eq!(d.satellites_used, 5);

    // Satellites-only SKY: DOPs survive, list replaced
    apply(&mut d, &l[2], 3.0);
    assert_close(d.dop.hdop, 0.92);
    assert_eq!(d.satellites.len(), 2);
    assert_eq!(d.satellites_used, 1);
    let untracked = &d.satellites[1];
    assert_eq!(untracked.prn, 24);
    assert!(untracked.elevation.is_nan() && untracked.azimuth.is_nan());
    assert_close(untracked.snr, 19.0);
    assert_eq!((untracked.sigid, untracked.health), (0, 0));

    // Empty satellites array clears the list
    apply(&mut d, &l[3], 4.0);
    assert!(d.satellites.is_empty());
    assert_eq!(d.satellites_used, 0);
    assert_close(d.dop.hdop, 0.92);
}

// === PPS / TOFF / ERROR ===

#[test]
fn timing_and_error() {
    let l = lines("timing.jsonl");
    let mut d = GPSData::default();

    assert_eq!(apply(&mut d, &l[0], 1.0), Some(MsgClass::Pps));
    assert_eq!(d.pps.real_sec, 1715522702);
    assert_eq!(d.pps.real_nsec, 0);
    assert_eq!(d.pps.clock_sec, 1715522701);
    assert_eq!(d.pps.clock_nsec, 999987654);
    assert_eq!(d.pps.precision, -20);
    assert_eq!(d.pps.qerr, -12345);
    assert!((d.pps_offset_us() - 12.346).abs() < 1e-6);
    assert_eq!(d.last_tpv, 0.0);

    assert_eq!(apply(&mut d, &l[1], 2.0), Some(MsgClass::Toff));
    assert_eq!(d.toff.real_sec, 1715522702);
    assert_eq!(d.toff.real_nsec, 0);
    assert_eq!(d.toff.clock_sec, 1715522702);
    assert_eq!(d.toff.clock_nsec, 123456789);
    // TOFF must not touch PPS
    assert_eq!(d.pps.clock_nsec, 999987654);

    let msg = GpsdMessage::parse(&l[2], 3.0).unwrap();
    assert_eq!(msg.class, MsgClass::Error);
    assert_eq!(msg.json["message"], "Unrecognized request '?FOO'");
    let before = snapshot(&d);
    msg.apply(&mut d);
    assert_eq!(d.last_seen, 3.0);
    d.last_seen = 2.0;
    assert_eq!(snapshot(&d), before, "ERROR changes nothing but last_seen");
}

// === Classes the app does not handle ===

#[test]
fn unknown_classes_are_ignored() {
    let mut d = populated();
    let before = snapshot(&d);
    for line in lines("unknown_classes.jsonl") {
        assert!(GpsdMessage::parse(&line, 1.0).is_none(), "{line}");
        assert!(classify_line(&line, 1.0).is_none(), "{line}");
        assert_eq!(apply(&mut d, &line, 1.0), None);
    }
    // GST's lat/lon are error estimates and POLL embeds a TPV: neither
    // may leak into the position.
    assert_eq!(snapshot(&d), before);
}

// === Line endings and framing ===

#[test]
fn crlf_lines() {
    let content = fixture("crlf.jsonl");
    assert!(content.contains("\r\n"), "fixture must keep CRLF endings");
    let raw: Vec<&str> = content.split_terminator('\n').collect();
    assert_eq!(raw.len(), 5);
    assert!(raw.iter().all(|l| l.ends_with('\r')));

    let mut d = GPSData::default();
    // Trailing \r is JSON whitespace: parse accepts it directly
    assert_eq!(apply(&mut d, raw[0], 1.0), Some(MsgClass::Version));
    assert_eq!(d.version.release, "3.22");
    assert_eq!(d.version.proto_minor, 14);

    match classify_line(raw[1], 2.0) {
        Some(GpsdEvent::Message(m)) => m.apply(&mut d),
        other => panic!("expected TPV, got {other:?}"),
    }
    assert_eq!(d.mode, FixMode::Fix3D);
    assert_close(d.latitude, 48.85837);
    assert_close(d.longitude, 2.294481);
    assert_eq!(d.time, "2024-05-12T14:05:05.000Z");

    match classify_line(raw[2], 2.0) {
        Some(GpsdEvent::Nmea(s)) => {
            assert!(s.starts_with("$GPGGA,140505.00"));
            assert!(s.ends_with("*68"), "CR stripped: {s:?}");
        }
        other => panic!("expected NMEA, got {other:?}"),
    }
    assert!(classify_line(raw[3], 2.0).is_none(), "bare CR is blank");

    assert_eq!(apply(&mut d, raw[4], 3.0), Some(MsgClass::Sky));
    assert_close(d.dop.hdop, 1.25);
    assert_eq!(d.satellites.len(), 1);
}

#[test]
fn session_fixture_classifies_every_line() {
    let mut d = GPSData::default();
    let mut classes = Vec::new();
    let mut nmea = 0;
    let mut ignored = 0;
    for line in lines("session.jsonl") {
        match classify_line(&line, 10.0) {
            Some(GpsdEvent::Message(m)) => {
                classes.push(m.class);
                m.apply(&mut d);
            }
            Some(GpsdEvent::Nmea(_)) => nmea += 1,
            Some(other) => panic!("unexpected {other:?}"),
            None => ignored += 1,
        }
    }
    use MsgClass::*;
    assert_eq!(
        classes,
        [Version, Devices, Watch, Tpv, Sky, Pps, Toff, Tpv],
        "GST is dropped"
    );
    assert_eq!(nmea, 3);
    assert_eq!(ignored, 2, "GST + blank line");
    assert_eq!(d.time, "2024-05-12T14:05:02.000Z");
    assert_close(d.latitude, 51.4779285);
    assert_eq!(d.satellites.len(), 3);
    assert_eq!(d.satellites_used, 2);
    assert_eq!(d.device.driver, "u-blox");
    assert_eq!(d.version.release, "3.25");
}

// === Malformed input ===

/// Index of each line in `malformed.jsonl`.
mod m {
    pub const TRUNCATED: usize = 0;
    pub const WRONG_TYPES_TPV: usize = 1;
    pub const PRN_OVERFLOW: usize = 2;
    pub const MODE_NEGATIVE: usize = 3;
    pub const MODE_WRAPS: usize = 4;
    pub const HUGE_FLOATS: usize = 5;
    pub const PPS_EXTREMES: usize = 6;
    pub const OUT_OF_RANGE_FLOAT: usize = 7;
    pub const NON_OBJECTS: std::ops::RangeInclusive<usize> = 8..=13;
    pub const LOWERCASE_CLASS: usize = 14;
    pub const SKY_SATS_NOT_ARRAY: usize = 15;
    pub const SKY_SATS_NON_OBJECTS: usize = 16;
    pub const DEVICE_WRONG_TYPES: usize = 17;
    pub const DEVICES_NOT_ARRAY: usize = 18;
    pub const SAT_FIELDS_OUT_OF_RANGE: usize = 19;
    pub const VERSION_WRONG_TYPES: usize = 20;
    pub const BLANK: usize = 21;
    pub const WHITESPACE: usize = 22;
    pub const NMEA: usize = 23;
    pub const BOM: usize = 24;
    pub const TRAILING_GARBAGE: usize = 25;
}

#[test]
fn malformed_every_line_never_panics() {
    let l = lines("malformed.jsonl");
    assert_eq!(l.len(), 26);
    let mut d = populated();
    for line in &l {
        let _ = classify_line(line, 5.0);
        apply(&mut d, line, 5.0);
        // Model invariants that rendering relies on
        assert!(d.satellites_used as usize <= d.satellites.len());
        let _ = d.pps_offset_us();
        let _ = d.constellation_counts();
    }
}

#[test]
fn malformed_rejected_lines_leave_state_untouched() {
    let l = lines("malformed.jsonl");
    let mut rejected: Vec<usize> = vec![
        m::TRUNCATED,
        m::OUT_OF_RANGE_FLOAT,
        m::LOWERCASE_CLASS,
        m::BLANK,
        m::WHITESPACE,
        m::NMEA,
        m::BOM,
        m::TRAILING_GARBAGE,
    ];
    rejected.extend(m::NON_OBJECTS);
    let mut d = populated();
    let before = snapshot(&d);
    for i in rejected {
        assert!(
            GpsdMessage::parse(&l[i], 9.0).is_none(),
            "line {i}: {}",
            l[i]
        );
        assert_eq!(apply(&mut d, &l[i], 9.0), None);
    }
    assert_eq!(snapshot(&d), before);
    assert!(matches!(
        classify_line(&l[m::NMEA], 0.0),
        Some(GpsdEvent::Nmea(_))
    ));
}

#[test]
fn malformed_wrong_types_read_as_absent() {
    let l = lines("malformed.jsonl");
    let mut d = populated();
    let time_before = d.time.clone();

    // TPV with every field mistyped: a TPV replaces the fix, so typed
    // fields become "unknown", and a non-string time is not stored.
    assert_eq!(
        apply(&mut d, &l[m::WRONG_TYPES_TPV], 9.0),
        Some(MsgClass::Tpv)
    );
    assert_eq!(d.mode, FixMode::Unknown);
    assert_eq!(d.status, FixStatus::Unknown);
    assert!(d.latitude.is_nan() && d.longitude.is_nan() && d.alt_msl.is_nan());
    assert_eq!(d.leapseconds, 0);
    assert_eq!(d.time, time_before);
    assert!(!d.has_fix());

    apply(&mut d, &l[m::MODE_NEGATIVE], 9.0);
    assert_eq!(d.mode, FixMode::Unknown);
    assert_eq!(d.status, FixStatus::Unknown);

    // SKY: mistyped DOP and non-array satellites are ignored (partial SKY)
    let mut d = populated();
    let (hdop, sats) = (d.dop.hdop, d.satellites.len());
    apply(&mut d, &l[m::SKY_SATS_NOT_ARRAY], 9.0);
    assert_eq!(d.dop.hdop, hdop);
    assert_eq!(d.satellites.len(), sats);

    // DEVICE / DEVICES: mistyped fields keep the previous values
    let dev_before = format!("{:?}", d.device);
    apply(&mut d, &l[m::DEVICES_NOT_ARRAY], 9.0);
    assert_eq!(format!("{:?}", d.device), dev_before);
    apply(&mut d, &l[m::DEVICE_WRONG_TYPES], 9.0);
    assert_eq!(d.device.path, "/dev/ttyACM0");
    assert_eq!(d.device.driver, "u-blox");
    assert_eq!(d.device.bps, 9600);
    assert_close(d.device.cycle, 1.0);
    assert_close(d.device.mincycle, 0.02);

    // VERSION: non-string release is kept
    apply(&mut d, &l[m::VERSION_WRONG_TYPES], 9.0);
    assert_eq!(d.version.release, "3.25");
    assert_eq!(d.version.proto_major, 0);
    assert_eq!(d.version.proto_minor, 0);
}

#[test]
fn malformed_out_of_range_integers_are_not_truncated() {
    let l = lines("malformed.jsonl");
    let mut d = GPSData::default();

    // 259 / 257 must not wrap to a 3D GPS fix
    apply(&mut d, &l[m::MODE_WRAPS], 1.0);
    assert_eq!(d.mode, FixMode::Unknown);
    assert_eq!(d.status, FixStatus::Unknown);
    assert!(!d.has_fix());

    // 2^32 + 18 must not wrap to 18
    apply(&mut d, &l[m::HUGE_FLOATS], 1.0);
    assert_eq!(d.leapseconds, 0);

    // PRN/svid 70000 must not wrap to 4464
    apply(&mut d, &l[m::PRN_OVERFLOW], 1.0);
    assert_eq!(d.satellites.len(), 1);
    assert_eq!((d.satellites[0].prn, d.satellites[0].svid), (0, 0));
    assert!(d.satellites[0].used);

    apply(&mut d, &l[m::SAT_FIELDS_OUT_OF_RANGE], 1.0);
    let s = &d.satellites[0];
    assert_eq!(s.prn, 70);
    assert_eq!((s.gnssid, s.sigid, s.health), (0, 0, 0));
    assert_eq!(s.freqid, None, "freqid 200 must not wrap to -56");
    assert!(!s.used, "\"yes\" is not a bool");

    // Non-object satellite entries are skipped
    apply(&mut d, &l[m::SKY_SATS_NON_OBJECTS], 1.0);
    assert_eq!(d.satellites.len(), 1);
    assert_eq!(d.satellites[0].prn, 9);
    assert_eq!(d.satellites_used, 1);

    // DEVICE native 300 keeps the previous value
    let mut d = populated();
    apply(&mut d, &l[m::DEVICE_WRONG_TYPES], 1.0);
    assert_eq!(d.device.native, 1);
}

#[test]
fn malformed_pps_extremes() {
    let l = lines("malformed.jsonl");
    let mut d = GPSData::default();
    apply(&mut d, &l[m::PPS_EXTREMES], 1.0);
    assert_eq!(d.pps.real_sec, i64::MAX);
    assert_eq!(d.pps.clock_sec, i64::MIN);
    assert_eq!(d.pps.clock_nsec, -999_999_999);
    assert_eq!(d.pps.precision, 0, "-(2^32)-20 must not wrap to -20");
    assert_eq!(d.pps.qerr, 0, "u64::MAX does not fit i64");
    assert!(d.pps_offset_us().is_finite());
}

#[test]
fn malformed_huge_coords_are_rejected() {
    let l = lines("malformed.jsonl");
    let mut d = GPSData::default();
    apply(&mut d, &l[m::HUGE_FLOATS], 1.0);
    // Out-of-range lat/lon are treated as missing, so no bogus fix appears
    assert!(d.latitude.is_nan());
    assert!(d.longitude.is_nan());
    assert!(!d.has_fix());
}

#[test]
fn oversized_and_deeply_nested_lines() {
    let mut d = populated();
    let before = snapshot(&d);

    // 1 MiB TPV: valid JSON, just huge. It parses and must not panic.
    let pad = "x".repeat(1 << 20);
    let big = format!(r#"{{"class":"TPV","mode":3,"lat":1.5,"lon":2.5,"pad":"{pad}"}}"#);
    let mut tmp = d.clone();
    assert_eq!(apply(&mut tmp, &big, 1.0), Some(MsgClass::Tpv));
    assert_close(tmp.latitude, 1.5);

    // 1 MiB of garbage
    assert!(GpsdMessage::parse(&"{".repeat(1 << 20), 1.0).is_none());

    // Nesting deeper than serde_json's recursion limit is rejected, not a
    // stack overflow
    let deep = format!(
        r#"{{"class":"SKY","satellites":{}{}}}"#,
        "[".repeat(10_000),
        "]".repeat(10_000)
    );
    assert_eq!(apply(&mut d, &deep, 1.0), None);
    assert_eq!(snapshot(&d), before);
}

#[test]
fn invalid_utf8_after_lossy_decode() {
    // The client decodes lines with from_utf8_lossy; replacement characters
    // inside strings are fine, outside strings the line is rejected.
    let inside = b"{\"class\":\"VERSION\",\"release\":\"3.\xff\xfe\",\"proto_major\":3}";
    let mut d = GPSData::default();
    let line = String::from_utf8_lossy(inside);
    assert_eq!(apply(&mut d, &line, 1.0), Some(MsgClass::Version));
    assert_eq!(d.version.release, "3.\u{fffd}\u{fffd}");

    let outside = b"{\"class\":\"TPV\",\xff\"mode\":3}";
    assert!(GpsdMessage::parse(&String::from_utf8_lossy(outside), 1.0).is_none());
}
