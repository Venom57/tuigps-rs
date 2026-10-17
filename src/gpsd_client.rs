use anyhow::{Result, anyhow};
use serde_json::Value;
use std::time::{Duration, UNIX_EPOCH};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::data_model::*;

/// Maximum accepted length of a single gpsd line. Longer lines are discarded.
pub const MAX_LINE_LEN: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub enum GpsdEvent {
    Update(Box<GPSData>),
    Error(String),
    Nmea(String),
}

pub async fn gpsd_task(
    host: String,
    port: u16,
    tx: mpsc::Sender<GpsdEvent>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        match connect_and_read(&host, port, &tx, &mut shutdown).await {
            Ok(()) => break, // clean shutdown
            Err(e) => {
                let _ = tx.send(GpsdEvent::Error(e.to_string())).await;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {},
                    _ = shutdown.changed() => break,
                }
            }
        }
    }
}

async fn connect_and_read(
    host: &str,
    port: u16,
    tx: &mpsc::Sender<GpsdEvent>,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let stream = TcpStream::connect((host, port)).await?;
    let (reader, mut writer) = stream.into_split();

    // Send WATCH command
    let watch = r#"?WATCH={"enable":true,"json":true,"pps":true,"timing":true,"nmea":true}"#;
    writer.write_all(watch.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let mut reader = BufReader::new(reader);
    let mut buf = Vec::with_capacity(4096);
    let mut data = GPSData {
        connected: true,
        ..Default::default()
    };

    loop {
        tokio::select! {
            result = read_line_bounded(&mut reader, &mut buf, MAX_LINE_LEN) => {
                match result? {
                    LineRead::Eof => return Err(anyhow!("gpsd connection closed")),
                    LineRead::Oversized => continue,
                    LineRead::Line => {}
                }
                // Invalid UTF-8 is replaced rather than dropping the connection
                let line = String::from_utf8_lossy(&buf);

                let receipt_time = std::time::SystemTime::now()
                    .duration_since(UNIX_EPOCH)?
                    .as_secs_f64();

                // Check for NMEA
                let trimmed = line.trim();
                if trimmed.starts_with('$') || trimmed.starts_with('!') {
                    let _ = tx.send(GpsdEvent::Nmea(trimmed.to_string())).await;
                } else {
                    process_message(trimmed, &mut data, receipt_time);
                    let _ = tx.send(GpsdEvent::Update(Box::new(data.clone()))).await;
                }
            }
            _ = shutdown.changed() => return Ok(()),
        }
    }
}

fn process_message(line: &str, data: &mut GPSData, receipt_time: f64) {
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return,
    };
    let class = msg["class"].as_str().unwrap_or("");

    match class {
        "TPV" => extract_tpv(&msg, data),
        "SKY" => extract_sky(&msg, data),
        "PPS" => extract_pps(&msg, data),
        "TOFF" => extract_toff(&msg, data),
        "DEVICES" => extract_devices(&msg, data),
        "DEVICE" => extract_device(&msg, data),
        "VERSION" => extract_version(&msg, data),
        _ => {} // ignore unknown classes
    }

    data.last_seen = receipt_time;
}

/// Outcome of [`read_line_bounded`].
#[derive(Debug, PartialEq, Eq)]
pub enum LineRead {
    /// A line (without its trailing newline) is in the buffer.
    Line,
    /// A line longer than the limit was read and discarded.
    Oversized,
    /// End of stream, no more data.
    Eof,
}

/// Read one `\n`-terminated line into `buf` (newline stripped), never holding
/// more than `max` bytes. Oversized lines are consumed and discarded.
pub async fn read_line_bounded<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<LineRead> {
    buf.clear();
    let mut oversized = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(if oversized {
                LineRead::Oversized
            } else if buf.is_empty() {
                LineRead::Eof
            } else {
                LineRead::Line
            });
        }
        let newline = available.iter().position(|&b| b == b'\n');
        let chunk = match newline {
            Some(i) => &available[..i],
            None => available,
        };
        if !oversized {
            if buf.len() + chunk.len() > max {
                oversized = true;
                buf.clear();
            } else {
                buf.extend_from_slice(chunk);
            }
        }
        let consumed = newline.map_or(available.len(), |i| i + 1);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(if oversized {
                LineRead::Oversized
            } else {
                LineRead::Line
            });
        }
    }
}

