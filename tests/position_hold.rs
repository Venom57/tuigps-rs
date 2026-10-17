//! Numerical tests for `PositionHold` (plan T6, B5, B6, CEP review).

use tuigps_rs::position_hold::{PositionHold, circular_error_radius, meters_per_degree};

/// Deterministic xorshift64* PRNG with Box-Muller normals.
struct Rng(u64);

impl Rng {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // (0, 1]
        ((x >> 11) as f64 + 1.0) / (1u64 << 53) as f64
    }

    fn normal(&mut self) -> f64 {
        let u1 = self.uniform();
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

fn two_pass(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    (mean, var.sqrt())
}

fn rel_close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * b.abs().max(1e-300)
}

/// Offsets (north, east) in meters drawn from a bivariate normal with
/// principal sigmas `s_major`, `s_minor`, major axis rotated `angle_deg`
/// from north towards east.
fn gaussian_offsets(
    rng: &mut Rng,
    n: usize,
    s_major: f64,
    s_minor: f64,
    angle_deg: f64,
) -> Vec<(f64, f64)> {
    let (sa, ca) = angle_deg.to_radians().sin_cos();
    (0..n)
        .map(|_| {
            let a = rng.normal() * s_major;
            let b = rng.normal() * s_minor;
            (a * ca - b * sa, a * sa + b * ca)
        })
        .collect()
}

#[test]
fn mean_and_std_match_two_pass_reference() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let (lat0, lon0) = (47.3, 8.5);
    let n = 2000;
    let lats: Vec<f64> = (0..n).map(|_| lat0 + rng.normal() * 2e-5).collect();
    let lons: Vec<f64> = (0..n).map(|_| lon0 + rng.normal() * 3e-5).collect();
    let alts: Vec<f64> = (0..n).map(|_| 400.0 + rng.normal() * 2.5).collect();

    let mut hold = PositionHold::new();
    for i in 0..n {
        hold.add_fix(lats[i], lons[i], alts[i]);
    }
    let r = hold.result().unwrap();
    let (ml, sl) = two_pass(&lats);
    let (mo, so) = two_pass(&lons);
    let (ma, sa) = two_pass(&alts);

    assert_eq!(r.count, n as u64);
    assert_eq!(r.count_alt, n as u64);
    assert!((r.mean_lat - ml).abs() < 1e-12);
    assert!((r.mean_lon - mo).abs() < 1e-12);
    assert!((r.mean_alt - ma).abs() < 1e-9);
    assert!(rel_close(r.std_lat, sl, 1e-6), "{} vs {sl}", r.std_lat);
    assert!(rel_close(r.std_lon, so, 1e-6), "{} vs {so}", r.std_lon);
    assert!(rel_close(r.std_alt, sa, 1e-9), "{} vs {sa}", r.std_alt);

    let (m_lat, m_lon) = meters_per_degree(r.mean_lat);
    assert!(rel_close(r.std_north_m, sl * m_lat, 1e-6));
    assert!(rel_close(r.std_east_m, so * m_lon, 1e-6));
}

#[test]
fn meters_per_degree_matches_wgs84_reference() {
    // Reference values (WGS84): equator and 45 degrees.
    let (lat0, lon0) = meters_per_degree(0.0);
    assert!((lat0 - 110_574.27).abs() < 0.1, "{lat0}");
    assert!((lon0 - 111_319.49).abs() < 0.1, "{lon0}");
    let (lat45, lon45) = meters_per_degree(45.0);
    assert!((lat45 - 111_131.75).abs() < 0.1, "{lat45}");
    assert!((lon45 - 78_846.81).abs() < 0.1, "{lon45}");
}

