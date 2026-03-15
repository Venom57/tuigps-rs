use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FixMode {
    Unknown = 0,
    NoFix = 1,
    Fix2D = 2,
    Fix3D = 3,
}

impl From<u8> for FixMode {
    fn from(v: u8) -> Self {
        match v {
            1 => FixMode::NoFix,
            2 => FixMode::Fix2D,
            3 => FixMode::Fix3D,
            _ => FixMode::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FixStatus {
    Unknown = 0,
    Gps = 1,
    Dgps = 2,
    RtkFix = 3,
    RtkFloat = 4,
    Dr = 5,
    GnssDr = 6,
    TimeOnly = 7,
    Simulated = 8,
    PpsFix = 9,
}

impl From<u8> for FixStatus {
    fn from(v: u8) -> Self {
        match v {
            1 => FixStatus::Gps,
            2 => FixStatus::Dgps,
            3 => FixStatus::RtkFix,
            4 => FixStatus::RtkFloat,
            5 => FixStatus::Dr,
            6 => FixStatus::GnssDr,
            7 => FixStatus::TimeOnly,
            8 => FixStatus::Simulated,
            9 => FixStatus::PpsFix,
            _ => FixStatus::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SatelliteInfo {
    pub prn: u16,
    pub gnssid: u8,
    pub svid: u16,
    pub elevation: f64,
    pub azimuth: f64,
    pub snr: f64,
    pub used: bool,
    pub sigid: u8,
    pub health: u8,
    pub freqid: Option<i8>,
}

#[derive(Debug, Clone)]
pub struct DOPValues {
    pub hdop: f64,
    pub vdop: f64,
    pub pdop: f64,
    pub gdop: f64,
    pub tdop: f64,
    pub xdop: f64,
    pub ydop: f64,
}

impl Default for DOPValues {
    fn default() -> Self {
        Self {
            hdop: f64::NAN,
            vdop: f64::NAN,
            pdop: f64::NAN,
            gdop: f64::NAN,
            tdop: f64::NAN,
            xdop: f64::NAN,
            ydop: f64::NAN,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ErrorEstimates {
    pub eph: f64,
    pub epv: f64,
    pub ept: f64,
    pub eps: f64,
    pub epd: f64,
    pub epc: f64,
    pub epx: f64,
    pub epy: f64,
    pub sep: f64,
}

impl Default for ErrorEstimates {
    fn default() -> Self {
        Self {
            eph: f64::NAN,
            epv: f64::NAN,
            ept: f64::NAN,
            eps: f64::NAN,
            epd: f64::NAN,
            epc: f64::NAN,
            epx: f64::NAN,
            epy: f64::NAN,
            sep: f64::NAN,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PPSData {
    pub real_sec: i64,
    pub real_nsec: i64,
    pub clock_sec: i64,
    pub clock_nsec: i64,
    pub precision: i32,
    pub qerr: i64,
}

#[derive(Debug, Clone, Default)]
pub struct TOFFData {
    pub real_sec: i64,
    pub real_nsec: i64,
    pub clock_sec: i64,
    pub clock_nsec: i64,
}

#[derive(Debug, Clone, Default)]
pub struct DeviceInfo {
    pub path: String,
    pub driver: String,
    pub subtype: String,
    pub bps: u32,
    pub cycle: f64,
    pub mincycle: f64,
    pub activated: String,
    pub native: u8,
}

#[derive(Debug, Clone, Default)]
pub struct VersionInfo {
    pub release: String,
    pub proto_major: u32,
    pub proto_minor: u32,
}

#[derive(Debug, Clone)]
pub struct GPSData {
    // Connection state
    pub connected: bool,
    pub last_seen: f64,
    pub error_message: String,

    // TPV
    pub latitude: f64,
    pub longitude: f64,
    pub alt_hae: f64,
    pub alt_msl: f64,
    pub geoid_sep: f64,
    pub speed: f64,
    pub track: f64,
    pub climb: f64,
    pub time: String,
    pub leapseconds: i32,
    pub mode: FixMode,
    pub status: FixStatus,
    pub magtrack: f64,
    pub magvar: f64,

    // ECEF
    pub ecefx: f64,
    pub ecefy: f64,
    pub ecefz: f64,
    pub ecefvx: f64,
    pub ecefvy: f64,
    pub ecefvz: f64,

    // Composites
    pub dop: DOPValues,
    pub errors: ErrorEstimates,
    pub pps: PPSData,
    pub toff: TOFFData,
    pub device: DeviceInfo,
    pub version: VersionInfo,

    // TOFF accumulation
    pub toff_samples: Vec<f64>,
    pub toff_armed_offset: f64,
    pub toff_armed_gps_time: String,
    pub toff_armed_sys_time: f64,

    // Satellites
    pub satellites: Vec<SatelliteInfo>,
    pub satellites_used: u32,
}

impl Default for GPSData {
    fn default() -> Self {
        Self {
            connected: false,
            last_seen: 0.0,
            error_message: String::new(),

            latitude: f64::NAN,
            longitude: f64::NAN,
            alt_hae: f64::NAN,
            alt_msl: f64::NAN,
            geoid_sep: f64::NAN,
            speed: f64::NAN,
            track: f64::NAN,
            climb: f64::NAN,
            time: String::new(),
            leapseconds: 0,
            mode: FixMode::Unknown,
            status: FixStatus::Unknown,
            magtrack: f64::NAN,
            magvar: f64::NAN,

            ecefx: f64::NAN,
            ecefy: f64::NAN,
            ecefz: f64::NAN,
            ecefvx: f64::NAN,
            ecefvy: f64::NAN,
            ecefvz: f64::NAN,

            dop: DOPValues::default(),
            errors: ErrorEstimates::default(),
            pps: PPSData::default(),
            toff: TOFFData::default(),
            device: DeviceInfo::default(),
            version: VersionInfo::default(),

            toff_samples: Vec::new(),
            toff_armed_offset: f64::NAN,
            toff_armed_gps_time: String::new(),
            toff_armed_sys_time: f64::NAN,

            satellites: Vec::new(),
            satellites_used: 0,
        }
    }
}

impl GPSData {
    /// Returns (visible, used) counts per gnssid
    pub fn constellation_counts(&self) -> HashMap<u8, (u32, u32)> {
        let mut counts: HashMap<u8, (u32, u32)> = HashMap::new();
        for sat in &self.satellites {
            let entry = counts.entry(sat.gnssid).or_insert((0, 0));
            entry.0 += 1;
            if sat.used {
                entry.1 += 1;
            }
        }
        counts
    }

    pub fn has_fix(&self) -> bool {
        matches!(self.mode, FixMode::Fix2D | FixMode::Fix3D)
            && self.latitude.is_finite()
            && self.longitude.is_finite()
    }

    /// PPS offset in microseconds
    pub fn pps_offset_us(&self) -> f64 {
        let offset_sec =
            (self.pps.real_sec - self.pps.clock_sec) as f64
            + (self.pps.real_nsec - self.pps.clock_nsec) as f64 / 1e9;
        offset_sec * 1e6
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_gpsdata_nan_fields() {
        let d = GPSData::default();
        assert!(d.latitude.is_nan());
        assert!(d.longitude.is_nan());
        assert!(d.alt_hae.is_nan());
        assert!(d.speed.is_nan());
        assert!(d.dop.hdop.is_nan());
        assert!(d.errors.eph.is_nan());
    }

    #[test]
    fn test_default_gpsdata_enums() {
        let d = GPSData::default();
        assert_eq!(d.mode, FixMode::Unknown);
        assert_eq!(d.status, FixStatus::Unknown);
    }

    #[test]
    fn test_fixmode_from_u8() {
        assert_eq!(FixMode::from(0), FixMode::Unknown);
        assert_eq!(FixMode::from(1), FixMode::NoFix);
        assert_eq!(FixMode::from(2), FixMode::Fix2D);
        assert_eq!(FixMode::from(3), FixMode::Fix3D);
        assert_eq!(FixMode::from(255), FixMode::Unknown);
    }

    #[test]
    fn test_fixstatus_from_u8() {
        assert_eq!(FixStatus::from(1), FixStatus::Gps);
        assert_eq!(FixStatus::from(3), FixStatus::RtkFix);
        assert_eq!(FixStatus::from(9), FixStatus::PpsFix);
        assert_eq!(FixStatus::from(99), FixStatus::Unknown);
    }

    #[test]
    fn test_has_fix_3d() {
        let mut d = GPSData::default();
        d.mode = FixMode::Fix3D;
        d.latitude = 51.5;
        d.longitude = -0.1;
        assert!(d.has_fix());
    }

    #[test]
    fn test_has_fix_2d() {
        let mut d = GPSData::default();
        d.mode = FixMode::Fix2D;
        d.latitude = 51.5;
        d.longitude = -0.1;
        assert!(d.has_fix());
    }

    #[test]
    fn test_has_fix_nofix() {
        let mut d = GPSData::default();
        d.mode = FixMode::NoFix;
        d.latitude = 51.5;
        d.longitude = -0.1;
        assert!(!d.has_fix());
    }

    #[test]
    fn test_has_fix_nan_coords() {
        let mut d = GPSData::default();
        d.mode = FixMode::Fix3D;
        // lat/lon are NaN by default
        assert!(!d.has_fix());
    }

    #[test]
    fn test_constellation_counts() {
        let mut d = GPSData::default();
        d.satellites = vec![
            SatelliteInfo { prn: 1, gnssid: 0, svid: 1, elevation: 45.0, azimuth: 180.0, snr: 35.0, used: true, sigid: 0, health: 1, freqid: None },
            SatelliteInfo { prn: 2, gnssid: 0, svid: 2, elevation: 30.0, azimuth: 90.0, snr: 25.0, used: false, sigid: 0, health: 1, freqid: None },
            SatelliteInfo { prn: 65, gnssid: 2, svid: 1, elevation: 60.0, azimuth: 270.0, snr: 40.0, used: true, sigid: 0, health: 1, freqid: None },
        ];
        let counts = d.constellation_counts();
        assert_eq!(counts[&0], (2, 1)); // GPS: 2 visible, 1 used
        assert_eq!(counts[&2], (1, 1)); // Galileo: 1 visible, 1 used
    }

    #[test]
    fn test_pps_offset_us() {
        let mut d = GPSData::default();
        // 1 second difference
        d.pps.real_sec = 1001;
        d.pps.real_nsec = 0;
        d.pps.clock_sec = 1000;
        d.pps.clock_nsec = 0;
        let offset = d.pps_offset_us();
        assert!((offset - 1_000_000.0).abs() < 1e-6); // 1s = 1_000_000 us

        // Sub-microsecond difference
        d.pps.real_sec = 1000;
        d.pps.real_nsec = 500;
        d.pps.clock_sec = 1000;
        d.pps.clock_nsec = 0;
        let offset = d.pps_offset_us();
        assert!((offset - 0.5).abs() < 1e-6); // 500ns = 0.5us
    }
}