fn json_f64(msg: &Value, key: &str) -> f64 {
    msg[key].as_f64().unwrap_or(f64::NAN)
}

fn extract_tpv(msg: &Value, data: &mut GPSData) {
    data.mode = FixMode::from(msg["mode"].as_u64().unwrap_or(0) as u8);
    data.status = FixStatus::from(msg["status"].as_u64().unwrap_or(0) as u8);

    data.latitude = json_f64(msg, "lat");
    data.longitude = json_f64(msg, "lon");
    data.alt_hae = json_f64(msg, "altHAE");
    data.alt_msl = json_f64(msg, "altMSL");
    data.geoid_sep = json_f64(msg, "geoidSep");
    data.speed = json_f64(msg, "speed");
    data.track = json_f64(msg, "track");
    data.climb = json_f64(msg, "climb");
    data.magtrack = json_f64(msg, "magtrack");
    data.magvar = json_f64(msg, "magvar");

    data.leapseconds = msg["leapseconds"].as_i64().unwrap_or(0) as i32;

    // Error estimates
    data.errors.eph = json_f64(msg, "eph");
    data.errors.epv = json_f64(msg, "epv");
    data.errors.ept = json_f64(msg, "ept");
    data.errors.eps = json_f64(msg, "eps");
    data.errors.epd = json_f64(msg, "epd");
    data.errors.epc = json_f64(msg, "epc");
    data.errors.epx = json_f64(msg, "epx");
    data.errors.epy = json_f64(msg, "epy");
    data.errors.sep = json_f64(msg, "sep");

    // ECEF
    data.ecefx = json_f64(msg, "ecefx");
    data.ecefy = json_f64(msg, "ecefy");
    data.ecefz = json_f64(msg, "ecefz");
    data.ecefvx = json_f64(msg, "ecefvx");
    data.ecefvy = json_f64(msg, "ecefvy");
    data.ecefvz = json_f64(msg, "ecefvz");

    // Store GPS time string (TOFF computation is done in app.rs)
    if let Some(time_str) = msg["time"].as_str() {
        data.time = time_str.to_string();
    }
}

fn extract_sky(msg: &Value, data: &mut GPSData) {
    data.dop.hdop = json_f64(msg, "hdop");
    data.dop.vdop = json_f64(msg, "vdop");
    data.dop.pdop = json_f64(msg, "pdop");
    data.dop.gdop = json_f64(msg, "gdop");
    data.dop.tdop = json_f64(msg, "tdop");
    data.dop.xdop = json_f64(msg, "xdop");
    data.dop.ydop = json_f64(msg, "ydop");

    if let Some(sats) = msg["satellites"].as_array()
        && !sats.is_empty()
    {
        data.satellites = sats
            .iter()
            .map(|s| SatelliteInfo {
                prn: s["PRN"].as_u64().unwrap_or(0) as u16,
                gnssid: s["gnssid"].as_u64().unwrap_or(0) as u8,
                svid: s["svid"].as_u64().unwrap_or(0) as u16,
                elevation: json_f64(s, "el"),
                azimuth: json_f64(s, "az"),
                snr: json_f64(s, "ss"),
                used: s["used"].as_bool().unwrap_or(false),
                sigid: s["sigid"].as_u64().unwrap_or(0) as u8,
                health: s["health"].as_u64().unwrap_or(0) as u8,
                freqid: s["freqid"].as_i64().map(|v| v as i8),
            })
            .collect();
        data.satellites_used = data.satellites.iter().filter(|s| s.used).count() as u32;
    }
}

fn extract_pps(msg: &Value, data: &mut GPSData) {
    data.pps.real_sec = msg["real_sec"].as_i64().unwrap_or(0);
    data.pps.real_nsec = msg["real_nsec"].as_i64().unwrap_or(0);
    data.pps.clock_sec = msg["clock_sec"].as_i64().unwrap_or(0);
    data.pps.clock_nsec = msg["clock_nsec"].as_i64().unwrap_or(0);
    data.pps.precision = msg["precision"].as_i64().unwrap_or(0) as i32;
    data.pps.qerr = msg["qErr"].as_i64().unwrap_or(0);
}