#[test]
fn nan_altitudes_use_separate_count() {
    let mut hold = PositionHold::new();
    let alts = [100.0, f64::NAN, 104.0, f64::NAN, f64::NAN, 102.0];
    for (i, &alt) in alts.iter().enumerate() {
        hold.add_fix(10.0 + i as f64 * 1e-6, 20.0, alt);
    }
    let r = hold.result().unwrap();
    assert_eq!(r.count, 6);
    assert_eq!(r.count_alt, 3);
    let (ma, sa) = two_pass(&[100.0, 104.0, 102.0]);
    assert!((r.mean_alt - ma).abs() < 1e-12, "{}", r.mean_alt);
    assert!((r.std_alt - sa).abs() < 1e-12, "{}", r.std_alt);
}

#[test]
fn all_nan_altitude_gives_nan_not_zero() {
    let mut hold = PositionHold::new();
    for i in 0..5 {
        hold.add_fix(10.0 + i as f64 * 1e-6, 20.0, f64::NAN);
    }
    let r = hold.result().unwrap();
    assert_eq!(r.count_alt, 0);
    assert!(r.mean_alt.is_nan());
    assert!(r.std_alt.is_nan());
    assert!(r.cep50.is_finite());
}

#[test]
fn single_altitude_sample_has_mean_but_no_std() {
    let mut hold = PositionHold::new();
    hold.add_fix(10.0, 20.0, f64::NAN);
    hold.add_fix(10.0, 20.0, 55.0);
    hold.add_fix(10.0, 20.0, f64::INFINITY);
    let r = hold.result().unwrap();
    assert_eq!(r.count_alt, 1);
    assert_eq!(r.mean_alt, 55.0);
    assert!(r.std_alt.is_nan());
}

#[test]
fn antimeridian_mean_and_spread() {
    let mut rng = Rng(42);
    let offsets = gaussian_offsets(&mut rng, 1000, 3.0, 3.0, 0.0);
    let lat0 = -16.5; // Fiji
    let (m_lat, m_lon) = meters_per_degree(lat0);

    let run = |lon0: f64| {
        let mut hold = PositionHold::new();
        for &(dn, de) in &offsets {
            let lon = lon0 + de / m_lon;
            // Normalize to [-180, 180) like gpsd does.
            let lon = (lon + 180.0).rem_euclid(360.0) - 180.0;
            hold.add_fix(lat0 + dn / m_lat, lon, f64::NAN);
        }
        hold.result().unwrap()
    };

    let at_180 = run(180.0);
    let at_0 = run(0.0);

    // Mean is within centimeters of the antimeridian, not near 0.
    let err_deg = (at_180.mean_lon.abs() - 180.0).abs();
    assert!(err_deg * m_lon < 0.5, "mean_lon {}", at_180.mean_lon);
    assert!((-180.0..180.0).contains(&at_180.mean_lon));
    // Spread and CEP are identical to the same cloud at lon 0.
    assert!(rel_close(at_180.std_east_m, at_0.std_east_m, 1e-6));
    assert!(rel_close(at_180.std_lon, at_0.std_lon, 1e-6));
    assert!(rel_close(at_180.cep50, at_0.cep50, 1e-6));
    assert!(rel_close(at_180.cep95, at_0.cep95, 1e-6));
    assert!(at_180.std_east_m < 4.0, "{}", at_180.std_east_m);
}

#[test]
fn antimeridian_two_points() {
    let mut hold = PositionHold::new();
    hold.add_fix(0.0, 179.9, 0.0);
    hold.add_fix(0.0, -179.9, 0.0);
    let r = hold.result().unwrap();
    assert!((r.mean_lon.abs() - 180.0).abs() < 1e-9, "{}", r.mean_lon);
    // Two points 0.2 deg apart: sample std = 0.2 / sqrt(2).
    assert!(
        (r.std_lon - 0.2 / 2f64.sqrt()).abs() < 1e-9,
        "{}",
        r.std_lon
    );
}

/// Empirical fraction of points within `radius` meters of the sample mean.
fn fraction_within(offsets: &[(f64, f64)], radius: f64) -> f64 {
    let n = offsets.len() as f64;
    let mn = offsets.iter().map(|o| o.0).sum::<f64>() / n;
    let me = offsets.iter().map(|o| o.1).sum::<f64>() / n;
    offsets
        .iter()
        .filter(|(dn, de)| ((dn - mn).powi(2) + (de - me).powi(2)).sqrt() <= radius)
        .count() as f64
        / n
}

