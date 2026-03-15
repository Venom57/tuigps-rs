use anyhow::Result;
use std::fs::File;
use std::io::{BufWriter, Write};

use crate::data_model::GPSData;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogFormat {
    Gpx,
    Csv,
}

pub struct GpsLogger {
    file: Option<BufWriter<File>>,
    format: LogFormat,
    pub point_count: u32,
    last_time: String,
    pub active: bool,
    pub filename: String,
}

impl GpsLogger {
    pub fn new(format: LogFormat) -> Self {
        Self {
            file: None,
            format,
            point_count: 0,
            last_time: String::new(),
            active: false,
            filename: String::new(),
        }
    }

    pub fn start(&mut self) -> Result<()> {
        match self.format {
            LogFormat::Gpx => self.start_gpx(),
            LogFormat::Csv => self.start_csv(),
        }
    }

    pub fn stop(&mut self) -> Result<()> {
        match self.format {
            LogFormat::Gpx => self.stop_gpx(),
            LogFormat::Csv => self.stop_csv(),
        }
    }

    pub fn log_point(&mut self, data: &GPSData) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        match self.format {
            LogFormat::Gpx => self.log_point_gpx(data),
            LogFormat::Csv => self.log_point_csv(data),
        }
    }

    fn start_gpx(&mut self) -> Result<()> {
        let filename = format!(
            "tuigps_{}.gpx",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );
        let mut file = BufWriter::new(File::create(&filename)?);
        writeln!(file, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
        writeln!(file, r#"<gpx version="1.0">"#)?;
        writeln!(file, r#"<trk><trkseg>"#)?;
        self.file = Some(file);
        self.filename = filename;
        self.active = true;
        self.point_count = 0;
        Ok(())
    }

    fn start_csv(&mut self) -> Result<()> {
        let filename = format!(
            "tuigps_{}.csv",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );
        let mut file = BufWriter::new(File::create(&filename)?);
        writeln!(file, "time,latitude,longitude,alt_msl,speed,track,climb")?;
        self.file = Some(file);
        self.filename = filename;
        self.active = true;
        self.point_count = 0;
        Ok(())
    }

    fn log_point_gpx(&mut self, data: &GPSData) -> Result<()> {
        if !data.has_fix() {
            return Ok(());
        }
        if data.time == self.last_time {
            return Ok(());
        }
        self.last_time = data.time.clone();

        if let Some(ref mut file) = self.file {
            writeln!(
                file,
                r#"<trkpt lat="{}" lon="{}">"#,
                data.latitude, data.longitude
            )?;
            if data.alt_msl.is_finite() {
                writeln!(file, "<ele>{}</ele>", data.alt_msl)?;
            }
            writeln!(file, "<time>{}</time>", data.time)?;
            writeln!(file, "</trkpt>")?;
            self.point_count += 1;
        }
        Ok(())
    }

    fn log_point_csv(&mut self, data: &GPSData) -> Result<()> {
        if !data.has_fix() {
            return Ok(());
        }
        if data.time == self.last_time {
            return Ok(());
        }
        self.last_time = data.time.clone();

        if let Some(ref mut file) = self.file {
            writeln!(
                file,
                "{},{},{},{},{},{},{}",
                data.time, data.latitude, data.longitude, data.alt_msl, data.speed, data.track,
                data.climb
            )?;
            self.point_count += 1;
        }
        Ok(())
    }

    fn stop_gpx(&mut self) -> Result<()> {
        if let Some(ref mut file) = self.file {
            writeln!(file, "</trkseg></trk></gpx>")?;
            file.flush()?;
        }
        self.file = None;
        self.active = false;
        Ok(())
    }

    fn stop_csv(&mut self) -> Result<()> {
        if let Some(ref mut file) = self.file {
            file.flush()?;
        }
        self.file = None;
        self.active = false;
        Ok(())
    }
}
