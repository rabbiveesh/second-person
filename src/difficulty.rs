//! Adaptive difficulty in play: what each band and the assist dial do to the game, and the
//! systems that feed rounds to the engine (`crate::adapt`).
//!
//! [`Tuning`] is the current round's difficulty, read by the target, combat and radar systems.
//! Its default is the hand-tuned game (every band at [`BASE_BAND`], the dial at
//! [`BASELINE_ASSISTS`]), and it stays that way unless [`AdaptiveDifficulty`] exists: the real
//! game inserts it (`main.rs`); headless tests and staged examples don't, so they stay
//! deterministic.
//!
//! Two kinds of knob, never shown to the player:
//! - **Bands** ([`HisLevers`]) are how good *he* is. Stealth bands his senses, Gunfight his gunplay.
//! - **The assist dial** ([`AssistLevers`]) is quiet help for *you*: aim assist, the sonar's pace
//!   (all the way to no sonar at all), and how hard his shots hit.

use std::collections::VecDeque;

use bevy::prelude::*;
use rand::{SeedableRng, rngs::StdRng};

use crate::{
    adapt::{self, AdaptEvent, BASE_BAND, Band, Cue, PlayerProfile, RoundResult, Skill, profile::BASELINE_ASSISTS},
    combat::{self, TargetHit},
    round::{GameState, SpawnRound},
    target::{self, Suspicion, Target},
};

/// This round's difficulty. See the module docs.
#[derive(Resource, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Resource)]
pub struct Tuning {
    pub stealth: Band,
    pub gunfight: Band,
    /// The assist dial, 0..1.
    pub assists: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { stealth: BASE_BAND, gunfight: BASE_BAND, assists: BASELINE_ASSISTS }
    }
}

impl Tuning {
    pub fn him(&self) -> HisLevers {
        HisLevers::from_bands(self.stealth, self.gunfight)
    }

    pub fn help(&self) -> AssistLevers {
        AssistLevers::from_dial(self.assists)
    }
}

/// Interpolate a knob over the bands: `lo` at band 1, `base` (today's value) at [`BASE_BAND`],
/// `hi` at band 10, linear in between.
fn by_band(band: Band, lo: f32, base: f32, hi: f32) -> f32 {
    let b = band.clamp(adapt::MIN_BAND, adapt::MAX_BAND) as f32;
    let (min, mid, max) = (adapt::MIN_BAND as f32, BASE_BAND as f32, adapt::MAX_BAND as f32);
    if b <= mid { lo + (base - lo) * (b - min) / (mid - min) } else { base + (hi - base) * (b - mid) / (max - mid) }
}

/// How good he is, from the bands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HisLevers {
    // Stealth band: his senses.
    /// Half-angle of the cone he spots you in (radians).
    pub view_half_angle: f32,
    /// Multiplies how fast seeing you fills his suspicion.
    pub spot_rate: f32,
    /// Multiplies every hearing range (gunshots, near misses, footsteps, bumps, whistles).
    pub hearing: f32,
    // Gunfight band: his gunplay.
    /// Seconds of sight per return-fire shot.
    pub fire_interval: f32,
    pub fire_damage: f32,
    /// How long he holds the angle where you ducked out of sight.
    pub hold_angle_secs: f32,
    /// Grappling hook cooldown; `None` = he doesn't grapple.
    pub grapple_cooldown: Option<f32>,
    /// Dodge roll cooldown; `None` = he doesn't roll.
    pub roll_cooldown: Option<f32>,
    pub hp: u32,
}

/// Lowest Gunfight band at which he uses the grappling hook...
pub const GRAPPLE_FROM_BAND: Band = 4;
/// ...and the dodge roll.
pub const ROLL_FROM_BAND: Band = 3;

