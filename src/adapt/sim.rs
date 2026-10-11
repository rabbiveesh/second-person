//! Synthetic players run through the real reducer, to tune the constants and to test the
//! engine's behaviour over hundreds of rounds. Used by `examples/simulate.rs` and `tests/adapt.rs`.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::choose::next_round;
use super::profile::{AdaptEvent, Cue, PlayerProfile, RoundResult, reduce};
use super::skill::{Band, Skill};

/// A pretend player: a success curve per skill (logistic in the band), plus habits.
#[derive(Debug, Clone, Copy)]
pub struct SyntheticPlayer {
    pub name: &'static str,
    pub about: &'static str,
    /// Per skill: the band at which an unassisted attempt succeeds half the time.
    pub ability: [f32; Skill::COUNT],
    /// Bands per logistic unit: bigger is flatter (more inconsistent).
    pub slope: f32,
    /// Ability gained with practice: `+learn · n / (n + LEARN_HALF)` after `n` rounds.
    pub learn: f32,
    /// Chance a lost round is lost fast (charging in, dying in seconds).
    pub rushes: f32,
}

pub const LEARN_HALF: f32 = 100.0;
/// At a full assist dial, this share of the remaining failure chance goes away.
pub const ASSIST_HELP: f32 = 0.5;
/// Chance he engages after your first (sneaky) hit, rather than you finishing him unseen.
pub const ENGAGE_AFTER_FIRST_HIT: f32 = 0.7;
/// Landing the first hit takes this share of the remaining gunfight failure chance away.
pub const FIRST_HIT_EDGE: f32 = 0.3;
/// Rounds ignored by the "after warm-up" metrics (calibration + settling).
pub const WARMUP: usize = 30;
/// Trailing window for "time near target".
pub const TRAILING: usize = 20;
/// The win rate the engine aims to keep players in.
pub const TARGET: (f32, f32) = (0.6, 0.85);
/// A band change that reverses the same skill's previous change within this many rounds is an
/// oscillation (ping-pong). Slower reversals are just tracking a player's level.
pub const OSC_SPAN: usize = 25;

pub const PLAYERS: [SyntheticPlayer; 6] = [
    SyntheticPlayer {
        name: "precise",
        about: "consistent and good at everything",
        ability: [8.0, 8.0],
        slope: 0.7,
        learn: 0.0,
        rushes: 0.05,
    },
    SyntheticPlayer {
        name: "sloppy",
        about: "decent but inconsistent",
        ability: [5.5, 5.5],
        slope: 1.6,
        learn: 0.0,
        rushes: 0.2,
    },
    SyntheticPlayer {
        name: "beginner",
        about: "struggles even at band 1",
        ability: [1.0, 1.0],
        slope: 1.0,
        learn: 0.0,
        rushes: 0.3,
    },
    SyntheticPlayer {
        name: "brawler",
        about: "never sneaks, wins straight fights",
        ability: [1.5, 8.0],
        slope: 0.8,
        learn: 0.0,
        rushes: 0.15,
    },
    SyntheticPlayer {
        name: "sneak",
        about: "great at getting the first hit, weak in a fight",
        ability: [8.0, 2.5],
        slope: 0.8,
        learn: 0.0,
        rushes: 0.05,
    },
    SyntheticPlayer {
        name: "learner",
        about: "starts weak, improves steadily with practice",
        ability: [2.0, 2.0],
        slope: 0.9,
        learn: 6.0,
        rushes: 0.15,
    },
];

pub fn player(name: &str) -> Option<SyntheticPlayer> {
    PLAYERS.iter().copied().find(|p| p.name == name)
}

impl SyntheticPlayer {
    /// Chance of succeeding at `skill` at `band` after `practice` rounds, with assist dial `dial`.
    pub fn p(&self, skill: Skill, band: Band, practice: u32, dial: f32) -> f32 {
        let n = practice as f32;
        let ability = self.ability[skill.index()] + self.learn * n / (n + LEARN_HALF);
        let p = 1.0 / (1.0 + ((band as f32 - ability) / self.slope).exp());
        p + (1.0 - p) * ASSIST_HELP * dial.clamp(0.0, 1.0)
    }

    /// Play one round at `bands` with `dial`.
    pub fn play(&self, bands: [Band; Skill::COUNT], practice: u32, dial: f32, rng: &mut impl Rng) -> RoundResult {
        let sneak = rng.random::<f32>() < self.p(Skill::Stealth, bands[Skill::Stealth.index()], practice, dial);
        let engaged = !sneak || rng.random::<f32>() < ENGAGE_AFTER_FIRST_HIT;
        let won = if engaged {
            let p = self.p(Skill::Gunfight, bands[Skill::Gunfight.index()], practice, dial);
            let p = if sneak { p + (1.0 - p) * FIRST_HIT_EDGE } else { p };
            rng.random::<f32>() < p
        } else {
            true
        };
        let secs = if !won && rng.random::<f32>() < self.rushes {
            rng.random_range(5.0..18.0)
        } else {
            rng.random_range(25.0..90.0)
        };
        RoundResult { won, secs, engaged, spotted_first: !sneak }
    }
}

