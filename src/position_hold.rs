//! Position averaging ("hold") using Welford's online algorithm.
//!
//! Latitude/longitude and altitude are tracked with separate sample counts,
//! so fixes without a valid altitude still contribute to the horizontal mean
//! without corrupting the altitude statistics.
//!
//! Longitudes are unwrapped relative to the first accepted fix before being
//! accumulated, so averaging across the antimeridian (e.g. 179.9 and -179.9)
//! gives ~180, not ~0. This is exact as long as the scatter spans less than
//! 180 degrees of longitude, which is always true for a stationary receiver.
//!
//! # CEP
//!
//! CEP50/CEP95 are the radii (meters) of circles centred on the mean position
//! that contain 50% / 95% of the individual fixes, assuming the horizontal
//! scatter is bivariate normal. They are computed from the full 2x2
//! north/east sample covariance (including correlation) by numerically
//! integrating the bivariate normal over a disc, so they are exact for
//! elliptical as well as circular error distributions. For a circular
//! distribution with per-axis sigma `s` they reduce to the textbook values
//! `CEP50 = s * sqrt(2 ln 2) ~= 1.1774 s` and `CEP95 = s * sqrt(2 ln 20) ~=
//! 2.4477 s`.
//!
//! (An earlier version used `CEP50 ~= 0.5887 (s_n + s_e)` and
//! `CEP95 = 2.146 CEP50`. The first ignores N/E correlation and degrades for
//! elongated ellipses; the second overstates CEP95 by ~3% even in the
//! circular case, where the exact ratio is `sqrt(ln 20 / ln 2) ~= 2.0789`.)
//!
//! Degrees are converted to meters with the WGS84 meridional and prime
//! vertical radii of curvature at the mean latitude.

use std::f64::consts::PI;
use std::time::{Duration, Instant};

/// WGS84 semi-major axis, meters.
const WGS84_A: f64 = 6_378_137.0;
/// WGS84 first eccentricity squared.
const WGS84_E2: f64 = 6.694_379_990_141_32e-3;

pub struct PositionHold {
    /// Number of horizontal (lat/lon) samples.
    count: u64,
    /// Number of altitude samples (fixes with a finite altitude).
    count_alt: u64,
    mean_lat: f64,
    /// Running mean of longitude offsets from `lon_ref`, degrees.
    mean_dlon: f64,
    mean_alt: f64,
    m2_lat: f64,
    m2_lon: f64,
    m2_alt: f64,
    /// Co-moment of latitude and longitude offsets (for covariance).
    c_latlon: f64,
    /// Longitude of the first accepted fix; all longitudes are unwrapped
    /// relative to it.
    lon_ref: f64,
    start_time: Instant,
}

pub struct HoldResult {
    pub mean_lat: f64,
    /// Mean longitude, normalized to [-180, 180).
    pub mean_lon: f64,
    /// Mean altitude, or NaN if no fix had a finite altitude.
    pub mean_alt: f64,
    /// Sample standard deviation of latitude, degrees.
    pub std_lat: f64,
    /// Sample standard deviation of longitude, degrees.
    pub std_lon: f64,
    /// Sample standard deviation of altitude, meters (NaN if fewer than two
    /// altitude samples).
    pub std_alt: f64,
    /// Radius containing 50% of fixes, meters.
    pub cep50: f64,
    /// Radius containing 95% of fixes, meters.
    pub cep95: f64,
    /// Number of horizontal samples.
    pub count: u64,
    pub duration: Duration,
    /// Sample standard deviation northward, meters.
    pub std_north_m: f64,
    /// Sample standard deviation eastward, meters.
    pub std_east_m: f64,
    /// Number of samples that contributed to the altitude statistics.
    pub count_alt: u64,
}

impl Default for PositionHold {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrap an angle in degrees into [-180, 180).
fn wrap180(deg: f64) -> f64 {
    (deg + 180.0).rem_euclid(360.0) - 180.0
}

/// Meters per degree of latitude and longitude at `lat_deg` (WGS84).
pub fn meters_per_degree(lat_deg: f64) -> (f64, f64) {
    let phi = lat_deg.to_radians();
    let s2 = phi.sin().powi(2);
    let w = (1.0 - WGS84_E2 * s2).sqrt();
    let m = WGS84_A * (1.0 - WGS84_E2) / (w * w * w); // meridional radius
    let n = WGS84_A / w; // prime vertical radius
    let k = PI / 180.0;
    (m * k, n * phi.cos() * k)
}

impl PositionHold {
    pub fn new() -> Self {
        Self {
            count: 0,
            count_alt: 0,
            mean_lat: 0.0,
            mean_dlon: 0.0,
            mean_alt: 0.0,
            m2_lat: 0.0,
            m2_lon: 0.0,
            m2_alt: 0.0,
            c_latlon: 0.0,
            lon_ref: 0.0,
            start_time: Instant::now(),
        }
    }

