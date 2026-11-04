//! A track from successive location fixes.
//!
//! The platform says where you are; the core answers what a person actually
//! asks — how far have I walked, in which direction, how fast. All plain maths,
//! so it is tested against distances you can check on a map instead of against a
//! phone in a pocket.

use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Mean Earth radius, metres (IUGG).
const EARTH_RADIUS: f64 = 6_371_008.8;

/// One reading: where, and how sure the device is (metres).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoFix {
    pub lat: f64,
    pub lon: f64,
    pub accuracy: f64,
}

/// The track after a fix: where you are now, and what it has done since it
/// started.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    pub lat: f64,
    pub lon: f64,
    pub accuracy: f64,
    pub fixes: i32,
    /// Metres travelled since the first fix
    pub moved: f64,
    /// Degrees from the first fix, 0 = north
    pub bearing: f64,
    /// Metres per second, smoothed
    pub speed: f64,
    /// Since the first fix
    pub seconds: f64,
}

/// The great-circle distance between two points, in metres.
///
/// Haversine: exact enough at walking scale, and no convergence traps near the
/// poles the way a flat approximation has.
pub fn distance(a: GeoFix, b: GeoFix) -> f64 {
    let lat1 = a.lat.to_radians();
    let lat2 = b.lat.to_radians();
    let d_lat = lat2 - lat1;
    let d_lon = (b.lon - a.lon).to_radians();
    let h = (d_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (d_lon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS * h.sqrt().min(1.0).asin()
}

/// The initial compass bearing from `a` to `b`, in degrees (0 = north,
/// 90 = east).
pub fn bearing(a: GeoFix, b: GeoFix) -> f64 {
    let lat1 = a.lat.to_radians();
    let lat2 = b.lat.to_radians();
    let d_lon = (b.lon - a.lon).to_radians();
    let y = d_lon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * d_lon.cos();
    (y.atan2(x).to_degrees() + 360.0).rem_euclid(360.0)
}

/// Builds a [`GeoPoint`] out of the fixes a platform callback delivers.
#[derive(Debug, Default)]
pub struct Track {
    first: Option<GeoFix>,
    last: Option<GeoFix>,
    start: Option<Instant>,
    last_at: Option<Instant>,
    moved: f64,
    speed: f64,
    fixes: i32,
}

impl Track {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a reading and reports the track so far. `now` is when the reading
    /// was taken.
    pub fn fix(&mut self, fix: GeoFix, now: Instant) -> GeoPoint {
        let (first, last, start, last_at) = match (self.first, self.last, self.start, self.last_at)
        {
            (Some(f), Some(l), Some(s), Some(t)) => (f, l, s, t),
            _ => {
                self.first = Some(fix);
                self.last = Some(fix);
                self.start = Some(now);
                self.last_at = Some(now);
                self.fixes = 1;
                return self.point(fix);
            }
        };

        let step = distance(last, fix);
        let dt = now.saturating_duration_since(last_at).as_secs_f64();

        // A step smaller than the fix's own accuracy is noise, not walking:
        // standing still with ±22 m of GPS jitter otherwise "walks" hundreds of
        // metres in a minute. The fix still counts, and the last position still
        // moves with it — otherwise one bad fix poisons the next step.
        let walks = step > last.accuracy.max(2.0);

        if walks || dt > 1.0 {
            if walks {
                self.moved += step;
            }
            if dt > 0.05 && walks {
                // Smoothed, so one noisy pair does not swing the number the UI
                // shows — but the first measurement is the first measurement:
                // damping it against a made-up zero would understate a real walk.
                let instant = step / dt;
                self.speed = if self.speed == 0.0 {
                    instant
                } else {
                    0.6 * self.speed + 0.4 * instant
                };
            }
            self.last = Some(fix);
            self.last_at = Some(now);
        }
        self.fixes += 1;
        let _ = (first, start);
        self.point(fix)
    }

    fn point(&self, fix: GeoFix) -> GeoPoint {
        let seconds = match (self.last_at, self.start) {
            (Some(last_at), Some(start)) => last_at.saturating_duration_since(start).as_secs_f64(),
            _ => 0.0,
        };
        GeoPoint {
            lat: fix.lat,
            lon: fix.lon,
            accuracy: fix.accuracy,
            fixes: self.fixes,
            moved: self.moved,
            bearing: match self.first {
                Some(first) if self.fixes > 1 => bearing(first, fix),
                _ => 0.0,
            },
            speed: self.speed,
            seconds: if seconds.is_finite() { seconds } else { 0.0 },
        }
    }

    /// Throws the track away: the UI stopped listening, or started again
    /// somewhere else.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;
    use std::time::Duration;

    /// A degree of a great circle, for the tests: 2πR/360.
    const DEGREE_METRES: f64 = 2.0 * PI * EARTH_RADIUS / 360.0;

    fn at(lat: f64, lon: f64) -> GeoFix {
        GeoFix {
            lat,
            lon,
            accuracy: 5.0,
        }
    }

    #[test]
    fn distance_between_capitals() {
        let paris = GeoFix {
            lat: 48.8566,
            lon: 2.3522,
            accuracy: 10.0,
        };
        let london = GeoFix {
            lat: 51.5074,
            lon: -0.1278,
            accuracy: 10.0,
        };
        // Paris → London is 343.5 km, a number you can check on any map.
        let got = distance(paris, london);
        assert!((got - 343_556.0).abs() < 343_556.0 * 0.005, "{got} m");
    }

    #[test]
    fn bearing_paris_to_london_is_nnw() {
        let paris = GeoFix {
            lat: 48.8566,
            lon: 2.3522,
            accuracy: 10.0,
        };
        let london = GeoFix {
            lat: 51.5074,
            lon: -0.1278,
            accuracy: 10.0,
        };
        // The *initial* bearing. You arrive in London on 336°, which is the
        // number that looks familiar on a globe.
        let got = bearing(paris, london);
        assert!((got - 330.2).abs() < 0.5, "{got}°");
    }

    #[test]
    fn a_degree_on_the_equator() {
        let origin = at(0.0, 0.0);
        let east = at(0.0, 1.0);
        let north = at(1.0, 0.0);

        let got = distance(origin, east);
        assert!((got - DEGREE_METRES).abs() < 200.0, "{got} m");
        assert!((bearing(origin, east) - 90.0).abs() < 0.01);
        assert!(bearing(origin, north).abs() < 0.01);
    }

    #[test]
    fn adds_up_the_walk() {
        let t0 = Instant::now();
        let mut track = Track::new();
        let start = at(0.0, 0.0);
        let one = at(0.0, 0.001); // ~111 m east
        let two = at(0.0, 0.002); // another ~111 m

        let p = track.fix(start, t0);
        assert_eq!(
            (p.fixes, p.moved, p.bearing, p.speed, p.seconds),
            (1, 0.0, 0.0, 0.0, 0.0)
        );

        let p = track.fix(one, t0 + Duration::from_secs(10));
        assert_eq!(p.fixes, 2);
        assert!(
            (p.moved - DEGREE_METRES * 0.001).abs() < 5.0,
            "moved {}",
            p.moved
        );
        assert!((p.bearing - 90.0).abs() < 1.0, "bearing {}", p.bearing);
        assert!(
            (p.speed - DEGREE_METRES * 0.001 / 10.0).abs() < 1.0,
            "speed {}",
            p.speed
        );
        assert!((p.seconds - 10.0).abs() < 0.01);

        let p = track.fix(two, t0 + Duration::from_secs(20));
        assert!(
            (p.moved - DEGREE_METRES * 0.002).abs() < 10.0,
            "moved {}",
            p.moved
        );

        track.reset();
        let p = track.fix(start, t0 + Duration::from_secs(30));
        assert_eq!(
            (p.fixes, p.moved, p.speed),
            (1, 0.0, 0.0),
            "a reset is a new track"
        );
    }

    #[test]
    fn jitter_under_the_accuracy_is_not_walking() {
        // ±22 m of GPS noise, standing still: 20 fixes of 5 m each. A track that
        // counted them would report 100 m walked without moving.
        let t0 = Instant::now();
        let mut track = Track::new();
        let noisy = GeoFix {
            lat: 0.0,
            lon: 0.0,
            accuracy: 22.0,
        };
        track.fix(noisy, t0);
        for i in 1..20 {
            // 5 m north each fix, well inside the accuracy.
            let step = GeoFix {
                lat: 5.0 * i as f64 / DEGREE_METRES,
                lon: 0.0,
                accuracy: 22.0,
            };
            let p = track.fix(step, t0 + Duration::from_secs(i as u64));
            assert!(p.moved < 1.0, "fix {i}: moved {}", p.moved);
        }
    }
}
