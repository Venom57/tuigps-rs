//! Track logging to GPX 1.1 or CSV files.
//!
//! Integrity guarantees:
//! - Files are opened with `create_new`, so an existing file is never
//!   truncated; on a name collision a numeric suffix (`_1`, `_2`, ...) is
//!   appended before the extension.
//! - The writer is flushed after every point, so a crash or `SIGKILL` loses at
//!   most the point being written. (A GPX file killed mid-run lacks its
//!   closing tags, but every complete `<trkpt>` is on disk.)
//! - [`GpsLogger::stop`] is idempotent, and dropping an active logger
//!   finalizes the file (closing GPX tags + flush).

use anyhow::{Context, Result, bail};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::data_model::{FixMode, FixStatus, GPSData};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Gpx,
    Csv,
}

impl LogFormat {
    fn extension(self) -> &'static str {
        match self {
            LogFormat::Gpx => "gpx",
            LogFormat::Csv => "csv",
        }
    }
}

/// CSV header row. Kept stable so downstream tooling can rely on it.
///
/// Columns:
/// - `time`: gpsd TPV time, ISO 8601 UTC (e.g. `2024-01-15T12:30:00.000Z`)
/// - `latitude`, `longitude`: WGS84 decimal degrees
/// - `alt_msl`: altitude above mean sea level, meters
/// - `speed`: ground speed, m/s
/// - `track`: course over ground, degrees true
/// - `climb`: vertical speed, m/s
///
/// Values gpsd did not report (NaN/inf internally) are written as empty
/// fields, never as `NaN`.
pub const CSV_HEADER: &str = "time,latitude,longitude,alt_msl,speed,track,climb";

/// Highest numeric suffix tried before giving up on finding a free filename.
const MAX_COLLISION_SUFFIX: u32 = 9999;

pub struct GpsLogger {
    file: Option<BufWriter<File>>,
    format: LogFormat,
    dir: PathBuf,
    pub point_count: u32,
    last_time: String,
    pub active: bool,
    pub filename: String,
}

impl GpsLogger {
    pub fn new(format: LogFormat, dir: impl Into<PathBuf>) -> Self {
        Self {
            file: None,
            format,
            dir: dir.into(),
            point_count: 0,
            last_time: String::new(),
            active: false,
            filename: String::new(),
        }
    }

    /// Output format of this logger.
    pub fn format(&self) -> LogFormat {
        self.format
    }

    /// Open a new, uniquely named log file in the output directory and write
    /// its header. If a log is already open it is finalized first.
    pub fn start(&mut self) -> Result<()> {
        if self.active {
            self.stop()?;
        }
        let stem = format!("tuigps_{}", chrono::Local::now().format("%Y%m%d_%H%M%S"));
        let (path, file) = create_unique(&self.dir, &stem, self.format.extension())?;
        let mut file = BufWriter::new(file);
        match self.format {
            LogFormat::Gpx => write_gpx_header(&mut file)?,
            LogFormat::Csv => writeln!(file, "{CSV_HEADER}")?,
        }
        file.flush()?;
        self.file = Some(file);
        self.filename = path.to_string_lossy().into_owned();
        self.active = true;
        self.point_count = 0;
        self.last_time.clear();
        Ok(())
    }

    /// Finalize and close the log file. Safe to call more than once; calls
    /// after the first are no-ops.
    pub fn stop(&mut self) -> Result<()> {
        self.active = false;
        let Some(mut file) = self.file.take() else {
            return Ok(());
        };
        if self.format == LogFormat::Gpx {
            writeln!(file, "</trkseg></trk>")?;
            writeln!(file, "</gpx>")?;
        }
        file.flush()?;
        file.get_ref().sync_all().ok();
        Ok(())
    }

    /// Append one point. Points are skipped when there is no 2D/3D fix, when
    /// the TPV time is empty, or when the time equals the previously logged
    /// point (gpsd sends many non-TPV updates per epoch).
    pub fn log_point(&mut self, data: &GPSData) -> Result<()> {
        if !self.active || !data.has_fix() {
            return Ok(());
        }
        if data.time.is_empty() || data.time == self.last_time {
            return Ok(());
        }
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        match self.format {
            LogFormat::Gpx => write_gpx_point(file, data)?,
            LogFormat::Csv => write_csv_point(file, data)?,
        }
        file.flush()?;
        self.last_time.clone_from(&data.time);
        self.point_count += 1;
        Ok(())
    }
}

impl Drop for GpsLogger {
    fn drop(&mut self) {
        // Best effort: there is nowhere to report an error from Drop.
        let _ = self.stop();
    }
}