    /// Number of lat/lon fixes accumulated so far.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Add one fix. Fixes with non-finite lat/lon are ignored entirely; a
    /// non-finite altitude only excludes the fix from altitude statistics.
    pub fn add_fix(&mut self, lat: f64, lon: f64, alt: f64) {
        if !lat.is_finite() || !lon.is_finite() {
            return;
        }

        if self.count == 0 {
            self.lon_ref = lon;
        }
        let dlon = wrap180(lon - self.lon_ref);

        self.count += 1;
        let n = self.count as f64;

        let delta_lat = lat - self.mean_lat;
        self.mean_lat += delta_lat / n;
        self.m2_lat += delta_lat * (lat - self.mean_lat);

        let delta_lon = dlon - self.mean_dlon;
        self.mean_dlon += delta_lon / n;
        self.m2_lon += delta_lon * (dlon - self.mean_dlon);

        // Co-moment: uses the pre-update lat delta and post-update lon mean.
        self.c_latlon += delta_lat * (dlon - self.mean_dlon);

        if alt.is_finite() {
            self.count_alt += 1;
            let na = self.count_alt as f64;
            let delta_alt = alt - self.mean_alt;
            self.mean_alt += delta_alt / na;
            self.m2_alt += delta_alt * (alt - self.mean_alt);
        }
    }

