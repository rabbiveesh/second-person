//! The adaptive difficulty engine: pure logic, no Bevy and no game types (the wiring and the
//! levers each band and the dial pull are in `crate::difficulty`).
//!
//! - [`PlayerProfile`] + [`reduce`]: a pure reducer over [`AdaptEvent`]s. Per [`Skill`]: a center
//!   band 1..=10, a spread 0..1 and a rolling window of the last 20 rounds. Globally: the assist
//!   dial 0..1 (separate from difficulty), calibration, recent wins and losses.
//! - [`next_round`]: each skill's band for the next round, sampled from [`band_distribution`]
//!   around its center, so a promotion is never a cliff: stretch rounds come first.
//! - [`calibration`]: the first rounds are a disguised placement test.
//! - [`sim`]: synthetic players for tuning and tests (`cargo run --example simulate -- --all`).
//!
//! Never tell the player: nothing here produces a label, and [`Cue`]s are for logs and tools.
//!
//! # Rules
//!
//! - One round gives evidence on each skill it tells us about ([`RoundResult::outcome`]):
//!   Stealth is clean when your first hit lands before he engages; Gunfight, only when he
//!   engaged, is clean when you win.
//! - **Assists fade before difficulty rises.** A win lowers the dial by [`profile::ASSIST_FADE`];
//!   a loss raises it by [`profile::ASSIST_RISE`]. A promotion needs the dial ≤
//!   [`profile::ASSIST_EPS`], and only rounds played that unassisted count as promotion
//!   evidence. So while assists are on, winning lowers assists instead of raising the band.
//! - **Promote** a skill at ≥75% clean over ≥5 rounds at its center (and ≥60% on stretch rounds
//!   if there were ≥2); **demote** at <50% over ≥5. Evidence is per epoch: a band change bumps
//!   the epoch, so old window entries never count toward the new center. Going back up into a
//!   band just fallen out of needs 10 rounds *and* ≥2 stretch rounds at ≥60% (hysteresis).
//!   Promote/demote narrow the spread; >75% clean over the skill's last ≥10 rounds widens it.
//! - **Frustration** ([`frustration`]): 3 losses in a row, or a quick loss (<20s) right after
//!   another loss. Response: drop each struggling skill's band by 1 (only narrow it, after a
//!   stretch round), and raise assists by the frustration bump instead of the plain rise.
//!
//! # Tuning
//!
//! See `src/adapt/README.md` for the simulator results the constants were tuned against.

pub mod calibration;
pub mod choose;
pub mod frustration;
pub mod profile;
pub mod sim;
pub mod skill;
pub mod window;

pub use calibration::{Calibration, Placement, Probe};
pub use choose::{band_distribution, next_round, sample_band};
pub use frustration::FrustrationSignal;
pub use profile::{AdaptEvent, Cue, PlayerProfile, RoundResult, SkillState, reduce};
pub use skill::{BASE_BAND, Band, MAX_BAND, MIN_BAND, Skill};
pub use window::{Outcome, RollingWindow, WindowEntry};
