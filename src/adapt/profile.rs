//! [`PlayerProfile`] and its pure reducer, [`reduce`].

use super::calibration::{Calibration, Placement, Probe};
use super::frustration::{self, FrustrationSignal};
use super::skill::{Band, MAX_BAND, MIN_BAND, Skill};
use super::window::{Outcome, RollingWindow, WindowEntry};

// ─── Tuning (see the module docs for how these were picked) ──────────────────

/// A new player's assist dial: the hand-tuned game's aim assist and sonar.
pub const BASELINE_ASSISTS: f32 = 0.5;
/// Assists at or below this count as "off": promotion needs it, and only rounds played this
/// unassisted count as promotion evidence.
pub const ASSIST_EPS: f32 = 0.02;
/// A win lowers the assist dial by this much...
pub const ASSIST_FADE: f32 = 0.07;
/// ...and a loss raises it by this much.
pub const ASSIST_RISE: f32 = 0.08;
/// Frustration raises the dial by up to this much (diminishing as the dial rises), instead of
/// the plain [`ASSIST_RISE`].
pub const FRUSTRATION_ASSISTS: f32 = 0.15;

/// Promote: clean rate at the center (unassisted, this epoch) at least this...
pub const PROMOTE_AT: f32 = 0.75;
/// ...over at least this many rounds...
pub const MIN_EVIDENCE: usize = 5;
/// ...and, with at least [`MIN_STRETCH`] stretch rounds (above center), a stretch clean rate of
/// at least this.
pub const STRETCH_AT: f32 = 0.6;
pub const MIN_STRETCH: usize = 2;
/// Demote: clean rate at the center (this epoch) below this over [`MIN_EVIDENCE`]+ rounds.
pub const DEMOTE_BELOW: f32 = 0.5;
/// Hysteresis: promoting back into a band the skill was demoted (or eased) out of needs this
/// many rounds of evidence instead of [`MIN_EVIDENCE`]. Stops 4↔5 ping-pong for players whose
/// true level sits on a band boundary.
pub const REPROMOTE_EVIDENCE: usize = 10;

/// Spread: widen by [`WIDEN_STEP`] (up to [`WIDEN_MAX`]) while the skill's whole window is
/// above [`WIDEN_AT`] clean over [`WIDEN_MIN_ROUNDS`]+ rounds.
pub const WIDEN_AT: f32 = 0.75;
pub const WIDEN_MIN_ROUNDS: usize = 10;
pub const WIDEN_STEP: f32 = 0.1;
pub const WIDEN_MAX: f32 = 0.8;
/// Narrowing on promote / demote / frustration, and the floors.
pub const PROMOTE_NARROW: f32 = 0.1;
pub const PROMOTE_SPREAD_MIN: f32 = 0.2;
pub const DEMOTE_NARROW: f32 = 0.15;
pub const DEMOTE_SPREAD_MIN: f32 = 0.1;
pub const FRUSTRATION_NARROW: f32 = 0.15;
pub const DEFAULT_SPREAD: f32 = 0.5;

/// Rounds remembered globally (loss streaks).
pub const RECENT_ROUNDS: usize = 8;

// ─── State ───────────────────────────────────────────────────────────────────

/// One skill's difficulty state.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillState {
    /// Center band; rounds are sampled around it ([`super::choose::band_distribution`]).
    pub center: Band,
    /// 0 = nearly always the center, 1 = lots of reinforcement and stretch.
    pub spread: f32,
    pub window: RollingWindow,
    /// Bumps on every band change; window entries from older epochs don't count toward the
    /// new center.
    pub epoch: u32,
    /// The band this skill last fell out of (demotion or frustration), until it's re-earned.
    /// Promoting back into it needs [`REPROMOTE_EVIDENCE`] rounds.
    pub fell_from: Option<Band>,
}

impl SkillState {
    pub fn new(center: Band) -> Self {
        SkillState { center, spread: DEFAULT_SPREAD, window: RollingWindow::default(), epoch: 0, fell_from: None }
    }