/// One simulated round.
#[derive(Debug, Clone)]
pub struct SimRound {
    pub bands: [Band; Skill::COUNT],
    pub result: RoundResult,
    /// Assist dial the round was played with.
    pub assists: f32,
    /// Every skill's center after the round.
    pub centers: [Band; Skill::COUNT],
    pub cues: Vec<Cue>,
}

#[derive(Debug, Clone)]
pub struct SimRun {
    pub player: SyntheticPlayer,
    pub rounds: Vec<SimRound>,
    pub profile: PlayerProfile,
}

/// Play `rounds` rounds from a new profile (calibration included).
pub fn run(player: SyntheticPlayer, seed: u64, rounds: usize) -> SimRun {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut profile = PlayerProfile::new();
    let mut out = Vec::with_capacity(rounds);
    for i in 0..rounds {
        let bands = next_round(&profile, &mut rng);
        let dial = profile.assists;
        profile = reduce(profile, AdaptEvent::RoundStarted { bands });
        let result = player.play(bands, i as u32, dial, &mut rng);
        profile = reduce(profile, AdaptEvent::RoundFinished(result));
        out.push(SimRound {
            bands,
            result,
            assists: dial,
            centers: std::array::from_fn(|k| profile.skills[k].center),
            cues: profile.cues.clone(),
        });
    }
    SimRun { player, rounds: out, profile }
}

/// Summary numbers for a run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub rounds: usize,
    pub win_rate: f32,
    /// Wins after [`WARMUP`].
    pub win_after_warmup: f32,
    /// Share of post-warm-up rounds whose trailing-[`TRAILING`] win rate is within [`TARGET`].
    pub time_near_target: f32,
    /// Band changes that reverse the same skill's previous change within [`OSC_SPAN`] rounds
    /// (summed over skills).
    pub oscillations: usize,
    /// All reversals, however far apart.
    pub reversals: usize,
    pub promotions: usize,
    pub demotions: usize,
    pub frustrations: usize,
    /// Mean assist dial after warm-up.
    pub mean_assists: f32,
    /// Share of post-warm-up rounds played with the dial ≥ 0.9.
    pub maxed_assists: f32,
    /// Share of post-warm-up rounds played fully unassisted (dial ≤ `ASSIST_EPS`).
    pub unassisted: f32,
    pub final_centers: [Band; Skill::COUNT],
}

impl SimRun {
    pub fn metrics(&self) -> Metrics {
        let n = self.rounds.len();
        let rate = |rs: &[SimRound]| {
            if rs.is_empty() { 0.0 } else { rs.iter().filter(|r| r.result.won).count() as f32 / rs.len() as f32 }
        };
        let after = &self.rounds[WARMUP.min(n)..];
        let near = (WARMUP.min(n)..n)
            .filter(|&i| {
                let r = rate(&self.rounds[(i + 1).saturating_sub(TRAILING)..=i]);
                (TARGET.0..=TARGET.1).contains(&r)
            })
            .count();
        let (mut oscillations, mut reversals) = (0, 0);
        let mut last_dir = [0i32; Skill::COUNT];
        let mut last_at = [0usize; Skill::COUNT];
        let mut prev = self.rounds.first().map(|r| r.centers).unwrap_or([1; Skill::COUNT]);
        let mut calibrated = false;
        let (mut promotions, mut demotions, mut frustrations) = (0, 0, 0);
        for (i, r) in self.rounds.iter().enumerate() {
            for c in &r.cues {
                match c {
                    Cue::Promoted(..) => promotions += 1,
                    Cue::Demoted(..) => demotions += 1,
                    Cue::Frustrated(_) => frustrations += 1,
                    Cue::Calibrated(_) => calibrated = true,
                    Cue::Eased(..) => {}
                }
            }
            if r.cues.iter().any(|c| matches!(c, Cue::Calibrated(_))) {
                prev = r.centers;
                continue;
            }
            if !calibrated {
                continue;
            }
            for k in 0..Skill::COUNT {
                let d = r.centers[k] as i32 - prev[k] as i32;
                if d != 0 {
                    let dir = d.signum();
                    if last_dir[k] != 0 && dir != last_dir[k] {
                        reversals += 1;
                        if i - last_at[k] <= OSC_SPAN {
                            oscillations += 1;
                        }
                    }
                    last_dir[k] = dir;
                    last_at[k] = i;
                }
            }
            prev = r.centers;
        }
        let mean = |f: &dyn Fn(&SimRound) -> f32| {
            if after.is_empty() { 0.0 } else { after.iter().map(f).sum::<f32>() / after.len() as f32 }
        };
        Metrics {
            rounds: n,
            win_rate: rate(&self.rounds),
            win_after_warmup: rate(after),
            time_near_target: if after.is_empty() { 0.0 } else { near as f32 / after.len() as f32 },
            oscillations,
            reversals,
            promotions,
            demotions,
            frustrations,
            mean_assists: mean(&|r| r.assists),
            maxed_assists: mean(&|r| if r.assists >= 0.9 { 1.0 } else { 0.0 }),
            unassisted: mean(&|r| if r.assists <= super::profile::ASSIST_EPS { 1.0 } else { 0.0 }),
            final_centers: self.profile.skills.each_ref().map(|s| s.center),
        }
    }
}