impl HisLevers {
    pub fn from_bands(stealth: Band, gunfight: Band) -> Self {
        HisLevers {
            view_half_angle: by_band(stealth, 0.42, target::VIEW_HALF_ANGLE, 0.7),
            spot_rate: by_band(stealth, 0.5, 1.0, 1.6),
            hearing: by_band(stealth, 0.7, 1.0, 1.3),
            fire_interval: by_band(gunfight, 1.3, combat::RETURN_FIRE_INTERVAL, 0.55),
            fire_damage: by_band(gunfight, 6.0, combat::RETURN_FIRE_DAMAGE, 10.0),
            hold_angle_secs: by_band(gunfight, 0.5, combat::HOLD_ANGLE_SECS, 2.5),
            grapple_cooldown: (gunfight >= GRAPPLE_FROM_BAND).then(|| by_band(gunfight, 20.0, combat::GRAPPLE_COOLDOWN, 9.0)),
            roll_cooldown: (gunfight >= ROLL_FROM_BAND).then(|| by_band(gunfight, 4.0, combat::ROLL_COOLDOWN, 1.6)),
            hp: match gunfight {
                ..=2 => target::TARGET_MAX_HP - 1,
                9.. => target::TARGET_MAX_HP + 1,
                _ => target::TARGET_MAX_HP,
            },
        }
    }
}

impl Default for HisLevers {
    fn default() -> Self {
        Self::from_bands(BASE_BAND, BASE_BAND)
    }
}

/// Quiet help for you, from the assist dial. Monotone: more dial, never less help.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AssistLevers {
    /// Bullet magnetism cone (radians) and range (see `combat::magnetised`).
    pub magnet_cone: f32,
    pub magnet_range: f32,
    /// Seconds between sonar sweeps in the `Auto` radar mode; `None` = no sonar.
    pub sonar_period: Option<f32>,
    /// Multiplies the damage his shots do to you.
    pub damage_taken: f32,
    /// Seconds of countdown over the arena before each round (see `intro`).
    pub countdown: f32,
    /// Whether the overhead shot shows which way he's facing.
    pub reveal_facing: bool,
}

/// Below this dial the `Auto` radar stops sweeping: a player good enough to fade the help out
/// plays on the bare map.
pub const SONAR_FROM: f32 = 0.1;

impl AssistLevers {
    pub fn from_dial(dial: f32) -> Self {
        let a = dial.clamp(0.0, 1.0);
        AssistLevers {
            // 12° and 30m (the hand-tuned values) at the baseline dial.
            magnet_cone: (4.0 + 16.0 * a).to_radians(),
            magnet_range: 20.0 + 20.0 * a,
            // 2s at the baseline dial, quicker as the dial rises.
            sonar_period: (a >= SONAR_FROM).then(|| 6.0 / (1.0 + 4.0 * a)),
            // Only above the baseline: up to 30% off at a full dial.
            damage_taken: 1.0 - 0.6 * (a - BASELINE_ASSISTS).max(0.0),
            // 3s at the baseline dial: 2s to glance at for a player who's fading the help, 4s at full.
            countdown: 2.0 + 2.0 * a,
            // Like the sonar: a player good enough to play the bare map finds his facing for themselves.
            reveal_facing: a >= SONAR_FROM,
        }
    }
}

impl Default for AssistLevers {
    fn default() -> Self {
        Self::from_dial(BASELINE_ASSISTS)
    }
}

/// The adaptive engine's state. While it exists, every round's [`Tuning`] comes from it.
#[derive(Resource)]
pub struct AdaptiveDifficulty {
    pub profile: PlayerProfile,
    /// The last [`HISTORY_LEN`] finished rounds, oldest first (for the debug dump).
    pub history: VecDeque<RoundRecord>,
    rng: StdRng,
}

/// Finished rounds kept in [`AdaptiveDifficulty::history`].
pub const HISTORY_LEN: usize = 50;

/// One finished round: how it was tuned, how it went, and what the engine did about it.
#[derive(Debug, Clone)]
pub struct RoundRecord {
    pub tuning: Tuning,
    pub result: RoundResult,
    pub cues: Vec<Cue>,
}

impl AdaptiveDifficulty {
    pub fn new(profile: PlayerProfile, seed: u64) -> Self {
        AdaptiveDifficulty { profile, history: VecDeque::new(), rng: StdRng::seed_from_u64(seed) }
    }
}

impl Default for AdaptiveDifficulty {
    fn default() -> Self {
        Self::new(PlayerProfile::new(), rand::random())
    }
}

/// What the current round has shown so far.
#[derive(Resource, Default, Debug)]
struct RoundLog {
    started: f32,
    engaged: bool,
    /// `Some(true)` when he engaged before your first hit, `Some(false)` when your hit came first.
    spotted_first: Option<bool>,
}

