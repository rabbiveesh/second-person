//! A skill's rolling window of recent round outcomes.

use super::skill::Band;

/// How a round went for one skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Clean,
    Struggle,
}

/// One round, as seen by one of the skills it exercised.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowEntry {
    pub outcome: Outcome,
    /// The band this skill was played at in that round.
    pub band: Band,
    /// The skill's center when the round started.
    pub center: Band,
    /// The skill's epoch when the round started. The epoch bumps on every band change, so old
    /// entries never count as evidence for the new center ("fresh evidence").
    pub epoch: u32,
    /// The assist dial when the round started. Only (nearly) unassisted wins count toward a
    /// promotion: assists fade before difficulty rises.
    pub assists: f32,
}

/// The last [`WINDOW_SIZE`] entries. Immutable: [`RollingWindow::push`] returns a new window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RollingWindow {
    pub entries: Vec<WindowEntry>,
}

/// Rounds remembered per skill.
pub const WINDOW_SIZE: usize = 20;

/// `(clean fraction, count)`; `None` with no data.
fn rate<'a>(it: impl Iterator<Item = &'a WindowEntry>) -> (Option<f32>, usize) {
    let (mut clean, mut n) = (0usize, 0usize);
    for e in it {
        n += 1;
        if e.outcome == Outcome::Clean {
            clean += 1;
        }
    }
    if n == 0 { (None, 0) } else { (Some(clean as f32 / n as f32), n) }
}

impl RollingWindow {
    pub fn push(&self, entry: WindowEntry) -> Self {
        let mut entries = self.entries.clone();
        entries.push(entry);
        if entries.len() > WINDOW_SIZE {
            entries.drain(..entries.len() - WINDOW_SIZE);
        }
        RollingWindow { entries }
    }

    /// Clean rate over everything remembered.
    pub fn clean_rate(&self) -> (Option<f32>, usize) {
        rate(self.entries.iter())
    }

    /// Clean rate at the current center in the current epoch. `unassisted_only` keeps only
    /// rounds played with assists ≤ [`super::profile::ASSIST_EPS`].
    pub fn at_center(&self, center: Band, epoch: u32, unassisted_only: bool) -> (Option<f32>, usize) {
        rate(self.entries.iter().filter(|e| {
            e.epoch == epoch
                && e.band == center
                && (!unassisted_only || e.assists <= super::profile::ASSIST_EPS)
        }))
    }

    /// Clean rate of stretch rounds (above the center) in the current epoch.
    pub fn above_center(&self, center: Band, epoch: u32) -> (Option<f32>, usize) {
        rate(self.entries.iter().filter(|e| e.epoch == epoch && e.band > center))
    }
}