    pub fn result(&self) -> Option<HoldResult> {
        if self.count < 2 {
            return None;
        }
        let n = self.count as f64;
        let var_lat = self.m2_lat / (n - 1.0);
        let var_lon = self.m2_lon / (n - 1.0);
        let cov_latlon = self.c_latlon / (n - 1.0);
        let std_lat = var_lat.sqrt();
        let std_lon = var_lon.sqrt();

        let (mean_alt, std_alt) = match self.count_alt {
            0 => (f64::NAN, f64::NAN),
            1 => (self.mean_alt, f64::NAN),
            na => (self.mean_alt, (self.m2_alt / (na as f64 - 1.0)).sqrt()),
        };

        let (m_lat, m_lon) = meters_per_degree(self.mean_lat);
        let var_n = var_lat * m_lat * m_lat;
        let var_e = var_lon * m_lon * m_lon;
        let cov_ne = cov_latlon * m_lat * m_lon;

        let (cep50, cep95) = match principal_sigmas(var_n, var_e, cov_ne) {
            Some((s1, s2)) => (
                circular_error_radius(s1, s2, 0.5),
                circular_error_radius(s1, s2, 0.95),
            ),
            None => (f64::NAN, f64::NAN),
        };

        Some(HoldResult {
            mean_lat: self.mean_lat,
            mean_lon: wrap180(self.lon_ref + self.mean_dlon),
            mean_alt,
            std_lat,
            std_lon,
            std_alt,
            cep50,
            cep95,
            count: self.count,
            duration: self.start_time.elapsed(),
            std_north_m: var_n.sqrt(),
            std_east_m: var_e.sqrt(),
            count_alt: self.count_alt,
        })
    }
}

/// Standard deviations along the principal axes (major, minor) of a 2x2
/// covariance matrix `[[vx, c], [c, vy]]`. `None` if the input is not finite.
fn principal_sigmas(vx: f64, vy: f64, c: f64) -> Option<(f64, f64)> {
    if !(vx.is_finite() && vy.is_finite() && c.is_finite()) {
        return None;
    }
    let mean = 0.5 * (vx + vy);
    let diff = 0.5 * (vx - vy);
    let r = (diff * diff + c * c).sqrt();
    let l1 = (mean + r).max(0.0);
    let l2 = (mean - r).max(0.0);
    Some((l1.sqrt(), l2.sqrt()))
}

/// Radius `R` of a circle centred on the mean such that a zero-mean bivariate
/// normal with independent principal sigmas `s1 >= s2` puts probability `p`
/// inside it.
pub fn circular_error_radius(s1: f64, s2: f64, p: f64) -> f64 {
    let (s1, s2) = if s1 >= s2 { (s1, s2) } else { (s2, s1) };
    if !(s1.is_finite() && s2.is_finite()) || !(0.0..1.0).contains(&p) {
        return f64::NAN;
    }
    if s1 == 0.0 {
        return 0.0;
    }
    // P(R) is monotonic in R; double the upper bracket until it covers p.
    let mut lo = 0.0;
    let mut hi = s1;
    while disc_probability(s1, s2, hi) < p {
        hi *= 2.0;
        if hi > s1 * 1e3 {
            break;
        }
    }
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if disc_probability(s1, s2, mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Probability that a zero-mean bivariate normal with independent sigmas
/// `s1 >= s2 >= 0` (and `s1 > 0`) falls within radius `r` of the origin.
///
/// In standardized elliptical coordinates `x = s1 rho cos u`,
/// `y = s2 rho sin u` the density is radially `exp(-rho^2/2) rho / 2pi`, and
/// the disc boundary is `rho_max(u) = r / sqrt(s1^2 cos^2 u + s2^2 sin^2 u)`,
/// which gives
///
/// `P = 1 - (1/2pi) Int_0^2pi exp(-r^2 / (2 (s1^2 cos^2 u + s2^2 sin^2 u))) du`.
///
/// The integrand is smooth, bounded and periodic (also for `s2 = 0`, where it
/// reduces to the 1D result `erf(r / (s1 sqrt 2))`), so the midpoint rule
/// converges very rapidly. By symmetry one quadrant suffices.
fn disc_probability(s1: f64, s2: f64, r: f64) -> f64 {
    if r <= 0.0 {
        return 0.0;
    }
    const STEPS: usize = 128;
    let h = (PI / 2.0) / STEPS as f64;
    let (v1, v2) = (s1 * s1, s2 * s2);
    let half_r2 = 0.5 * r * r;
    let mut sum = 0.0;
    for i in 0..STEPS {
        let (s, c) = ((i as f64 + 0.5) * h).sin_cos();
        sum += (-half_r2 / (v1 * c * c + v2 * s * s)).exp();
    }
    (1.0 - sum / STEPS as f64).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_tracks_accepted_fixes_only() {
        let mut h = PositionHold::new();
        assert_eq!(h.count(), 0);
        h.add_fix(51.5, -0.1, 10.0);
        h.add_fix(f64::NAN, -0.1, 10.0);
        assert_eq!(h.count(), 1);
    }

    #[test]
    fn test_empty_result() {
        let hold = PositionHold::new();
        assert!(hold.result().is_none());
    }

    #[test]
    fn test_single_fix() {
        let mut hold = PositionHold::new();
        hold.add_fix(51.5, -0.1, 100.0);
        assert!(hold.result().is_none()); // need at least 2
    }

    #[test]
    fn test_two_fixes() {
        let mut hold = PositionHold::new();
        hold.add_fix(51.5000, -0.1000, 100.0);
        hold.add_fix(51.5001, -0.1001, 101.0);
        let r = hold.result().unwrap();
        assert_eq!(r.count, 2);
        assert!((r.mean_lat - 51.50005).abs() < 1e-6);
        assert!(r.cep50 > 0.0);
        assert!(r.cep95 > r.cep50);
    }

    #[test]
    fn test_nan_fix_ignored() {
        let mut hold = PositionHold::new();
        hold.add_fix(51.5, -0.1, 100.0);
        hold.add_fix(f64::NAN, f64::NAN, 100.0);
        hold.add_fix(51.5001, -0.1001, 101.0);
        let r = hold.result().unwrap();
        assert_eq!(r.count, 2); // NaN fix was skipped
    }

    #[test]
    fn test_many_fixes_convergence() {
        let mut hold = PositionHold::new();
        // Feed 100 identical positions
        for _ in 0..100 {
            hold.add_fix(51.5, -0.1, 100.0);
        }
        let r = hold.result().unwrap();
        assert_eq!(r.count, 100);
        assert!((r.mean_lat - 51.5).abs() < 1e-10);
        assert!(r.cep50 < 0.01); // should be essentially zero
    }

    #[test]
    fn test_circular_cep_closed_form() {
        let s = 2.0;
        let r50 = circular_error_radius(s, s, 0.5);
        let r95 = circular_error_radius(s, s, 0.95);
        assert!((r50 - s * (2.0 * 2f64.ln()).sqrt()).abs() < 1e-9);
        assert!((r95 - s * (2.0 * 20f64.ln()).sqrt()).abs() < 1e-9);
    }

    #[test]
    fn test_one_dimensional_cep() {
        // s2 = 0: 50% of a 1D normal lies within 0.67449 sigma.
        let r50 = circular_error_radius(1.0, 0.0, 0.5);
        assert!((r50 - 0.674_489_750_196_081_7).abs() < 1e-9);
        let r95 = circular_error_radius(1.0, 0.0, 0.95);
        assert!((r95 - 1.959_963_984_540_054).abs() < 1e-9);
    }

    #[test]
    fn test_wrap180() {
        assert_eq!(wrap180(180.0), -180.0);
        assert_eq!(wrap180(-180.0), -180.0);
        assert!((wrap180(359.8) - -0.2).abs() < 1e-12);
        assert!((wrap180(-359.8) - 0.2).abs() < 1e-12);
    }
}