pub fn plugin(app: &mut App) {
    let adaptive = resource_exists::<AdaptiveDifficulty>;
    app.init_resource::<Tuning>()
        .init_resource::<RoundLog>()
        .add_systems(OnEnter(GameState::Playing), start_round.before(SpawnRound).run_if(adaptive))
        .add_systems(
            Update,
            watch_round.after(combat::check_outcome).run_if(in_state(GameState::Playing)).run_if(adaptive),
        )
        .add_systems(OnEnter(GameState::Won), finish_round::<true>.run_if(adaptive))
        .add_systems(OnEnter(GameState::Lost), finish_round::<false>.run_if(adaptive));
}

/// Pick this round's bands and set [`Tuning`]. A round abandoned mid-play (a new arena) is
/// simply replaced: it counts for nothing.
fn start_round(
    time: Res<Time>,
    mut adaptive: ResMut<AdaptiveDifficulty>,
    mut tuning: ResMut<Tuning>,
    mut log: ResMut<RoundLog>,
) {
    let AdaptiveDifficulty { profile, rng, .. } = &mut *adaptive;
    let bands = adapt::next_round(profile, rng);
    *profile = adapt::reduce(profile.clone(), AdaptEvent::RoundStarted { bands });
    *tuning = Tuning {
        stealth: bands[Skill::Stealth.index()],
        gunfight: bands[Skill::Gunfight.index()],
        assists: profile.assists,
    };
    *log = RoundLog { started: time.elapsed_secs(), ..default() };
    debug!("round: {tuning:?}");
}

/// Note whether he engaged, and which came first: his engagement or your first hit.
fn watch_round(mut log: ResMut<RoundLog>, mut hits: MessageReader<TargetHit>, target: Single<&Suspicion, With<Target>>) {
    if hits.read().count() > 0 && log.spotted_first.is_none() {
        log.spotted_first = Some(false);
    }
    if target.engaged {
        log.engaged = true;
        log.spotted_first.get_or_insert(true);
    }
}

fn finish_round<const WON: bool>(
    time: Res<Time>,
    tuning: Res<Tuning>,
    mut adaptive: ResMut<AdaptiveDifficulty>,
    log: Res<RoundLog>,
) {
    let result = RoundResult {
        won: WON,
        secs: time.elapsed_secs() - log.started,
        engaged: log.engaged,
        // A loss means he engaged; a win with no sighting logged means you got him first.
        spotted_first: log.spotted_first.unwrap_or(!WON),
    };
    let adaptive = &mut *adaptive;
    adaptive.profile = adapt::reduce(adaptive.profile.clone(), AdaptEvent::RoundFinished(result));
    for cue in &adaptive.profile.cues {
        debug!("adapt: {cue:?}");
    }
    if adaptive.history.len() == HISTORY_LEN {
        adaptive.history.pop_front();
    }
    adaptive.history.push_back(RoundRecord { tuning: *tuning, result, cues: adaptive.profile.cues.clone() });
}

