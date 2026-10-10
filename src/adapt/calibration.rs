//! The disguised placement test: the first rounds.
//!
//! Probe rounds play both skills at the same band. The first is at [`START_BAND`]; a win moves
//! the next probe up [`UP_STEP`] bands, a loss down [`DOWN_STEP`]. It stops as soon as ability is
//! bracketed (a win and a loss), or the player bottoms out / tops out, and after [`MAX_PROBES`]
//! rounds at most. Then [`Calibration::placement`] picks the starting bands, spread and assists.

use super::skill::{Band, MAX_BAND, MIN_BAND, clamp_band};

pub const START_BAND: Band = 3;
pub const UP_STEP: i32 = 2;
pub const DOWN_STEP: i32 = 1;
pub const MIN_PROBES: usize = 2;
pub const MAX_PROBES: usize = 3;
/// Starting assists when no probe was won...
pub const NO_WIN_ASSISTS: f32 = 0.8;
/// ...when some probe was lost...
pub const SOME_LOSS_ASSISTS: f32 = 0.6;
/// ...and when every probe was won.
pub const ALL_WON_ASSISTS: f32 = 0.35;
pub const PLACEMENT_SPREAD: f32 = 0.5;

/// One probe round's result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probe {
    pub band: Band,
    pub won: bool,
}

/// Where calibration landed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Starting center for every skill.
    pub band: Band,
    pub spread: f32,
    pub assists: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Calibration {
    pub probes: Vec<Probe>,
    /// Set once the placement has been applied to the profile.
    pub done: bool,
}

impl Calibration {
    /// Calibration that's already over (for profiles set up directly, e.g. in tests).
    pub fn finished() -> Self {
        Calibration { probes: Vec::new(), done: true }
    }

    /// Band of the next probe round.
    pub fn next_band(&self) -> Band {
        match self.probes.last() {
            None => START_BAND,
            Some(p) => clamp_band(p.band as i32 + if p.won { UP_STEP } else { -DOWN_STEP }),
        }
    }

    pub fn record(&self, probe: Probe) -> Self {
        let mut probes = self.probes.clone();
        probes.push(probe);
        Calibration { probes, done: self.done }
    }

    /// Enough probes to place the player.
    pub fn complete(&self) -> bool {
        let n = self.probes.len();
        if n >= MAX_PROBES {
            return true;
        }
        if n < MIN_PROBES {
            return false;
        }
        let bracketed = self.probes.iter().any(|p| p.won) && self.probes.iter().any(|p| !p.won);
        let last = self.probes[n - 1];
        let at_floor = !last.won && last.band <= MIN_BAND;
        let at_ceiling = last.won && last.band >= MAX_BAND;
        bracketed || at_floor || at_ceiling
    }

    /// Starting dials from the probes so far: the highest band won (band 1 if none).
    pub fn placement(&self) -> Placement {
        let wins = self.probes.iter().filter(|p| p.won);
        let band = wins.map(|p| p.band).max().unwrap_or(MIN_BAND);
        let any_won = self.probes.iter().any(|p| p.won);
        let any_lost = self.probes.iter().any(|p| !p.won);
        let assists = if !any_won {
            NO_WIN_ASSISTS
        } else if any_lost {
            SOME_LOSS_ASSISTS
        } else {
            ALL_WON_ASSISTS
        };
        Placement { band, spread: PLACEMENT_SPREAD, assists }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(band: Band, won: bool) -> Probe {
        Probe { band, won }
    }

    #[test]
    fn walks_up_two_down_one() {
        let c = Calibration::default();
        assert_eq!(c.next_band(), 3);
        let c = c.record(probe(3, true));
        assert_eq!(c.next_band(), 5);
        let c = c.record(probe(5, false));
        assert_eq!(c.next_band(), 4);
        assert!(c.complete(), "bracketed after a win and a loss");
        assert_eq!(c.placement().band, 3);
    }

    #[test]
    fn floor_and_ceiling_clamp() {
        let c = Calibration::default().record(probe(1, false));
        assert_eq!(c.next_band(), 1);
        let c = Calibration::default().record(probe(10, true));
        assert_eq!(c.next_band(), 10);
    }

    #[test]
    fn winning_every_probe_places_high_with_little_help() {
        let c = Calibration::default().record(probe(3, true)).record(probe(5, true)).record(probe(7, true));
        assert!(c.complete());
        let p = c.placement();
        assert_eq!(p.band, 7);
        assert_eq!(p.assists, ALL_WON_ASSISTS);
    }
}
