//! The adaptive difficulty engine (`adapt`): reducer rules, calibration, and simulator-level
//! behaviour over hundreds of rounds.

use second_person::adapt::{
    AdaptEvent, Cue, FrustrationSignal, Outcome, PlayerProfile, RoundResult, Skill, next_round, profile, reduce, sim,
};
use rand::SeedableRng;
use rand::rngs::StdRng;

const S: Skill = Skill::Stealth;
const G: Skill = Skill::Gunfight;

/// A round: a sneaky win, a won fight after being spotted, or a lost fight.
#[derive(Clone, Copy)]
enum R {
    Sneak,
    WonFight,
    LostFight,
    /// Lost within seconds.
    QuickLoss,
}

fn result(r: R) -> RoundResult {
    match r {
        R::Sneak => RoundResult { won: true, secs: 40.0, engaged: false, spotted_first: false },
        R::WonFight => RoundResult { won: true, secs: 40.0, engaged: true, spotted_first: true },
        R::LostFight => RoundResult { won: false, secs: 40.0, engaged: true, spotted_first: true },
        R::QuickLoss => RoundResult { won: false, secs: 6.0, engaged: true, spotted_first: true },
    }
}

/// Play one round with every skill at its center.
fn round(p: PlayerProfile, r: R) -> PlayerProfile {
    let bands = [p.center(S), p.center(G)];
    reduce(reduce(p, AdaptEvent::RoundStarted { bands }), AdaptEvent::RoundFinished(result(r)))
}

fn rounds(mut p: PlayerProfile, r: R, n: usize) -> PlayerProfile {
    for _ in 0..n {
        p = round(p, r);
    }
    p
}

#[test]
fn reducer_does_not_touch_its_input() {
    let p0 = PlayerProfile::calibrated(4, 0.0);
    let before = p0.clone();
    let p1 = round(p0.clone(), R::Sneak);
    assert_eq!(p0, before);
    assert_ne!(p1, p0);
    assert_eq!(round(p0, R::Sneak), p1, "same input, same output");
}

#[test]
fn a_round_is_evidence_for_the_skills_it_tested() {
    let p = round(PlayerProfile::calibrated(4, 0.0), R::Sneak);
    assert_eq!(p.skill(S).window.entries.len(), 1);
    assert!(p.skill(G).window.entries.is_empty(), "no fight, no gunfight evidence");
    let p = round(p, R::WonFight);
    assert_eq!(p.skill(S).window.entries.last().unwrap().outcome, Outcome::Struggle, "spotted first");
    assert_eq!(p.skill(G).window.entries.last().unwrap().outcome, Outcome::Clean);
}

#[test]
fn assists_fade_before_the_band_rises() {
    let p = PlayerProfile::calibrated(4, 0.3);
    // Winning with assists on lowers the dial, not the band.
    let p = rounds(p, R::Sneak, 3);
    assert_eq!(p.center(S), 4);
    assert!(p.assists > profile::ASSIST_EPS && p.assists < 0.3);
    // Once the dial is off, fresh unassisted wins promote.
    let p = rounds(p, R::Sneak, 10);
    assert!(p.center(S) > 4, "center {}", p.center(S));
    assert_eq!(p.assists, 0.0);
}

#[test]
fn promotion_needs_enough_unassisted_rounds_at_the_center() {
    let p = rounds(PlayerProfile::calibrated(4, 0.0), R::Sneak, profile::MIN_EVIDENCE - 1);
    assert_eq!(p.center(S), 4);
    let p = round(p, R::Sneak);
    assert_eq!(p.center(S), 5);
    assert!(p.cues.contains(&Cue::Promoted(S, 5)));
}

#[test]
fn sustained_losing_demotes_and_raises_assists() {
    let p = rounds(PlayerProfile::calibrated(6, 0.0), R::LostFight, 2);
    assert!(p.assists >= 2.0 * profile::ASSIST_RISE - 1e-6);
    // Two more losses: the third in a row is frustration, which eases the band right away.
    let p = round(p, R::LostFight);
    assert!(p.cues.contains(&Cue::Frustrated(FrustrationSignal::LossStreak)));
    assert_eq!(p.center(G), 5);
    assert_eq!(p.center(S), 5);
}

#[test]
fn a_quick_loss_after_a_loss_eases_off_at_once() {
    let p = round(PlayerProfile::calibrated(6, 0.0), R::QuickLoss);
    assert!(p.cues.is_empty(), "one quick loss is just charging in");
    let p = round(p, R::QuickLoss);
    assert!(p.cues.contains(&Cue::Frustrated(FrustrationSignal::QuickLoss)));
    assert_eq!(p.center(G), 5);
}