fn extract_toff(msg: &Value, data: &mut GPSData) {
    data.toff.real_sec = msg["real_sec"].as_i64().unwrap_or(0);
    data.toff.real_nsec = msg["real_nsec"].as_i64().unwrap_or(0);
    data.toff.clock_sec = msg["clock_sec"].as_i64().unwrap_or(0);
    data.toff.clock_nsec = msg["clock_nsec"].as_i64().unwrap_or(0);
}

fn extract_devices(msg: &Value, data: &mut GPSData) {
    if let Some(devices) = msg["devices"].as_array()
        && let Some(dev) = devices.first()
    {
        extract_device_fields(dev, data);
    }
}

fn extract_device(msg: &Value, data: &mut GPSData) {
    extract_device_fields(msg, data);
}

fn extract_device_fields(msg: &Value, data: &mut GPSData) {
    if let Some(v) = msg["path"].as_str() {
        data.device.path = v.to_string();
    }
    if let Some(v) = msg["driver"].as_str() {
        data.device.driver = v.to_string();
    }
    if let Some(v) = msg["subtype"].as_str() {
        data.device.subtype = v.to_string();
    }
    data.device.bps = msg["bps"].as_u64().unwrap_or(0) as u32;
    data.device.cycle = json_f64(msg, "cycle");
    data.device.mincycle = json_f64(msg, "mincycle");
    if let Some(v) = msg["activated"].as_str() {
        data.device.activated = v.to_string();
    }
    data.device.native = msg["native"].as_u64().unwrap_or(0) as u8;
}