    /// Rounds at the center needed before a promotion.
    pub fn evidence_needed(&self) -> usize {
        if self.fell_from == Some(self.center + 1) { REPROMOTE_EVIDENCE } else { MIN_EVIDENCE }
    }
}

/// A round the game said it started and hasn't finished yet.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveRound {
    /// Per skill ([`Skill::index`]): the band it's played at, and the center and epoch at start.
    pub skills: [RoundSkill; Skill::COUNT],
    /// Assist dial when the round started.
    pub assists: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoundSkill {
    pub band: Band,
    pub center: Band,
    pub epoch: u32,
}

/// Something the caller may react to (logs, the simulator), produced by the last [`reduce`].
/// Never shown to the player as a label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cue {
    /// The round went badly in a way that isn't fun; the engine eased off.
    Frustrated(FrustrationSignal),
    /// A skill's center went up.
    Promoted(Skill, Band),
    /// A skill's center went down after sustained struggle.
    Demoted(Skill, Band),
    /// A skill's center was eased down because of frustration.
    Eased(Skill, Band),
    /// The placement rounds are over; bands and assists were set.
    Calibrated(Placement),
}

/// Everything the adaptive engine knows about the player. Update it only with [`reduce`].
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerProfile {
    /// Indexed by [`Skill::index`].
    pub skills: [SkillState; Skill::COUNT],
    /// The assist dial, 0..1: how much quiet help the game gives. Separate from the bands.
    pub assists: f32,
    pub calibration: Calibration,
    /// Wins and losses of the last [`RECENT_ROUNDS`] rounds, oldest first.
    pub recent: Vec<bool>,
    pub round: Option<ActiveRound>,
    pub rounds_played: u32,
    /// What the last event asked the caller to do.
    pub cues: Vec<Cue>,
}

impl Default for PlayerProfile {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerProfile {
    /// A new player: calibration pending, baseline assists.
    pub fn new() -> Self {
        PlayerProfile {
            skills: std::array::from_fn(|_| SkillState::new(MIN_BAND)),
            assists: BASELINE_ASSISTS,
            calibration: Calibration::default(),
            recent: Vec::new(),
            round: None,
            rounds_played: 0,
            cues: Vec::new(),
        }
    }

    /// A player past calibration with every skill centered at `band` (tests, tools).
    pub fn calibrated(band: Band, assists: f32) -> Self {
        PlayerProfile {
            skills: std::array::from_fn(|_| SkillState::new(band.clamp(MIN_BAND, MAX_BAND))),
            assists: assists.clamp(0.0, 1.0),
            calibration: Calibration::finished(),
            ..Self::new()
        }
    }

    pub fn skill(&self, skill: Skill) -> &SkillState {
        &self.skills[skill.index()]
    }

    pub fn center(&self, skill: Skill) -> Band {
        self.skill(skill).center
    }

    pub fn calibrating(&self) -> bool {
        !self.calibration.done
    }

    /// Losses in a row at the end of [`PlayerProfile::recent`].
    pub fn losses_in_a_row(&self) -> usize {
        self.recent.iter().rev().take_while(|won| !**won).count()
    }
}

// ─── Events ──────────────────────────────────────────────────────────────────

/// How a round went.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RoundResult {
    pub won: bool,
    /// Seconds from the start of the round to the end.
    pub secs: f32,
    /// He engaged at some point (there was a gunfight).
    pub engaged: bool,
    /// He engaged before your first hit landed (you were spotted first).
    pub spotted_first: bool,
}