#[test]
fn falling_out_of_a_band_makes_it_harder_to_climb_back() {
    let p = rounds(PlayerProfile::calibrated(6, 0.0), R::LostFight, 3);
    assert_eq!(p.center(G), 5);
    assert_eq!(p.skill(G).evidence_needed(), profile::REPROMOTE_EVIDENCE);
    // Clear the dial: plenty of clean fights at 5 is still not enough to go back to 6...
    let mut p = PlayerProfile { assists: 0.0, ..p };
    p = rounds(p, R::WonFight, profile::REPROMOTE_EVIDENCE + 2);
    assert_eq!(p.center(G), 5);
    // ...until he's also beaten at 6 on stretch rounds.
    for _ in 0..profile::MIN_STRETCH {
        let bands = [p.center(S), 6];
        p = reduce(reduce(p, AdaptEvent::RoundStarted { bands }), AdaptEvent::RoundFinished(result(R::WonFight)));
    }
    assert_eq!(p.center(G), 6);
}

#[test]
fn an_abandoned_round_counts_for_nothing() {
    let p = PlayerProfile::calibrated(4, 0.0);
    let p = reduce(p, AdaptEvent::RoundStarted { bands: [4, 4] });
    let p = round(p, R::Sneak);
    assert_eq!(p.rounds_played, 1);
}

#[test]
fn calibration_places_a_strong_player_high_with_little_help() {
    let mut rng = StdRng::seed_from_u64(0);
    let mut p = PlayerProfile::new();
    let mut probes = Vec::new();
    while p.calibrating() {
        let bands = next_round(&p, &mut rng);
        assert_eq!(bands[0], bands[1], "probes play every skill at the same band");
        probes.push(bands[0]);
        p = reduce(reduce(p, AdaptEvent::RoundStarted { bands }), AdaptEvent::RoundFinished(result(R::Sneak)));
    }
    assert_eq!(probes, vec![3, 5, 7]);
    assert_eq!((p.center(S), p.center(G)), (7, 7));
    assert!(p.assists < profile::BASELINE_ASSISTS);
}

#[test]
fn calibration_places_a_struggling_player_low_with_more_help() {
    let mut p = PlayerProfile::new();
    let mut rng = StdRng::seed_from_u64(0);
    while p.calibrating() {
        let bands = next_round(&p, &mut rng);
        p = reduce(reduce(p, AdaptEvent::RoundStarted { bands }), AdaptEvent::RoundFinished(result(R::LostFight)));
    }
    assert_eq!(p.center(G), 1);
    assert!(p.assists > profile::BASELINE_ASSISTS);
}

// ─── Simulator ───────────────────────────────────────────────────────────────

fn avg_metrics(name: &str, seeds: u64) -> sim::Metrics {
    let p = sim::player(name).unwrap();
    let ms: Vec<_> = (0..seeds).map(|s| sim::run(p, s, 200).metrics()).collect();
    let n = ms.len() as f32;
    let avg = |f: &dyn Fn(&sim::Metrics) -> f32| ms.iter().map(f).sum::<f32>() / n;
    sim::Metrics {
        win_after_warmup: avg(&|m| m.win_after_warmup),
        time_near_target: avg(&|m| m.time_near_target),
        oscillations: avg(&|m| m.oscillations as f32).round() as usize,
        maxed_assists: avg(&|m| m.maxed_assists),
        unassisted: avg(&|m| m.unassisted),
        final_centers: ms[0].final_centers,
        ..ms[0]
    }
}

#[test]
fn simulated_players_end_up_in_the_target_win_rate() {
    for name in ["precise", "sloppy", "brawler", "sneak"] {
        let m = avg_metrics(name, 10);
        assert!(
            (0.55..=0.9).contains(&m.win_after_warmup),
            "{name}: wins after warm-up {:.0}%",
            m.win_after_warmup * 100.0
        );
        assert!(m.oscillations <= 5, "{name}: {} oscillations per 200 rounds", m.oscillations);
    }
}

#[test]
fn strong_players_lose_the_help_and_meet_a_sharper_him() {
    let m = avg_metrics("precise", 10);
    assert!(m.unassisted > 0.5, "unassisted {:.0}% of rounds", m.unassisted * 100.0);
    assert!(m.final_centers.iter().all(|c| *c >= 7), "centers {:?}", m.final_centers);
}

#[test]
fn beginners_get_help_without_pinning_the_dial() {
    let m = avg_metrics("beginner", 10);
    assert!(m.maxed_assists < 0.25, "maxed {:.0}% of rounds", m.maxed_assists * 100.0);
    assert!(m.final_centers.iter().all(|c| *c <= 2), "centers {:?}", m.final_centers);
}
