//! Integration tests for `GpsLogger` output integrity (plan T5, B3, P11).

use std::fs;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::Event;
use tuigps_rs::data_model::{DOPValues, FixMode, FixStatus, GPSData, SatelliteInfo};
use tuigps_rs::gps_logger::{CSV_HEADER, GpsLogger, LogFormat, LogMode};

fn fix(time: &str, lat: f64, lon: f64) -> GPSData {
    GPSData {
        mode: FixMode::Fix3D,
        latitude: lat,
        longitude: lon,
        alt_msl: 12.5,
        speed: 1.25,
        track: 270.0,
        climb: -0.5,
        time: time.to_string(),
        satellites_used: 9,
        dop: DOPValues {
            hdop: 0.9,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// One parsed `<trkpt>`: its attributes and the child element names in order.
#[derive(Debug, Default)]
struct TrkPt {
    lat: String,
    lon: String,
    children: Vec<String>,
    time: String,
}

#[derive(Debug, Default)]
struct Gpx {
    root_attrs: Vec<(String, String)>,
    points: Vec<TrkPt>,
}

/// True when the innermost open elements are `<trkpt><time>`.
fn in_trkpt_time(stack: &[String]) -> bool {
    stack.last().is_some_and(|n| n == "time")
        && stack.iter().rev().nth(1).is_some_and(|n| n == "trkpt")
}

/// Parse a GPX file, failing the test if it is not well-formed XML (including
/// unclosed elements at end of file).
fn parse_gpx(path: &Path) -> Gpx {
    let text = fs::read_to_string(path).unwrap();
    let mut reader = Reader::from_str(&text);
    reader.config_mut().trim_text(true);
    let mut gpx = Gpx::default();
    let mut stack: Vec<String> = Vec::new();
    let mut saw_root = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_string();
                let attrs: Vec<(String, String)> = e
                    .attributes()
                    .map(|a| {
                        let a = a.expect("valid attribute");
                        (
                            a.key.as_ref().to_string(),
                            a.normalized_value(XmlVersion::Implicit1_0)
                                .unwrap()
                                .into_owned(),
                        )
                    })
                    .collect();
                if stack.is_empty() {
                    assert!(!saw_root, "multiple root elements");
                    assert_eq!(name, "gpx");
                    saw_root = true;
                    gpx.root_attrs = attrs;
                } else if name == "trkpt" {
                    let get = |k: &str| {
                        attrs
                            .iter()
                            .find(|(key, _)| key == k)
                            .map(|(_, v)| v.clone())
                            .unwrap_or_default()
                    };
                    gpx.points.push(TrkPt {
                        lat: get("lat"),
                        lon: get("lon"),
                        ..Default::default()
                    });
                } else if stack.last().is_some_and(|p| p == "trkpt") {
                    gpx.points.last_mut().unwrap().children.push(name.clone());
                }
                stack.push(name);
            }
            Ok(Event::Text(t)) => {
                if in_trkpt_time(&stack) {
                    let time = &mut gpx.points.last_mut().unwrap().time;
                    time.push_str(&t.xml_content(XmlVersion::Implicit1_0));
                }
            }
            // quick-xml >= 0.38 reports entity references as separate events
            Ok(Event::GeneralRef(r)) => {
                if in_trkpt_time(&stack) {
                    let resolved = match r.resolve_char_ref().expect("valid char ref") {
                        Some(c) => c.to_string(),
                        None => quick_xml::escape::resolve_predefined_entity(&r)
                            .expect("predefined entity")
                            .to_string(),
                    };
                    gpx.points.last_mut().unwrap().time.push_str(&resolved);
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name().as_ref().to_string();
                assert_eq!(stack.pop().as_deref(), Some(name.as_str()));
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => panic!("malformed XML in {}: {e}", path.display()),
        }
    }
    assert!(saw_root, "no root element");
    assert!(stack.is_empty(), "unclosed elements at EOF: {stack:?}");
    gpx
}

fn root_attr<'a>(gpx: &'a Gpx, key: &str) -> Option<&'a str> {
    gpx.root_attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn assert_gpx11_header(gpx: &Gpx) {
    assert_eq!(root_attr(gpx, "version"), Some("1.1"));
    assert_eq!(root_attr(gpx, "creator"), Some("tuigps-rs"));
    assert_eq!(
        root_attr(gpx, "xmlns"),
        Some("http://www.topografix.com/GPX/1/1")
    );
}

#[test]
fn gpx_well_formed_after_stop() {
    let dir = tempfile::tempdir().unwrap();
    let mut logger = GpsLogger::new(LogFormat::Gpx, dir.path());
    logger.start().unwrap();
    assert!(logger.active);
    logger
        .log_point(&fix("2024-01-15T12:00:00.000Z", 51.5, -0.1))
        .unwrap();
    logger
        .log_point(&fix("2024-01-15T12:00:01.000Z", 51.6, -0.2))
        .unwrap();
    logger.stop().unwrap();
    assert!(!logger.active);
    assert_eq!(logger.point_count, 2);

    let gpx = parse_gpx(Path::new(&logger.filename));
    assert_gpx11_header(&gpx);
    assert_eq!(gpx.points.len(), 2);
    assert_eq!(gpx.points[0].lat, "51.5");
    assert_eq!(gpx.points[0].lon, "-0.1");
    assert_eq!(gpx.points[1].time, "2024-01-15T12:00:01.000Z");
    // GPX 1.1 wptType sequence order.
    assert_eq!(
        gpx.points[0].children,
        ["ele", "time", "fix", "sat", "hdop"]
    );
}

#[test]
fn gpx_well_formed_after_drop_without_stop() {
    let dir = tempfile::tempdir().unwrap();
    let filename;
    {
        let mut logger = GpsLogger::new(LogFormat::Gpx, dir.path());
        logger.start().unwrap();
        logger
            .log_point(&fix("2024-01-15T12:00:00Z", 1.0, 2.0))
            .unwrap();
        filename = logger.filename.clone();
        // dropped here while still active
    }
    let gpx = parse_gpx(Path::new(&filename));
    assert_gpx11_header(&gpx);
    assert_eq!(gpx.points.len(), 1);
}

#[test]
fn gpx_points_flushed_before_stop() {
    let dir = tempfile::tempdir().unwrap();
    let mut logger = GpsLogger::new(LogFormat::Gpx, dir.path());
    logger.start().unwrap();
    logger
        .log_point(&fix("2024-01-15T12:00:00Z", 1.0, 2.0))
        .unwrap();
    // Without stop/drop, the point must already be on disk.
    let text = fs::read_to_string(&logger.filename).unwrap();
    assert!(text.contains("<trkpt"), "point not flushed: {text}");
    assert!(text.contains("</trkpt>"));
}

#[test]
fn gpx_escapes_text_and_omits_missing_values() {
    let dir = tempfile::tempdir().unwrap();
    let mut logger = GpsLogger::new(LogFormat::Gpx, dir.path());
    logger.start().unwrap();
    let mut d = fix("t<&>\"'", 1.0, 2.0);
    d.alt_msl = f64::NAN;
    d.satellites_used = 0;
    d.dop.hdop = f64::NAN;
    d.mode = FixMode::Fix2D;
    logger.log_point(&d).unwrap();
    logger.stop().unwrap();
    let gpx = parse_gpx(Path::new(&logger.filename));
    assert_eq!(gpx.points.len(), 1);
    assert_eq!(gpx.points[0].time, "t<&>\"'");
    assert_eq!(gpx.points[0].children, ["time", "fix"]);
    let text = fs::read_to_string(&logger.filename).unwrap();
    assert!(!text.contains("NaN"));
}

#[test]
fn stop_is_idempotent_and_log_after_stop_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let mut logger = GpsLogger::new(LogFormat::Gpx, dir.path());
    logger.stop().unwrap(); // never started
    logger.start().unwrap();
    logger
        .log_point(&fix("2024-01-15T12:00:00Z", 1.0, 2.0))
        .unwrap();
    logger.stop().unwrap();
    logger.stop().unwrap();
    logger
        .log_point(&fix("2024-01-15T12:00:05Z", 1.0, 2.0))
        .unwrap();
    drop(logger);
    let files: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(files.len(), 1);
    let path = files[0].as_ref().unwrap().path();
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.matches("</gpx>").count(), 1);
    assert_eq!(parse_gpx(&path).points.len(), 1);
}