impl RoundResult {
    /// What the round says about `skill`, if anything. Stealth: clean when your first hit landed
    /// before he engaged. Gunfight: only when he engaged; clean when you won.
    pub fn outcome(&self, skill: Skill) -> Option<Outcome> {
        let clean = |c: bool| if c { Outcome::Clean } else { Outcome::Struggle };
        match skill {
            Skill::Stealth => Some(clean(!self.spotted_first)),
            Skill::Gunfight => self.engaged.then(|| clean(self.won)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AdaptEvent {
    /// A round began with these bands ([`Skill::index`]). Starting one while another is open
    /// abandons the open one: it counts for nothing.
    RoundStarted { bands: [Band; Skill::COUNT] },
    RoundFinished(RoundResult),
}

// ─── Reducer ─────────────────────────────────────────────────────────────────

/// The next profile after `event`. Pure: same input, same output; `state` is consumed and a new
/// profile is returned (clone first to keep the old one). [`PlayerProfile::cues`] holds what
/// this event asks of the caller.
pub fn reduce(state: PlayerProfile, event: AdaptEvent) -> PlayerProfile {
    let state = PlayerProfile { cues: Vec::new(), ..state };
    match event {
        AdaptEvent::RoundStarted { bands } => {
            let skills = std::array::from_fn(|i| {
                let st = &state.skills[i];
                RoundSkill { band: bands[i].clamp(MIN_BAND, MAX_BAND), center: st.center, epoch: st.epoch }
            });
            PlayerProfile { round: Some(ActiveRound { skills, assists: state.assists }), ..state }
        }
        AdaptEvent::RoundFinished(result) => round_finished(state, result),
    }
}

fn round_finished(state: PlayerProfile, result: RoundResult) -> PlayerProfile {
    let Some(round) = state.round.clone() else { return state };
    let mut recent = state.recent.clone();
    recent.push(result.won);
    if recent.len() > RECENT_ROUNDS {
        recent.drain(..recent.len() - RECENT_ROUNDS);
    }
    let s = PlayerProfile { recent, round: None, rounds_played: state.rounds_played + 1, ..state };
    let signal = frustration::detect(result.won, result.secs, s.losses_in_a_row());

    if s.calibrating() {
        return calibration_round(s, &round, result, signal);
    }

    // The assist dial: wins fade it, losses raise it. A frustrated round gets the larger of the
    // plain rise and the frustration bump, not both.
    let assists = if result.won {
        s.assists - ASSIST_FADE
    } else if signal.is_some() {
        s.assists + ASSIST_RISE.max(frustration_bump(s.assists))
    } else {
        s.assists + ASSIST_RISE
    }
    .clamp(0.0, 1.0);

    let mut skills = s.skills.clone();
    for skill in Skill::ALL {
        let Some(outcome) = result.outcome(skill) else { continue };
        let rs = round.skills[skill.index()];
        let st = &skills[skill.index()];
        let entry = WindowEntry { outcome, band: rs.band, center: rs.center, epoch: rs.epoch, assists: round.assists };
        skills[skill.index()] = SkillState { window: st.window.push(entry), ..st.clone() };
    }
    let s = PlayerProfile { skills, assists, ..s };

    if let Some(signal) = signal {
        return ease(s, &round, result, signal);
    }

    // Promote / demote / widen, per skill the round gave evidence on.
    let mut cues = Vec::new();
    let mut skills = s.skills.clone();
    for skill in Skill::ALL {
        if result.outcome(skill).is_none() {
            continue;
        }
        let st = &skills[skill.index()];
        let next = if st.center < MAX_BAND && s.assists <= ASSIST_EPS && should_promote(st) {
            cues.push(Cue::Promoted(skill, st.center + 1));
            SkillState {
                center: st.center + 1,
                spread: (st.spread - PROMOTE_NARROW).max(PROMOTE_SPREAD_MIN),
                epoch: st.epoch + 1,
                fell_from: st.fell_from.filter(|b| *b > st.center + 1),
                ..st.clone()
            }
        } else if st.center > MIN_BAND && should_demote(st) {
            cues.push(Cue::Demoted(skill, st.center - 1));
            SkillState {
                center: st.center - 1,
                spread: (st.spread - DEMOTE_NARROW).max(DEMOTE_SPREAD_MIN),
                epoch: st.epoch + 1,
                fell_from: Some(st.center),
                ..st.clone()
            }
        } else {
            match st.window.clean_rate() {
                (Some(rate), n) if n >= WIDEN_MIN_ROUNDS && rate > WIDEN_AT && st.spread < WIDEN_MAX => {
                    SkillState { spread: (st.spread + WIDEN_STEP).min(WIDEN_MAX), ..st.clone() }
                }
                _ => st.clone(),
            }
        };
        skills[skill.index()] = next;
    }
    PlayerProfile { skills, cues, ..s }
}

/// Promote: ≥ [`PROMOTE_AT`] clean over ≥ [`MIN_EVIDENCE`] unassisted rounds at the center since
/// the last band change, and stretch rounds (if ≥ [`MIN_STRETCH`]) ≥ [`STRETCH_AT`]. Into a band
/// just fallen out of: [`REPROMOTE_EVIDENCE`] rounds, and the stretch rounds are required.
pub fn should_promote(st: &SkillState) -> bool {
    let (at, n) = st.window.at_center(st.center, st.epoch, true);
    if n < st.evidence_needed() || at.unwrap_or(0.0) < PROMOTE_AT {
        return false;
    }
    let (above, m) = st.window.above_center(st.center, st.epoch);
    if st.fell_from == Some(st.center + 1) {
        // Back into a band this skill already failed at: show it on stretch rounds first.
        return m >= MIN_STRETCH && above.unwrap_or(0.0) >= STRETCH_AT;
    }
    !(m >= MIN_STRETCH && above.unwrap_or(0.0) < STRETCH_AT)
}

/// Demote: < [`DEMOTE_BELOW`] clean over ≥ [`MIN_EVIDENCE`] rounds at the center since the last
/// band change (assisted or not).
pub fn should_demote(st: &SkillState) -> bool {
    let (at, n) = st.window.at_center(st.center, st.epoch, false);
    n >= MIN_EVIDENCE && at.unwrap_or(1.0) < DEMOTE_BELOW
}

/// The frustration response, for the skills that struggled this round and were played at or
/// above their center: narrow the spread, and drop the center by 1, except after a *stretch*
/// round (played above the center), where the stretch was the problem, not the center.
fn ease(state: PlayerProfile, round: &ActiveRound, result: RoundResult, signal: FrustrationSignal) -> PlayerProfile {
    let mut cues = vec![Cue::Frustrated(signal)];
    let mut skills = state.skills.clone();
    for skill in Skill::ALL {
        let rs = round.skills[skill.index()];
        if result.outcome(skill) != Some(Outcome::Struggle) || rs.band < rs.center {
            continue;
        }
        let st = &skills[skill.index()];
        let stretch = rs.band > rs.center && st.center == rs.center;
        let center = if stretch { st.center } else { st.center.saturating_sub(1).max(MIN_BAND) };
        if center != st.center {
            cues.push(Cue::Eased(skill, center));
        }
        skills[skill.index()] = SkillState {
            center,
            spread: (st.spread - FRUSTRATION_NARROW).max(DEMOTE_SPREAD_MIN),
            epoch: if center != st.center { st.epoch + 1 } else { st.epoch },
            fell_from: if center != st.center { Some(st.center) } else { st.fell_from },
            ..st.clone()
        };
    }
    PlayerProfile { skills, cues, ..state }
}

/// How much frustration raises the dial from `assists`: [`FRUSTRATION_ASSISTS`], diminishing
/// near the top so repeated frustration doesn't pin the dial at max.
fn frustration_bump(assists: f32) -> f32 {
    FRUSTRATION_ASSISTS * (1.0 - assists)
}

fn calibration_round(
    s: PlayerProfile,
    round: &ActiveRound,
    result: RoundResult,
    signal: Option<FrustrationSignal>,
) -> PlayerProfile {
    let band = round.skills[0].band;
    let calibration = s.calibration.record(Probe { band, won: result.won });
    let mut cues: Vec<Cue> = signal.map(Cue::Frustrated).into_iter().collect();
    if !calibration.complete() {
        return PlayerProfile { calibration, cues, ..s };
    }
    let p = calibration.placement();
    let skills = std::array::from_fn(|i| SkillState {
        center: p.band,
        spread: p.spread,
        window: RollingWindow::default(),
        epoch: s.skills[i].epoch + 1,
        fell_from: None,
    });
    cues.push(Cue::Calibrated(p));
    PlayerProfile {
        skills,
        assists: p.assists,
        calibration: Calibration { done: true, ..calibration },
        cues,
        ..s
    }
}