fn extract_version(msg: &Value, data: &mut GPSData) {
    if let Some(v) = msg["release"].as_str() {
        data.version.release = v.to_string();
    }
    data.version.proto_major = msg["proto_major"].as_u64().unwrap_or(0) as u32;
    data.version.proto_minor = msg["proto_minor"].as_u64().unwrap_or(0) as u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_f64_present() {
        let v: Value = serde_json::from_str(r#"{"lat": 51.5074}"#).unwrap();
        assert!((json_f64(&v, "lat") - 51.5074).abs() < 1e-10);
    }

    #[test]
    fn test_json_f64_missing() {
        let v: Value = serde_json::from_str(r#"{}"#).unwrap();
        assert!(json_f64(&v, "lat").is_nan());
    }

    #[test]
    fn test_extract_tpv() {
        let mut data = GPSData::default();
        let msg: Value = serde_json::from_str(
            r#"{
                "class": "TPV",
                "mode": 3,
                "status": 1,
                "lat": 51.5074,
                "lon": -0.1278,
                "altHAE": 100.5,
                "altMSL": 55.2,
                "speed": 1.5,
                "track": 180.0,
                "climb": 0.1,
                "time": "2024-01-15T12:30:00.000Z",
                "eph": 5.0,
                "epv": 10.0
            }"#,
        )
        .unwrap();

        extract_tpv(&msg, &mut data);

        assert_eq!(data.mode, FixMode::Fix3D);
        assert_eq!(data.status, FixStatus::Gps);
        assert!((data.latitude - 51.5074).abs() < 1e-10);
        assert!((data.longitude - (-0.1278)).abs() < 1e-10);
        assert!((data.alt_hae - 100.5).abs() < 1e-10);
        assert!((data.speed - 1.5).abs() < 1e-10);
        assert_eq!(data.time, "2024-01-15T12:30:00.000Z");
        assert!((data.errors.eph - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_extract_sky() {
        let mut data = GPSData::default();
        let msg: Value = serde_json::from_str(
            r#"{
                "class": "SKY",
                "hdop": 0.8,
                "vdop": 1.2,
                "pdop": 1.5,
                "gdop": 2.0,
                "tdop": 0.9,
                "satellites": [
                    {"PRN": 1, "gnssid": 0, "svid": 1, "el": 45.0, "az": 180.0, "ss": 35.0, "used": true, "sigid": 0, "health": 1},
                    {"PRN": 5, "gnssid": 0, "svid": 5, "el": 20.0, "az": 90.0, "ss": 25.0, "used": false, "sigid": 0, "health": 1}
                ]
            }"#,
        )
        .unwrap();

        extract_sky(&msg, &mut data);

        assert!((data.dop.hdop - 0.8).abs() < 1e-10);
        assert!((data.dop.vdop - 1.2).abs() < 1e-10);
        assert_eq!(data.satellites.len(), 2);
        assert_eq!(data.satellites_used, 1);
        assert_eq!(data.satellites[0].prn, 1);
        assert!(data.satellites[0].used);
        assert!(!data.satellites[1].used);
    }

    #[test]
    fn test_extract_pps() {
        let mut data = GPSData::default();
        let msg: Value = serde_json::from_str(
            r#"{
                "class": "PPS",
                "real_sec": 1705318200,
                "real_nsec": 500,
                "clock_sec": 1705318200,
                "clock_nsec": 1000,
                "precision": -20,
                "qErr": 50
            }"#,
        )
        .unwrap();

        extract_pps(&msg, &mut data);

        assert_eq!(data.pps.real_sec, 1705318200);
        assert_eq!(data.pps.real_nsec, 500);
        assert_eq!(data.pps.precision, -20);
        assert_eq!(data.pps.qerr, 50);
    }

    #[test]
    fn test_extract_version() {
        let mut data = GPSData::default();
        let msg: Value = serde_json::from_str(
            r#"{
                "class": "VERSION",
                "release": "3.25",
                "proto_major": 3,
                "proto_minor": 15
            }"#,
        )
        .unwrap();

        extract_version(&msg, &mut data);

        assert_eq!(data.version.release, "3.25");
        assert_eq!(data.version.proto_major, 3);
        assert_eq!(data.version.proto_minor, 15);
    }

    #[test]
    fn test_extract_device() {
        let mut data = GPSData::default();
        let msg: Value = serde_json::from_str(
            r#"{
                "class": "DEVICE",
                "path": "/dev/ttyACM0",
                "driver": "u-blox",
                "subtype": "SW 1.00,HW 00080000",
                "bps": 9600,
                "cycle": 1.0
            }"#,
        )
        .unwrap();

        extract_device(&msg, &mut data);

        assert_eq!(data.device.path, "/dev/ttyACM0");
        assert_eq!(data.device.driver, "u-blox");
        assert_eq!(data.device.bps, 9600);
    }

    #[test]
    fn test_process_message_invalid_json() {
        let mut data = GPSData::default();
        process_message("not valid json", &mut data, 0.0);
        // Should not crash, data unchanged
        assert_eq!(data.mode, FixMode::Unknown);
    }

    #[test]
    fn test_process_message_unknown_class() {
        let mut data = GPSData::default();
        process_message(r#"{"class": "UNKNOWN"}"#, &mut data, 100.0);
        assert!((data.last_seen - 100.0).abs() < 1e-10);
    }

    // === Bounded line reader ===

    async fn read_all_lines(input: &[u8], cap: usize, max: usize) -> Vec<(LineRead, Vec<u8>)> {
        let mut reader = BufReader::with_capacity(cap, input);
        let mut buf = Vec::new();
        let mut out = Vec::new();
        loop {
            let r = read_line_bounded(&mut reader, &mut buf, max).await.unwrap();
            let done = r == LineRead::Eof;
            out.push((r, buf.clone()));
            if done {
                return out;
            }
        }
    }

    #[tokio::test]
    async fn test_bounded_reader_lines_and_eof() {
        let out = read_all_lines(b"abc\ndef\nghi", 2, 100).await;
        assert_eq!(
            out,
            vec![
                (LineRead::Line, b"abc".to_vec()),
                (LineRead::Line, b"def".to_vec()),
                (LineRead::Line, b"ghi".to_vec()),
                (LineRead::Eof, vec![]),
            ]
        );
    }

    #[tokio::test]
    async fn test_bounded_reader_discards_oversized() {
        let mut input = vec![b'x'; 50];
        input.extend_from_slice(b"\nok\n");
        input.extend_from_slice(&[b'y'; 20]); // oversized, unterminated at EOF
        let out = read_all_lines(&input, 4, 10).await;
        assert_eq!(
            out,
            vec![
                (LineRead::Oversized, vec![]),
                (LineRead::Line, b"ok".to_vec()),
                (LineRead::Oversized, vec![]),
                (LineRead::Eof, vec![]),
            ]
        );
    }

    #[tokio::test]
    async fn test_bounded_reader_exact_limit_ok() {
        let out = read_all_lines(b"0123456789\n", 3, 10).await;
        assert_eq!(out[0], (LineRead::Line, b"0123456789".to_vec()));
    }
}