/// Create `<dir>/<stem>.<ext>`, or `<stem>_N.<ext>` for the smallest N >= 1
/// that does not exist yet. Never truncates an existing file.
fn create_unique(dir: &Path, stem: &str, ext: &str) -> Result<(PathBuf, File)> {
    for n in 0..=MAX_COLLISION_SUFFIX {
        let name = if n == 0 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem}_{n}.{ext}")
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(e).with_context(|| format!("creating log file {}", path.display()));
            }
        }
    }
    bail!(
        "no free log filename for {stem}.{ext} in {} after {MAX_COLLISION_SUFFIX} attempts",
        dir.display()
    )
}

fn write_gpx_header(w: &mut impl Write) -> std::io::Result<()> {
    writeln!(w, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
    writeln!(
        w,
        concat!(
            r#"<gpx version="1.1" creator="tuigps-rs" "#,
            r#"xmlns="http://www.topografix.com/GPX/1/1" "#,
            r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" "#,
            r#"xsi:schemaLocation="http://www.topografix.com/GPX/1/1 "#,
            r#"http://www.topografix.com/GPX/1/1/gpx.xsd">"#
        )
    )?;
    writeln!(w, "<trk><name>tuigps-rs track</name><trkseg>")
}

/// Write one `<trkpt>`. Child elements follow the GPX 1.1 `wptType` sequence
/// order: ele, time, ..., fix, sat, hdop, vdop, pdop.
fn write_gpx_point(w: &mut impl Write, data: &GPSData) -> std::io::Result<()> {
    writeln!(
        w,
        r#"<trkpt lat="{}" lon="{}">"#,
        data.latitude, data.longitude
    )?;
    if data.alt_msl.is_finite() {
        writeln!(w, "  <ele>{}</ele>", data.alt_msl)?;
    }
    writeln!(w, "  <time>{}</time>", xml_escape(&data.time))?;
    if let Some(fix) = gpx_fix(data) {
        writeln!(w, "  <fix>{fix}</fix>")?;
    }
    if data.satellites_used > 0 {
        writeln!(w, "  <sat>{}</sat>", data.satellites_used)?;
    }
    for (tag, v) in [
        ("hdop", data.dop.hdop),
        ("vdop", data.dop.vdop),
        ("pdop", data.dop.pdop),
    ] {
        if v.is_finite() && v >= 0.0 {
            writeln!(w, "  <{tag}>{v}</{tag}>")?;
        }
    }
    writeln!(w, "</trkpt>")
}

/// Map gpsd mode/status to the GPX `fixType` enumeration.
fn gpx_fix(data: &GPSData) -> Option<&'static str> {
    match (data.mode, data.status) {
        (
            FixMode::Fix2D | FixMode::Fix3D,
            FixStatus::Dgps | FixStatus::RtkFix | FixStatus::RtkFloat,
        ) => Some("dgps"),
        (FixMode::Fix3D, _) => Some("3d"),
        (FixMode::Fix2D, _) => Some("2d"),
        _ => None,
    }
}

fn write_csv_point(w: &mut impl Write, data: &GPSData) -> std::io::Result<()> {
    writeln!(
        w,
        "{},{},{},{},{},{},{}",
        csv_escape(&data.time),
        csv_num(data.latitude),
        csv_num(data.longitude),
        csv_num(data.alt_msl),
        csv_num(data.speed),
        csv_num(data.track),
        csv_num(data.climb),
    )
}

/// Format a number for CSV; non-finite values become an empty field.
fn csv_num(v: f64) -> String {
    if v.is_finite() {
        v.to_string()
    } else {
        String::new()
    }
}

/// Quote a CSV field (RFC 4180) if it contains a delimiter, quote or newline.
fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Escape the five XML special characters for use in text or attributes.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escape_specials() {
        assert_eq!(xml_escape(r#"a<b>&"c'"#), "a&lt;b&gt;&amp;&quot;c&apos;");
        assert_eq!(xml_escape("2024-01-15T12:30:00Z"), "2024-01-15T12:30:00Z");
    }

    #[test]
    fn create_unique_adds_suffix_and_never_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let (p0, _) = create_unique(dir.path(), "x", "gpx").unwrap();
        std::fs::write(&p0, "keep").unwrap();
        let (p1, _) = create_unique(dir.path(), "x", "gpx").unwrap();
        let (p2, _) = create_unique(dir.path(), "x", "gpx").unwrap();
        assert_eq!(p0.file_name().unwrap(), "x.gpx");
        assert_eq!(p1.file_name().unwrap(), "x_1.gpx");
        assert_eq!(p2.file_name().unwrap(), "x_2.gpx");
        assert_eq!(std::fs::read_to_string(&p0).unwrap(), "keep");
    }

    #[test]
    fn create_unique_reports_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert!(create_unique(&missing, "x", "csv").is_err());
    }

    #[test]
    fn csv_helpers() {
        assert_eq!(csv_num(f64::NAN), "");
        assert_eq!(csv_num(f64::INFINITY), "");
        assert_eq!(csv_num(1.5), "1.5");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_escape("plain"), "plain");
    }
}
