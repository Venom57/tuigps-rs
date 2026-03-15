use std::time::{Duration, Instant};

pub struct PositionHold {
    count: u64,
    mean_lat: f64,
    mean_lon: f64,
    mean_alt: f64,
    m2_lat: f64,
    m2_lon: f64,
    m2_alt: f64,
    start_time: Instant,
}

pub struct HoldResult {
    pub mean_lat: f64,
    pub mean_lon: f64,
    pub mean_alt: f64,
    pub std_lat: f64,
    pub std_lon: f64,
    pub std_alt: f64,
    pub cep50: f64,
    pub cep95: f64,
    pub count: u64,
    pub duration: Duration,
}

impl PositionHold {
    pub fn new() -> Self {
        Self {
            count: 0,
            mean_lat: 0.0,
            mean_lon: 0.0,
            mean_alt: 0.0,
            m2_lat: 0.0,
            m2_lon: 0.0,
            m2_alt: 0.0,
            start_time: Instant::now(),
        }
    }

    pub fn add_fix(&mut self, lat: f64, lon: f64, alt: f64) {
        if !lat.is_finite() || !lon.is_finite() {
            return;
        }

        self.count += 1;
        let n = self.count as f64;

        let delta_lat = lat - self.mean_lat;
        self.mean_lat += delta_lat / n;
        self.m2_lat += delta_lat * (lat - self.mean_lat);

        let delta_lon = lon - self.mean_lon;
        self.mean_lon += delta_lon / n;
        self.m2_lon += delta_lon * (lon - self.mean_lon);

        if alt.is_finite() {
            let delta_alt = alt - self.mean_alt;
            self.mean_alt += delta_alt / n;
            self.m2_alt += delta_alt * (alt - self.mean_alt);
        }
    }

    pub fn result(&self) -> Option<HoldResult> {
        if self.count < 2 {
            return None;
        }
        let n = self.count as f64;
        let std_lat = (self.m2_lat / (n - 1.0)).sqrt();
        let std_lon = (self.m2_lon / (n - 1.0)).sqrt();
        let std_alt = (self.m2_alt / (n - 1.0)).sqrt();

        // Convert to meters
        const M_PER_DEG_LAT: f64 = 110540.0;
        let m_per_deg_lon = 111320.0 * self.mean_lat.to_radians().cos();

        let std_north = std_lat * M_PER_DEG_LAT;
        let std_east = std_lon * m_per_deg_lon;

        let cep50 = 0.5887 * (std_north + std_east);
        let cep95 = 2.146 * cep50;

        Some(HoldResult {
            mean_lat: self.mean_lat,
            mean_lon: self.mean_lon,
            mean_alt: self.mean_alt,
            std_lat,
            std_lon,
            std_alt,
            cep50,
            cep95,
            count: self.count,
            duration: self.start_time.elapsed(),
        })
    }
}