#[test]
fn restart_finalizes_previous_file_and_uses_new_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut logger = GpsLogger::new(LogFormat::Gpx, dir.path());
    logger.start().unwrap();
    logger
        .log_point(&fix("2024-01-15T12:00:00Z", 1.0, 2.0))
        .unwrap();
    let first = logger.filename.clone();
    logger.start().unwrap();
    assert_ne!(first, logger.filename);
    assert_eq!(logger.point_count, 0);
    // The same timestamp is logged again into the new file.
    logger
        .log_point(&fix("2024-01-15T12:00:00Z", 1.0, 2.0))
        .unwrap();
    assert_eq!(logger.point_count, 1);
    logger.stop().unwrap();
    assert_eq!(parse_gpx(Path::new(&first)).points.len(), 1);
    assert_eq!(parse_gpx(Path::new(&logger.filename)).points.len(), 1);
}

#[test]
fn filename_collision_gets_suffix_and_does_not_truncate() {
    let dir = tempfile::tempdir().unwrap();
    // Pre-create files for this second and the next so the collision is
    // guaranteed even if the clock ticks over between here and start().
    let now = chrono::Local::now();
    let mut existing = Vec::new();
    for offset in [0, 1] {
        let t = now + chrono::Duration::seconds(offset);
        let p = dir
            .path()
            .join(format!("tuigps_{}.csv", t.format("%Y%m%d_%H%M%S")));
        fs::write(&p, "precious").unwrap();
        existing.push(p);
    }

    let mut a = GpsLogger::new(LogFormat::Csv, dir.path());
    a.start().unwrap();
    let mut b = GpsLogger::new(LogFormat::Csv, dir.path());
    b.start().unwrap();
    assert!(a.filename.ends_with("_1.csv"), "{}", a.filename);
    assert_ne!(a.filename, b.filename);
    a.stop().unwrap();
    b.stop().unwrap();

    for p in existing {
        assert_eq!(fs::read_to_string(p).unwrap(), "precious");
    }
}