/// The adaptive state as text, for the F9 dump.
pub fn debug_text(world: &World) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    if let Some(t) = world.get_resource::<Tuning>() {
        let _ = writeln!(out, "this round: {t:?}");
        let _ = writeln!(out, "his levers: {:#?}", t.him());
        let _ = writeln!(out, "assist levers: {:#?}", t.help());
    }
    let Some(a) = world.get_resource::<AdaptiveDifficulty>() else {
        let _ = writeln!(out, "engine: off (fixed tuning)");
        return out;
    };
    let p = &a.profile;
    let _ = writeln!(
        out,
        "dial {:.3} · rounds played {} · calibrating {} (probes {:?}) · recent wins {:?}",
        p.assists, p.rounds_played, p.calibrating(), p.calibration.probes, p.recent
    );
    for skill in Skill::ALL {
        let st = p.skill(skill);
        let (rate, n) = st.window.clean_rate();
        let (at, m) = st.window.at_center(st.center, st.epoch, true);
        let _ = writeln!(
            out,
            "{:<8} center {} spread {:.2} epoch {} fell_from {:?} · clean {} over {n} · unassisted at center {} over {m}",
            skill.name(),
            st.center,
            st.spread,
            st.epoch,
            st.fell_from,
            rate.map_or("-".into(), |r| format!("{:.0}%", r * 100.0)),
            at.map_or("-".into(), |r| format!("{:.0}%", r * 100.0)),
        );
    }
    let _ = writeln!(out, "\nrounds (oldest first): bands stealth/gunfight, dial, result, cues");
    for r in &a.history {
        let res = &r.result;
        let _ = writeln!(
            out,
            "  {}/{} dial {:.2} · {} in {:.0}s{}{} · {:?}",
            r.tuning.stealth,
            r.tuning.gunfight,
            r.tuning.assists,
            if res.won { "won" } else { "lost" },
            res.secs,
            if res.engaged { " · fight" } else { "" },
            if res.spotted_first { " · spotted first" } else { "" },
            r.cues,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_band_and_baseline_dial_are_the_hand_tuned_game() {
        let him = HisLevers::default();
        assert_eq!(him.view_half_angle, target::VIEW_HALF_ANGLE);
        assert_eq!(him.spot_rate, 1.0);
        assert_eq!(him.hearing, 1.0);
        assert_eq!(him.fire_interval, combat::RETURN_FIRE_INTERVAL);
        assert_eq!(him.fire_damage, combat::RETURN_FIRE_DAMAGE);
        assert_eq!(him.hold_angle_secs, combat::HOLD_ANGLE_SECS);
        assert_eq!(him.grapple_cooldown, Some(combat::GRAPPLE_COOLDOWN));
        assert_eq!(him.roll_cooldown, Some(combat::ROLL_COOLDOWN));
        assert_eq!(him.hp, target::TARGET_MAX_HP);

        let help = AssistLevers::default();
        assert!((help.magnet_cone - combat::MAGNET_CONE).abs() < 1e-6);
        assert!((help.magnet_range - combat::MAGNET_RANGE).abs() < 1e-4);
        assert!((help.sonar_period.unwrap() - crate::radar::SONAR_PERIOD).abs() < 1e-4);
        assert_eq!(help.damage_taken, 1.0);
    }

    #[test]
    fn higher_bands_make_him_better() {
        for b in adapt::MIN_BAND..adapt::MAX_BAND {
            let (lo, hi) = (HisLevers::from_bands(b, b), HisLevers::from_bands(b + 1, b + 1));
            assert!(hi.view_half_angle > lo.view_half_angle);
            assert!(hi.spot_rate > lo.spot_rate && hi.hearing > lo.hearing);
            assert!(hi.fire_interval < lo.fire_interval && hi.fire_damage > lo.fire_damage);
            assert!(hi.hold_angle_secs > lo.hold_angle_secs);
            assert!(hi.grapple_cooldown.is_some() >= lo.grapple_cooldown.is_some());
            assert!(hi.roll_cooldown.is_some() >= lo.roll_cooldown.is_some());
            assert!(hi.hp >= lo.hp);
        }
        let weakest = HisLevers::from_bands(1, 1);
        assert_eq!((weakest.grapple_cooldown, weakest.roll_cooldown), (None, None));
    }

    #[test]
    fn assists_are_monotone_and_fade_the_sonar_out() {
        let mut prev = AssistLevers::from_dial(0.0);
        assert_eq!(prev.sonar_period, None, "no help at all: no sonar");
        assert!(!prev.reveal_facing && prev.countdown >= 2.0, "no help: a glance, and find his facing yourself");
        for i in 1..=20 {
            let l = AssistLevers::from_dial(i as f32 / 20.0);
            assert!(l.magnet_cone > prev.magnet_cone && l.magnet_range > prev.magnet_range);
            assert!(l.damage_taken <= prev.damage_taken);
            assert!(l.countdown > prev.countdown, "more time to read the arena");
            assert!(l.reveal_facing || !prev.reveal_facing, "his facing hidden as the dial rose");
            match (prev.sonar_period, l.sonar_period) {
                (Some(a), Some(b)) => assert!(b < a, "sweeps come quicker"),
                (Some(_), None) => panic!("sonar switched off as the dial rose"),
                _ => {}
            }
            prev = l;
        }
    }
}