fn hold_from_offsets(lat0: f64, lon0: f64, offsets: &[(f64, f64)]) -> PositionHold {
    let (m_lat, m_lon) = meters_per_degree(lat0);
    let mut hold = PositionHold::new();
    for &(dn, de) in offsets {
        hold.add_fix(lat0 + dn / m_lat, lon0 + de / m_lon, 0.0);
    }
    hold
}

#[test]
fn cep_on_circular_gaussian() {
    let mut rng = Rng(7);
    let sigma = 3.0;
    let offsets = gaussian_offsets(&mut rng, 20_000, sigma, sigma, 0.0);
    let r = hold_from_offsets(52.0, 13.4, &offsets).result().unwrap();

    // Closed form for a circular Gaussian (within sampling error).
    let cep50_theory = sigma * (2.0 * 2f64.ln()).sqrt(); // 1.1774 sigma
    let cep95_theory = sigma * (2.0 * 20f64.ln()).sqrt(); // 2.4477 sigma
    assert!(rel_close(r.cep50, cep50_theory, 0.02), "{}", r.cep50);
    assert!(rel_close(r.cep95, cep95_theory, 0.02), "{}", r.cep95);

    let f50 = fraction_within(&offsets, r.cep50);
    let f95 = fraction_within(&offsets, r.cep95);
    assert!((f50 - 0.50).abs() < 0.01, "f50 = {f50}");
    assert!((f95 - 0.95).abs() < 0.005, "f95 = {f95}");
}

#[test]
fn cep_on_correlated_elliptical_gaussian() {
    let mut rng = Rng(99);
    // 4 m x 1 m ellipse rotated 30 degrees: strongly correlated N/E errors.
    let offsets = gaussian_offsets(&mut rng, 20_000, 4.0, 1.0, 30.0);
    let r = hold_from_offsets(-33.9, 151.2, &offsets).result().unwrap();

    let f50 = fraction_within(&offsets, r.cep50);
    let f95 = fraction_within(&offsets, r.cep95);
    assert!((f50 - 0.50).abs() < 0.01, "f50 = {f50}");
    assert!((f95 - 0.95).abs() < 0.005, "f95 = {f95}");

    // Same answer as the analytic radius for the true principal sigmas.
    assert!(rel_close(
        r.cep50,
        circular_error_radius(4.0, 1.0, 0.5),
        0.02
    ));
    assert!(rel_close(
        r.cep95,
        circular_error_radius(4.0, 1.0, 0.95),
        0.02
    ));
}

#[test]
fn circular_error_radius_reference_values() {
    // Circular closed forms.
    assert!((circular_error_radius(1.0, 1.0, 0.5) - 1.177_410_022_515_474_7).abs() < 1e-9);
    assert!((circular_error_radius(1.0, 1.0, 0.95) - 2.447_746_830_680_816).abs() < 1e-9);
    // 1D limit: normal quantiles 0.674 and 1.960.
    assert!((circular_error_radius(2.0, 0.0, 0.5) - 2.0 * 0.674_489_750_196_081_7).abs() < 1e-8);
    assert!((circular_error_radius(0.0, 2.0, 0.95) - 2.0 * 1.959_963_984_540_054).abs() < 1e-8);
    // Degenerate and invalid inputs.
    assert_eq!(circular_error_radius(0.0, 0.0, 0.5), 0.0);
    assert!(circular_error_radius(f64::NAN, 1.0, 0.5).is_nan());
    assert!(circular_error_radius(1.0, 1.0, 1.0).is_nan());
}

#[test]
fn stationary_receiver_has_zero_cep() {
    let mut hold = PositionHold::new();
    for _ in 0..10 {
        hold.add_fix(51.5, -0.1, 100.0);
    }
    let r = hold.result().unwrap();
    assert_eq!(r.cep50, 0.0);
    assert_eq!(r.cep95, 0.0);
    assert_eq!(r.std_alt, 0.0);
}