#[test]
fn csv_header_rows_and_no_nan() {
    let dir = tempfile::tempdir().unwrap();
    let mut logger = GpsLogger::new(LogFormat::Csv, dir.path());
    logger.start().unwrap();

    let mut d = fix("2024-01-15T12:00:00Z", 51.5, -0.1);
    logger.log_point(&d).unwrap();
    d.time = "2024-01-15T12:00:01Z".into();
    d.alt_msl = f64::NAN;
    d.speed = f64::INFINITY;
    d.track = f64::NEG_INFINITY;
    d.climb = f64::NAN;
    logger.log_point(&d).unwrap();
    d.time = "2024-01-15T12:00:02Z".into();
    logger.log_point(&d).unwrap();
    logger.stop().unwrap();
    assert_eq!(logger.point_count, 3);

    let text = fs::read_to_string(&logger.filename).unwrap();
    assert!(!text.contains("NaN") && !text.contains("inf"), "{text}");

    let mut rdr = csv::Reader::from_path(&logger.filename).unwrap();
    let headers: Vec<String> = rdr.headers().unwrap().iter().map(String::from).collect();
    assert_eq!(headers.join(","), CSV_HEADER);
    let rows: Vec<csv::StringRecord> = rdr.records().map(Result::unwrap).collect();
    assert_eq!(rows.len(), 3);
    // The leading seven columns are a stable prefix; later ones are appended
    fn prefix(r: &csv::StringRecord) -> Vec<&str> {
        r.iter().take(7).collect()
    }
    assert_eq!(
        prefix(&rows[0]),
        [
            "2024-01-15T12:00:00Z",
            "51.5",
            "-0.1",
            "12.5",
            "1.25",
            "270",
            "-0.5"
        ]
    );
    assert_eq!(
        prefix(&rows[1]),
        ["2024-01-15T12:00:01Z", "51.5", "-0.1", "", "", "", ""]
    );
}

