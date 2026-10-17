use anyhow::{Result, anyhow};
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::data_model::*;

/// Maximum accepted length of a single gpsd line. Longer lines are discarded.
pub const MAX_LINE_LEN: usize = 64 * 1024;

/// Delay between reconnection attempts.
pub const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// gpsd JSON message class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgClass {
    Tpv,
    Sky,
    Pps,
    Toff,
    Devices,
    Device,
    Version,
    Watch,
    Error,
    Other,
}

impl MsgClass {
    pub fn from_class_str(class: &str) -> Self {
        match class {
            "TPV" => MsgClass::Tpv,
            "SKY" => MsgClass::Sky,
            "PPS" => MsgClass::Pps,
            "TOFF" => MsgClass::Toff,
            "DEVICES" => MsgClass::Devices,
            "DEVICE" => MsgClass::Device,
            "VERSION" => MsgClass::Version,
            "WATCH" => MsgClass::Watch,
            "ERROR" => MsgClass::Error,
            _ => MsgClass::Other,
        }
    }
}

/// One parsed gpsd JSON message, tagged with its class and the local time at
/// which it was received. The app applies it to its own `GPSData`, so the
/// client never has to clone the whole state per line.
#[derive(Debug, Clone)]
pub struct GpsdMessage {
    pub class: MsgClass,
    pub json: Value,
    pub receipt_time: f64,
}

impl GpsdMessage {
    /// Parse a gpsd JSON line. Returns `None` for invalid JSON and for
    /// message classes we do not handle, so nothing is sent for them.
    pub fn parse(line: &str, receipt_time: f64) -> Option<Self> {
        let json: Value = serde_json::from_str(line).ok()?;
        let class = MsgClass::from_class_str(json["class"].as_str()?);
        if class == MsgClass::Other {
            return None;
        }
        Some(Self {
            class,
            json,
            receipt_time,
        })
    }

    /// Apply this message to `data`, updating only what the message carries.
    pub fn apply(&self, data: &mut GPSData) {
        let msg = &self.json;
        match self.class {
            MsgClass::Tpv => {
                extract_tpv(msg, data);
                data.last_tpv = self.receipt_time;
            }
            MsgClass::Sky => extract_sky(msg, data),
            MsgClass::Pps => extract_pps(msg, data),
            MsgClass::Toff => extract_toff(msg, data),
            MsgClass::Devices => extract_devices(msg, data),
            MsgClass::Device => extract_device(msg, data),
            MsgClass::Version => extract_version(msg, data),
            MsgClass::Watch | MsgClass::Error | MsgClass::Other => {}
        }
        data.last_seen = self.receipt_time;
    }
}

#[derive(Debug, Clone)]
pub enum GpsdEvent {
    /// TCP connection established and WATCH sent.
    Connected,
    Message(GpsdMessage),
    Error(String),
    Nmea(String),
}

/// An event tagged with the connection generation of the task that sent it,
/// so the app can discard events from a task it has already replaced.
pub type TaggedEvent = (u64, GpsdEvent);

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub async fn gpsd_task(
    host: String,
    port: u16,
    generation: u64,
    tx: mpsc::Sender<TaggedEvent>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        match connect_and_read(&host, port, generation, &tx, &mut shutdown).await {
            Ok(()) => break, // clean shutdown
            Err(e) => {
                if tx
                    .send((generation, GpsdEvent::Error(e.to_string())))
                    .await
                    .is_err()
                {
                    break; // receiver gone
                }
                tokio::select! {
                    _ = tokio::time::sleep(RECONNECT_DELAY) => {},
                    _ = shutdown.changed() => break,
                }
            }
        }
    }
}

