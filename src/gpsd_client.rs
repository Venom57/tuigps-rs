use anyhow::{anyhow, Result};
use serde_json::Value;
use std::time::{Duration, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::data_model::*;

#[derive(Debug, Clone)]
pub enum GpsdEvent {
    Update(GPSData),
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
    let watch =
        r#"?WATCH={"enable":true,"json":true,"pps":true,"timing":true,"nmea":true}"#;
    writer.write_all(watch.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut data = GPSData::default();
    data.connected = true;

    loop {
        line.clear();
        tokio::select! {
            result = reader.read_line(&mut line) => {
                let n = result?;
                if n == 0 {
                    return Err(anyhow!("gpsd connection closed"));
                }

                let receipt_time = std::time::SystemTime::now()
                    .duration_since(UNIX_EPOCH)?
                    .as_secs_f64();

                // Check for NMEA
                let trimmed = line.trim();
                if trimmed.starts_with('$') || trimmed.starts_with('!') {
                    let _ = tx.send(GpsdEvent::Nmea(trimmed.to_string())).await;
                } else {
                    process_message(trimmed, &mut data, receipt_time);
                    let _ = tx.send(GpsdEvent::Update(data.clone())).await;
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
        "TPV" => extract_tpv(&msg, data, receipt_time),
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

fn json_f64(msg: &Value, key: &str) -> f64 {
    msg[key].as_f64().unwrap_or(f64::NAN)
}

fn extract_tpv(msg: &Value, data: &mut GPSData, receipt_time: f64) {
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

    // TOFF computation from GPS time vs receipt_time
    if let Some(time_str) = msg["time"].as_str() {
        data.time = time_str.to_string();
        if let Ok(gps_time) = chrono::DateTime::parse_from_rfc3339(time_str) {
            let gps_epoch =
                gps_time.timestamp() as f64 + gps_time.timestamp_subsec_nanos() as f64 / 1e9;
            let offset = gps_epoch - receipt_time;

            // Accumulate in circular buffer (max 20)
            if data.toff_samples.len() >= 20 {
                data.toff_samples.remove(0);
            }
            data.toff_samples.push(offset);
        }
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

    if let Some(sats) = msg["satellites"].as_array() {
        if !sats.is_empty() {
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
    if let Some(devices) = msg["devices"].as_array() {
        if let Some(dev) = devices.first() {
            extract_device_fields(dev, data);
        }
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