#[test]
fn csv_extended_columns() {
    let dir = tempfile::tempdir().unwrap();
    let mut logger = GpsLogger::new(LogFormat::Csv, dir.path());
    logger.start().unwrap();

    let mut d = fix("2024-01-15T12:00:00Z", 51.5, -0.1);
    d.alt_hae = 58.0;
    d.status = FixStatus::Dgps;
    let sat = SatelliteInfo {
        prn: 1,
        gnssid: 0,
        svid: 1,
        elevation: 45.0,
        azimuth: 90.0,
        snr: 40.0,
        used: true,
        sigid: 0,
        health: 1,
        freqid: None,
    };
    d.satellites = vec![sat; 12];
    d.dop.vdop = 1.4;
    d.errors.eph = 2.5;
    logger.log_point(&d).unwrap();
    d.time = "2024-01-15T12:00:01Z".into();
    d.mode = FixMode::Fix2D;
    d.status = FixStatus::Unknown;
    logger.log_point(&d).unwrap();
    logger.stop().unwrap();

    let mut rdr = csv::Reader::from_path(&logger.filename).unwrap();
    let headers = rdr.headers().unwrap().clone();
    let rows: Vec<csv::StringRecord> = rdr.records().map(Result::unwrap).collect();
    let col = |row: usize, name: &str| {
        let i = headers.iter().position(|h| h == name).unwrap();
        rows[row].get(i).unwrap().to_string()
    };
    assert_eq!(headers.len(), CSV_HEADER.split(',').count());
    assert!(rows.iter().all(|r| r.len() == headers.len()));
    assert_eq!(col(0, "alt_hae"), "58");
    assert_eq!(col(0, "mode"), "3d");
    assert_eq!(col(0, "status"), "dgps");
    assert_eq!(col(0, "sats_used"), "9");
    assert_eq!(col(0, "sats_visible"), "12");
    assert_eq!(col(0, "hdop"), "0.9");
    assert_eq!(col(0, "vdop"), "1.4");
    assert_eq!(col(0, "pdop"), "", "unreported DOP is an empty field");
    assert_eq!(col(0, "eph"), "2.5");
    assert_eq!(col(1, "mode"), "2d");
    assert_eq!(col(1, "status"), "");
}

#[test]
fn start_named_shares_stem_and_rejects_missing_dir() {
    let dir = tempfile::tempdir().unwrap();
    let mut gpx = GpsLogger::new(LogFormat::Gpx, dir.path());
    let mut csv = GpsLogger::new(LogFormat::Csv, dir.path());
    gpx.start_named("session").unwrap();
    csv.start_named("session").unwrap();
    assert!(gpx.filename.ends_with("session.gpx"));
    assert!(csv.filename.ends_with("session.csv"));

    let mut missing = GpsLogger::new(LogFormat::Csv, dir.path().join("nope"));
    let err = missing.start().unwrap_err().to_string();
    assert!(err.contains("does not exist"), "{err}");
    assert!(!missing.active);
}

#[test]
fn log_mode_formats_and_parsing() {
    assert_eq!(LogMode::Gpx.formats(), [LogFormat::Gpx]);
    assert_eq!(LogMode::Csv.formats(), [LogFormat::Csv]);
    assert_eq!(LogMode::Both.formats(), [LogFormat::Gpx, LogFormat::Csv]);
    for m in LogMode::ALL {
        assert_eq!(m.as_str().parse::<LogMode>(), Ok(m));
        assert_eq!(m.next().prev(), m);
    }
    assert_eq!("BOTH".parse::<LogMode>(), Ok(LogMode::Both));
    assert!("kml".parse::<LogMode>().is_err());
    assert_eq!(LogMode::default(), LogMode::Gpx);
}

#[test]
fn duplicate_timestamps_and_no_fix_are_skipped() {
    for format in [LogFormat::Csv, LogFormat::Gpx] {
        let dir = tempfile::tempdir().unwrap();
        let mut logger = GpsLogger::new(format, dir.path());
        logger.start().unwrap();
        let d = fix("2024-01-15T12:00:00Z", 1.0, 2.0);
        for _ in 0..5 {
            logger.log_point(&d).unwrap();
        }
        assert_eq!(logger.point_count, 1);

        let mut nofix = fix("2024-01-15T12:00:01Z", 1.0, 2.0);
        nofix.mode = FixMode::NoFix;
        logger.log_point(&nofix).unwrap();
        logger
            .log_point(&fix("2024-01-15T12:00:02Z", f64::NAN, 2.0))
            .unwrap();
        logger.log_point(&fix("", 1.0, 2.0)).unwrap();
        assert_eq!(logger.point_count, 1);

        logger
            .log_point(&fix("2024-01-15T12:00:03Z", 1.0, 2.0))
            .unwrap();
        assert_eq!(logger.point_count, 2);
        logger.stop().unwrap();

        let path = Path::new(&logger.filename);
        match format {
            LogFormat::Gpx => assert_eq!(parse_gpx(path).points.len(), 2),
            LogFormat::Csv => {
                let n = csv::Reader::from_path(path).unwrap().records().count();
                assert_eq!(n, 2);
            }
        }
    }
}