async fn connect_and_read(
    host: &str,
    port: u16,
    generation: u64,
    tx: &mpsc::Sender<TaggedEvent>,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let stream = tokio::select! {
        s = TcpStream::connect((host, port)) => s?,
        _ = shutdown.changed() => return Ok(()),
    };
    let (reader, mut writer) = stream.into_split();

    // Send WATCH command
    let watch = r#"?WATCH={"enable":true,"json":true,"pps":true,"timing":true,"nmea":true}"#;
    writer.write_all(watch.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let send = |event: GpsdEvent| tx.send((generation, event));
    send(GpsdEvent::Connected)
        .await
        .map_err(|_| anyhow!("event receiver closed"))?;

    let mut reader = BufReader::new(reader);
    let mut buf = Vec::with_capacity(4096);

    loop {
        tokio::select! {
            result = read_line_bounded(&mut reader, &mut buf, MAX_LINE_LEN) => {
                let event = match result? {
                    LineRead::Eof => return Err(anyhow!("gpsd connection closed")),
                    LineRead::Oversized => continue,
                    LineRead::Line => {
                        let line = String::from_utf8_lossy(&buf);
                        classify_line(&line, unix_now())
                    }
                };
                if let Some(event) = event
                    && send(event).await.is_err()
                {
                    return Ok(()); // receiver gone: nothing left to do
                }
            }
            _ = shutdown.changed() => return Ok(()),
        }
    }
}

/// Turn one raw line from gpsd into an event, or `None` if it should be ignored.
pub fn classify_line(line: &str, receipt_time: f64) -> Option<GpsdEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('$') || trimmed.starts_with('!') {
        return Some(GpsdEvent::Nmea(trimmed.to_string()));
    }
    GpsdMessage::parse(trimmed, receipt_time).map(GpsdEvent::Message)
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

/// Integer field `key` as `T`, or `None` if it is absent, not an integer, or
/// out of range for `T`. Out-of-range values are never silently truncated
/// (e.g. `"mode":259` must not become mode 3).
fn json_int<T: TryFrom<i64>>(msg: &Value, key: &str) -> Option<T> {
    msg[key].as_i64().and_then(|v| T::try_from(v).ok())
}

/// Overwrite `target` only if `key` is present as a number.
fn set_if_present(msg: &Value, key: &str, target: &mut f64) {
    if let Some(v) = msg[key].as_f64() {
        *target = v;
    }
}

fn extract_tpv(msg: &Value, data: &mut GPSData) {
    data.mode = FixMode::from(json_int::<u8>(msg, "mode").unwrap_or(0));
    data.status = FixStatus::from(json_int::<u8>(msg, "status").unwrap_or(0));

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

    data.leapseconds = json_int(msg, "leapseconds").unwrap_or(0);

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

/// SKY messages may be partial (e.g. u-blox sends DOPs and satellites in
/// separate messages), so only fields that are present are overwritten.
fn extract_sky(msg: &Value, data: &mut GPSData) {
    set_if_present(msg, "hdop", &mut data.dop.hdop);
    set_if_present(msg, "vdop", &mut data.dop.vdop);
    set_if_present(msg, "pdop", &mut data.dop.pdop);
    set_if_present(msg, "gdop", &mut data.dop.gdop);
    set_if_present(msg, "tdop", &mut data.dop.tdop);
    set_if_present(msg, "xdop", &mut data.dop.xdop);
    set_if_present(msg, "ydop", &mut data.dop.ydop);

    // An explicit (possibly empty) array replaces the list; absent keeps it.
    if let Some(sats) = msg["satellites"].as_array() {
        // Non-object entries are skipped rather than turned into PRN 0.
        data.satellites = sats
            .iter()
            .filter(|s| s.is_object())
            .map(|s| SatelliteInfo {
                prn: json_int(s, "PRN").unwrap_or(0),
                gnssid: json_int(s, "gnssid").unwrap_or(0),
                svid: json_int(s, "svid").unwrap_or(0),
                elevation: json_f64(s, "el"),
                azimuth: json_f64(s, "az"),
                snr: json_f64(s, "ss"),
                used: s["used"].as_bool().unwrap_or(false),
                sigid: json_int(s, "sigid").unwrap_or(0),
                health: json_int(s, "health").unwrap_or(0),
                freqid: json_int(s, "freqid"),
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
    data.pps.precision = json_int(msg, "precision").unwrap_or(0);
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

/// DEVICE notifications can be partial, so only present fields are updated.
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
    if let Some(v) = json_int(msg, "bps") {
        data.device.bps = v;
    }
    set_if_present(msg, "cycle", &mut data.device.cycle);
    set_if_present(msg, "mincycle", &mut data.device.mincycle);
    if let Some(v) = msg["activated"].as_str() {
        data.device.activated = v.to_string();
    }
    if let Some(v) = json_int(msg, "native") {
        data.device.native = v;
    }
}

fn extract_version(msg: &Value, data: &mut GPSData) {
    if let Some(v) = msg["release"].as_str() {
        data.version.release = v.to_string();
    }
    data.version.proto_major = json_int(msg, "proto_major").unwrap_or(0);
    data.version.proto_minor = json_int(msg, "proto_minor").unwrap_or(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    fn apply_line(line: &str, data: &mut GPSData, t: f64) -> Option<MsgClass> {
        let msg = GpsdMessage::parse(line, t)?;
        msg.apply(data);
        Some(msg.class)
    }

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
    fn test_partial_sky_keeps_dops_and_satellites() {
        let mut data = GPSData::default();
        apply_line(
            r#"{"class":"SKY","hdop":0.8,"pdop":1.5,"satellites":[{"PRN":1,"used":true}]}"#,
            &mut data,
            1.0,
        );
        // Satellites-only partial SKY: DOPs must survive
        apply_line(
            r#"{"class":"SKY","satellites":[{"PRN":2,"used":true},{"PRN":3}]}"#,
            &mut data,
            2.0,
        );
        assert!((data.dop.hdop - 0.8).abs() < 1e-10);
        assert!((data.dop.pdop - 1.5).abs() < 1e-10);
        assert_eq!(data.satellites.len(), 2);
        // DOP-only partial SKY: satellites must survive, present DOP updated
        apply_line(r#"{"class":"SKY","hdop":1.1}"#, &mut data, 3.0);
        assert!((data.dop.hdop - 1.1).abs() < 1e-10);
        assert!((data.dop.pdop - 1.5).abs() < 1e-10);
        assert_eq!(data.satellites.len(), 2);
        assert_eq!(data.satellites_used, 1);
    }

    #[test]
    fn test_empty_satellites_clears_list() {
        let mut data = GPSData::default();
        apply_line(
            r#"{"class":"SKY","satellites":[{"PRN":1,"used":true}]}"#,
            &mut data,
            1.0,
        );
        assert_eq!(data.satellites.len(), 1);
        apply_line(r#"{"class":"SKY","satellites":[]}"#, &mut data, 2.0);
        assert!(data.satellites.is_empty());
        assert_eq!(data.satellites_used, 0);
    }

    #[test]
    fn test_non_object_satellite_entries_skipped() {
        let mut data = GPSData::default();
        apply_line(
            r#"{"class":"SKY","satellites":[1,"x",null,[],{"PRN":9,"used":true}]}"#,
            &mut data,
            1.0,
        );
        assert_eq!(data.satellites.len(), 1);
        assert_eq!(data.satellites[0].prn, 9);
        assert_eq!(data.satellites_used, 1);
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

        // Partial DEVICE notification keeps earlier fields
        apply_line(
            r#"{"class":"DEVICE","path":"/dev/ttyACM0","activated":"2024-01-01T00:00:00Z"}"#,
            &mut data,
            1.0,
        );
        assert_eq!(data.device.bps, 9600);
        assert!((data.device.cycle - 1.0).abs() < 1e-10);
        assert_eq!(data.device.activated, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn test_out_of_range_integers_are_not_truncated() {
        let mut data = GPSData::default();
        // 259 = 256 + 3, 4294967314 = 2^32 + 18: used to wrap to a 3D fix
        // with 18 leap seconds.
        apply_line(
            r#"{"class":"TPV","mode":259,"status":257,"lat":1.0,"lon":2.0,"leapseconds":4294967314}"#,
            &mut data,
            1.0,
        );
        assert_eq!(data.mode, FixMode::Unknown);
        assert_eq!(data.status, FixStatus::Unknown);
        assert_eq!(data.leapseconds, 0);
        assert!(!data.has_fix());

        apply_line(
            r#"{"class":"SKY","satellites":[{"PRN":70000,"svid":65537,"gnssid":300,"sigid":-1,"health":999,"freqid":200}]}"#,
            &mut data,
            2.0,
        );
        let s = &data.satellites[0];
        assert_eq!(
            (s.prn, s.svid, s.gnssid, s.sigid, s.health),
            (0, 0, 0, 0, 0)
        );
        assert_eq!(s.freqid, None);

        // -(2^32) - 20 used to wrap to -20
        apply_line(r#"{"class":"PPS","precision":-4294967316}"#, &mut data, 3.0);
        assert_eq!(data.pps.precision, 0);

        // Out-of-range DEVICE fields keep the previous value
        apply_line(
            r#"{"class":"DEVICE","bps":9600,"native":1}"#,
            &mut data,
            4.0,
        );
        apply_line(
            r#"{"class":"DEVICE","bps":4294967396,"native":300}"#,
            &mut data,
            5.0,
        );
        assert_eq!(data.device.bps, 9600);
        assert_eq!(data.device.native, 1);

        apply_line(
            r#"{"class":"VERSION","proto_major":4294967299,"proto_minor":-1}"#,
            &mut data,
            6.0,
        );
        assert_eq!(data.version.proto_major, 0);
        assert_eq!(data.version.proto_minor, 0);
    }

    #[test]
    fn test_parse_invalid_json_is_none() {
        assert!(GpsdMessage::parse("not valid json", 0.0).is_none());
        assert!(GpsdMessage::parse(r#"{"no_class": 1}"#, 0.0).is_none());
    }

    #[test]
    fn test_parse_unknown_class_is_none() {
        assert!(GpsdMessage::parse(r#"{"class": "UNKNOWN"}"#, 100.0).is_none());
    }

    #[test]
    fn test_parse_tags_class() {
        let cases = [
            (r#"{"class":"TPV","mode":1}"#, MsgClass::Tpv),
            (r#"{"class":"SKY"}"#, MsgClass::Sky),
            (r#"{"class":"PPS"}"#, MsgClass::Pps),
            (r#"{"class":"TOFF"}"#, MsgClass::Toff),
            (r#"{"class":"DEVICES","devices":[]}"#, MsgClass::Devices),
            (r#"{"class":"DEVICE"}"#, MsgClass::Device),
            (r#"{"class":"VERSION"}"#, MsgClass::Version),
            (r#"{"class":"WATCH","enable":true}"#, MsgClass::Watch),
            (r#"{"class":"ERROR","message":"x"}"#, MsgClass::Error),
        ];
        for (line, class) in cases {
            assert_eq!(
                GpsdMessage::parse(line, 0.0).unwrap().class,
                class,
                "{line}"
            );
        }
    }

    #[test]
    fn test_tpv_sets_last_tpv_others_only_last_seen() {
        let mut data = GPSData::default();
        apply_line(r#"{"class":"TPV","mode":3}"#, &mut data, 10.0);
        assert_eq!(data.last_tpv, 10.0);
        assert_eq!(data.last_seen, 10.0);
        apply_line(r#"{"class":"PPS"}"#, &mut data, 12.0);
        assert_eq!(data.last_tpv, 10.0);
        assert_eq!(data.last_seen, 12.0);
    }

    #[test]
    fn test_classify_line() {
        assert!(matches!(
            classify_line("$GPGGA,1,2,3*00\r\n", 0.0),
            Some(GpsdEvent::Nmea(s)) if s == "$GPGGA,1,2,3*00"
        ));
        assert!(matches!(
            classify_line("!AIVDM,x", 0.0),
            Some(GpsdEvent::Nmea(_))
        ));
        assert!(classify_line("   ", 0.0).is_none());
        assert!(classify_line("garbage", 0.0).is_none());
        assert!(classify_line(r#"{"class":"FOO"}"#, 0.0).is_none());
        assert!(matches!(
            classify_line(r#"{"class":"TPV"}"#, 0.0),
            Some(GpsdEvent::Message(m)) if m.class == MsgClass::Tpv
        ));
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

    // === Client loop against a mock gpsd ===

    async fn recv(rx: &mut mpsc::Receiver<TaggedEvent>) -> TaggedEvent {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for event")
            .expect("channel closed")
    }

    #[tokio::test]
    async fn test_client_against_mock_gpsd() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, mut rx) = mpsc::channel(16);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(gpsd_task("127.0.0.1".to_string(), port, 7, tx, shutdown_rx));

        let (mut sock, _) = listener.accept().await.unwrap();
        let mut watch = vec![0u8; 256];
        let n = sock.read(&mut watch).await.unwrap();
        assert!(String::from_utf8_lossy(&watch[..n]).starts_with("?WATCH="));

        let mut payload = Vec::new();
        payload.extend_from_slice(b"{\"class\":\"VERSION\",\"release\":\"3.\xff\"}\n");
        payload.extend_from_slice(&vec![b'z'; MAX_LINE_LEN + 10]);
        payload.extend_from_slice(b"\n{\"class\":\"BOGUS\"}\nnot json\n");
        payload.extend_from_slice(b"$GPGGA,123\r\n");
        payload.extend_from_slice(
            b"{\"class\":\"TPV\",\"mode\":3,\"time\":\"2024-01-15T12:30:00Z\"}\n",
        );
        sock.write_all(&payload).await.unwrap();
        drop(sock);

        assert!(matches!(recv(&mut rx).await, (7, GpsdEvent::Connected)));
        // Invalid UTF-8 is replaced, not a disconnect
        match recv(&mut rx).await {
            (7, GpsdEvent::Message(m)) => {
                assert_eq!(m.class, MsgClass::Version);
                assert!(m.json["release"].as_str().unwrap().starts_with("3."));
            }
            other => panic!("unexpected {other:?}"),
        }
        // Oversized line, unknown class and invalid JSON produce nothing
        assert!(matches!(recv(&mut rx).await, (7, GpsdEvent::Nmea(s)) if s == "$GPGGA,123"));
        match recv(&mut rx).await {
            (7, GpsdEvent::Message(m)) => assert_eq!(m.class, MsgClass::Tpv),
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(recv(&mut rx).await, (7, GpsdEvent::Error(e)) if e.contains("closed")));

        // Clean shutdown during the reconnect delay
        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("task did not stop")
            .unwrap();
    }
}
