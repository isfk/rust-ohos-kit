//! The accelerometer, turned into degrees.
//!
//! The platform calls this 50 times a second; the arithmetic is the app's
//! business, not the UI's, so it lives here and not in ArkTS.

use std::time::Instant;

use serde::{Deserialize, Serialize};

/// What the UI gets: degrees, and how fast samples are arriving.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LevelReading {
    pub pitch: f64,
    pub roll: f64,
    pub rate: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A smoothed level.
///
/// The smoothing weight is a fifth: fast enough to feel live, slow enough that
/// the bubble does not jitter. One bump must not swing the reading, which is
/// what the spike test below pins.
#[derive(Debug)]
pub struct Level {
    x: f64,
    y: f64,
    z: f64,
    primed: bool,
    alpha: f64,
    samples: u32,
    since: Instant,
    per_sec: f64,
}

impl Level {
    pub fn new() -> Self {
        Self::new_at(Instant::now())
    }

    /// The clock is a parameter so a test can hand it two seconds of wall time
    /// without sleeping for two seconds.
    pub fn new_at(now: Instant) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            primed: false,
            alpha: 0.2,
            samples: 0,
            since: now,
            per_sec: 0.0,
        }
    }

    /// Takes one sample (m/s², gravity included) and reports the level. The rate
    /// is recomputed at most once a second: it is a reading, not an event.
    pub fn add(&mut self, x: f64, y: f64, z: f64) -> LevelReading {
        self.add_at(x, y, z, Instant::now())
    }

    pub fn add_at(&mut self, x: f64, y: f64, z: f64, now: Instant) -> LevelReading {
        if self.primed {
            self.x += self.alpha * (x - self.x);
            self.y += self.alpha * (y - self.y);
            self.z += self.alpha * (z - self.z);
        } else {
            // The first sample is the truth: averaging it against a zero the
            // phone never had would make the bubble jump on start.
            self.x = x;
            self.y = y;
            self.z = z;
            self.primed = true;
        }
        self.samples += 1;

        let elapsed = now.saturating_duration_since(self.since).as_secs_f64();
        if elapsed >= 1.0 {
            self.per_sec = f64::from(self.samples) / elapsed;
            self.samples = 0;
            self.since = now;
        }

        LevelReading {
            pitch: self.pitch(),
            roll: self.roll(),
            rate: self.per_sec,
            x: self.x,
            y: self.y,
            z: self.z,
        }
    }

    /// How far the top of the phone is tipped toward or away from you.
    fn pitch(&self) -> f64 {
        (-self.y).atan2(self.x.hypot(self.z)).to_degrees()
    }

    /// How far it is tipped left or right.
    fn roll(&self) -> f64 {
        self.x.atan2(self.y.hypot(self.z)).to_degrees()
    }
}

impl Default for Level {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const G: f64 = 9.8;

    fn run(level: &mut Level, samples: usize, x: f64, y: f64, z: f64) -> LevelReading {
        let mut got = level.add(x, y, z);
        for _ in 1..samples {
            got = level.add(x, y, z);
        }
        got
    }

    #[test]
    fn flat_is_zero() {
        let mut level = Level::new();
        // Face up on a table: gravity down the z axis.
        let got = run(&mut level, 50, 0.0, 0.0, G);
        assert!(got.pitch.abs() < 1.0, "pitch {}", got.pitch);
        assert!(got.roll.abs() < 1.0, "roll {}", got.roll);
    }

    #[test]
    fn tipped_thirty_degrees() {
        let mut level = Level::new();
        // Tipped 30° to the right: gravity splits between x and z.
        let x = G * 30f64.to_radians().sin();
        let z = G * 30f64.to_radians().cos();
        let got = run(&mut level, 200, x, 0.0, z);
        assert!((got.roll - 30.0).abs() < 1.0, "roll {}", got.roll);
        assert!(got.pitch.abs() < 1.0, "pitch {}", got.pitch);
    }

    #[test]
    fn one_bump_does_not_move_the_bubble() {
        let mut level = Level::new();
        run(&mut level, 50, 0.0, 0.0, G);
        // A bump: the raw value would jump to 90°.
        let got = level.add(G, 0.0, 0.0);
        assert!(
            got.roll <= 30.0,
            "one sample moved the bubble to {}°",
            got.roll
        );
    }

    #[test]
    fn rate_is_samples_over_a_second() {
        let t0 = Instant::now();
        let mut level = Level::new_at(t0);
        for i in 0..99 {
            level.add_at(0.0, 0.0, G, t0 + Duration::from_millis(20 * i));
        }
        // 100 samples over 2 s: ~50 a second, reported once the second is up.
        let got = level.add_at(0.0, 0.0, G, t0 + Duration::from_secs(2));
        assert!((40.0..=60.0).contains(&got.rate), "rate {}", got.rate);
    }
}
