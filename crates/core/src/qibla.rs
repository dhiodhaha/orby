//! Qibla: the great-circle bearing and distance to the Kaaba.

const KAABA: (f64, f64) = (21.4225, 39.8262);

/// Degrees from true north, clockwise.
pub fn bearing(lat: f64, lng: f64) -> f64 {
    let (a, b) = (lat.to_radians(), lng.to_radians());
    let (c, d) = (KAABA.0.to_radians(), KAABA.1.to_radians());
    let y = (d - b).sin() * c.cos();
    let x = a.cos() * c.sin() - a.sin() * c.cos() * (d - b).cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

pub fn distance_km(lat: f64, lng: f64) -> f64 {
    let (a, b) = (lat.to_radians(), lng.to_radians());
    let (c, d) = (KAABA.0.to_radians(), KAABA.1.to_radians());
    let h = ((c - a) / 2.0).sin().powi(2) + a.cos() * c.cos() * ((d - b) / 2.0).sin().powi(2);
    2.0 * 6371.0 * h.sqrt().asin()
}

/// 16-point compass name, e.g. "WNW".
pub fn compass(deg: f64) -> &'static str {
    const N: [&str; 16] = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
    N[(((deg % 360.0) / 22.5) + 0.5) as usize % 16]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yogyakarta() {
        let (lat, lng) = (-7.7956, 110.3695);
        assert_eq!(bearing(lat, lng).round(), 295.0);
        assert_eq!(compass(bearing(lat, lng)), "WNW");
        assert!((distance_km(lat, lng) - 8350.0).abs() < 20.0);
    }
}
