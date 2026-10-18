//! The adaptive score: tracks parse and survive a reformat, and the engine follows the game
//! state on the musical grid (cue switches on the bar line, stingers on the beat).

use second_person::music::score::{Role, Score};
use second_person::music::{Engine, script};

const SR: f64 = 48000.0;

fn track(name: &str) -> Score {
    let text = std::fs::read_to_string(format!("{}/music/{name}.score", env!("CARGO_MANIFEST_DIR"))).unwrap();
    Score::parse(&text).unwrap()
}

fn engine() -> Engine {
    Engine::new(track("him").compile().unwrap(), SR)
}

/// Render `secs` in callback-sized chunks; returns the peak and RMS.
fn run(e: &mut Engine, secs: f64) -> (f32, f32) {
    let mut buf = vec![0.0f32; 512 * 2];
    let n = (secs * SR / 512.0).ceil() as usize;
    let (mut peak, mut sum) = (0.0f32, 0.0f64);
    for _ in 0..n {
        e.render(&mut buf);
        for s in &buf {
            peak = peak.max(s.abs());
            sum += (*s as f64).powi(2);
        }
    }
    (peak, (sum / (n * buf.len()) as f64).sqrt() as f32)
}

/// Render until `f` holds, in 1 ms steps; returns the engine time when it did.
fn until(e: &mut Engine, max_secs: f64, f: impl Fn(&Engine) -> bool) -> f64 {
    let mut buf = vec![0.0f32; 48 * 2];
    let start = e.time();
    while e.time() - start < max_secs {
        if f(e) {
            return e.time();
        }
        e.render(&mut buf);
    }
    panic!("never happened within {max_secs}s");
}

fn gain(e: &Engine, stem: &str) -> f32 {
    e.status().stems.iter().find(|s| s.name == stem).unwrap().gain
}

#[test]
fn every_track_parses_and_survives_a_reformat() {
    let dir = format!("{}/music", env!("CARGO_MANIFEST_DIR"));
    let mut n = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "score") {
            let text = std::fs::read_to_string(&path).unwrap();
            let score = Score::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let again = Score::parse(&score.to_text()).unwrap();
            assert_eq!(again, score, "{} changes when reformatted", path.display());
            assert_eq!(score.to_text(), text, "{} isn't in canonical form (run `cargo run --bin editor -- --check --write`)", path.display());
            let c = score.compile().unwrap();
            for role in [Role::Calm, Role::Fight, Role::Win, Role::Lose] {
                assert!(c.cue(role).is_some(), "{}: no {role:?} cue", path.display());
            }
            n += 1;
        }
    }
    assert!(n > 0);
}

#[test]
fn bad_scores_say_what_is_wrong() {
    let ok = track("him").to_text();
    let no_patch = ok.replacen("patch = \"strings\"", "patch = \"violas\"", 1);
    assert!(Score::parse(&no_patch).unwrap_err().contains("no patch called `violas`"));
    let bad_chord = ok.replacen("chords = \"Dm Dm Bb A\"", "chords = \"Dm Qm\"", 1);
    assert!(Score::parse(&bad_chord).unwrap_err().contains("unknown chord `Qm`"));
    let bad_token = ok.replacen("r-12,r-5,r+12,2+12 _*15", "r-12 zz", 1);
    assert!(Score::parse(&bad_token).unwrap_err().contains("zz"));
}

#[test]
fn calm_plays_and_suspicion_moves_the_faders() {
    let mut e = engine();
    e.play();
    let (peak, rms) = run(&mut e, 4.0);
    assert!(rms > 0.01, "calm should be audible: rms {rms}");
    assert!(peak <= 1.0);
    assert!(gain(&e, "Low strings") > 0.9);
    assert!(gain(&e, "Celesta") > 0.9);
    assert!(gain(&e, "Pizzicato") < 0.01);
    assert!(gain(&e, "Spiccato") < 0.01, "fight stems stay down in calm");
    e.suspicion = 1.0;
    run(&mut e, 4.0);
    assert!(gain(&e, "Celesta") < 0.05, "celesta falls as he gets suspicious");
    assert!(gain(&e, "Pizzicato") > 0.9);
    assert!(gain(&e, "Tremolo") > 0.9);
    assert_eq!(e.status().cue, "calm", "suspicion alone never leaves calm");
}

#[test]
fn engaging_switches_to_the_fight_on_the_bar_line() {
    let mut e = engine();
    e.play();
    let bar = 60.0 / 92.0 * 4.0;
    let start = e.time() + 0.03;
    run(&mut e, bar * 1.3);
    e.engaged = true;
    assert_eq!(e.status().pending.as_deref(), Some("fight"));
    let at = until(&mut e, 4.0, |e| e.status().cue == "fight");
    // The fight's first step is scheduled at the bar line (the engine flips its status a
    // block before the downbeat, when it schedules it).
    let bars = (at - start) / bar;
    assert!((bars - bars.round()).abs() < 0.01, "switched {bars} bars in, not on a bar line");
    assert_eq!(bars.round(), 2.0);
    assert!((e.status().bpm - 138.0).abs() < 0.5, "the fight runs at its own tempo");
    run(&mut e, 1.0);
    assert!(gain(&e, "Spiccato") > 0.9, "the fight hits");
    assert!(gain(&e, "Low strings") < 0.01, "and calm drops out");
    // Letting go returns to calm, again on a bar line.
    e.engaged = false;
    until(&mut e, 3.0, |e| e.status().cue == "calm");
}

#[test]
fn winning_plays_the_stinger_from_the_next_beat_then_stops() {
    let mut e = engine();
    e.play();
    let beat = 60.0 / 92.0;
    let start = e.time() + 0.03;
    run(&mut e, beat * 2.4);
    e.end(true);
    assert_eq!(e.status().pending.as_deref(), Some("win"));
    let at = until(&mut e, 2.0, |e| e.status().cue == "win");
    let beats = (at - start) / beat;
    assert!((beats - beats.round()).abs() < 0.01, "stinger at {beats} beats, not on a beat");
    assert_eq!(beats.round(), 3.0);
    let (_, rms) = run(&mut e, 2.0);
    assert!(rms > 0.01, "the stinger sounds");
    run(&mut e, 6.0);
    assert!(!e.status().playing, "a stinger plays once");
    let (peak, _) = run(&mut e, 2.0);
    assert!(peak < 0.01, "and then it's quiet: {peak}");
}

#[test]
fn the_entrance_hit_plays_only_on_the_first_pass() {
    let mut e = engine();
    e.engaged = true;
    e.play();
    until(&mut e, 4.0, |e| e.status().cue == "fight");
    run(&mut e, 0.5);
    let meter = |e: &Engine| e.status().stems.iter().find(|s| s.name == "Impact").unwrap().meter;
    assert!(meter(&e) > 0.01, "the impact hits on the way in");
    // Four fight bars later, the loop comes round without it.
    let fight_bar = 60.0 / 138.0 * 4.0;
    run(&mut e, fight_bar * 4.0);
    assert!(meter(&e) < 0.002, "no impact on the second pass: {}", meter(&e));
}

#[test]
fn edits_swap_in_without_losing_the_place() {
    let mut e = engine();
    e.play();
    run(&mut e, 3.0);
    let before = e.status();
    let mut score = track("him");
    score.stems[0].part = "r-12 .*15".into();
    score.patches.get_mut("strings").unwrap().gain = 0.1;
    e.set_score(score.compile().unwrap());
    let after = e.status();
    assert_eq!(after.cue, before.cue);
    assert_eq!(after.step, before.step);
    assert!(after.playing);
    let (_, rms) = run(&mut e, 1.0);
    assert!(rms > 0.005);
}

#[test]
fn a_scripted_round_never_clips() {
    let mut e = engine();
    e.play();
    let mut t = 0.0f32;
    let mut peak = 0.0f32;
    let mut won = false;
    let mut cues = Vec::new();
    while t < script::LENGTH {
        let m = script::at(t);
        e.suspicion = m.suspicion;
        e.engaged = m.engaged;
        if m.won && !won {
            won = true;
            e.end(true);
        }
        let (p, _) = run(&mut e, 0.1);
        peak = peak.max(p);
        let c = e.status().cue;
        if cues.last() != Some(&c) {
            cues.push(c);
        }
        t += 0.1;
    }
    assert!(peak <= 1.0, "clipped: {peak}");
    assert_eq!(cues, ["calm", "fight", "win"]);
}

#[test]
fn transpose_moves_every_note() {
    let mut s = track("him");
    s.transpose = 3;
    let mut e = Engine::new(s.compile().unwrap(), SR);
    e.play();
    let (_, rms) = run(&mut e, 2.0);
    assert!(rms > 0.01);
}

fn soundfont() -> std::sync::Arc<second_person::music::SoundFont> {
    second_person::music::load_soundfont(format!("{}/{}", env!("CARGO_MANIFEST_DIR"), second_person::music::SOUNDFONT)).unwrap()
}

#[test]
fn sampled_patches_play_from_the_soundfont() {
    let mut e = Engine::new(track("mambo").compile().unwrap(), SR);
    e.set_soundfont(Some(soundfont()));
    e.play();
    let (peak, rms) = run(&mut e, 4.0);
    assert!(rms > 0.01, "the soundfont should be audible: rms {rms}");
    assert!(peak <= 1.0);
    let meter = |e: &Engine, name: &str| e.status().stems.iter().find(|s| s.name == name).unwrap().meter;
    assert!(meter(&e, "Bass") > 0.001 || meter(&e, "Vibes") > 0.001, "sampled stems meter like any other");
}

#[test]
fn without_a_soundfont_sampled_patches_use_their_synth() {
    let mut e = Engine::new(track("mambo").compile().unwrap(), SR);
    e.play();
    let (_, rms) = run(&mut e, 4.0);
    assert!(rms > 0.01, "the synth fallback should be audible: rms {rms}");
    assert!(!e.has_soundfont());
}

#[test]
fn drum_keys_follow_the_nearest_listed_note() {
    let c = track("mambo").compile().unwrap();
    let conga = c.stems.iter().find(|s| s.name == "Congas").unwrap().sample.clone().unwrap();
    assert_eq!(conga.key(60), 63, "c4 is the open conga");
    assert_eq!(conga.key(64), 62, "e4 is the slap");
    assert_eq!(conga.key(55), 64, "g3 is the low conga");
    assert_eq!(conga.key(48), 64, "anything lower is the nearest, the low conga");
    let bad = track("mambo").to_text().replacen("sample = { gain = 2.0, program = 32 }", "sample = { gain = 2.0 }", 1);
    assert!(Score::parse(&bad).unwrap_err().contains("give either a `program` or drum `keys`"));
}

#[test]
fn a_sampled_round_never_clips() {
    let mut e = Engine::new(track("mambo").compile().unwrap(), SR);
    e.set_soundfont(Some(soundfont()));
    e.play();
    let mut t = 0.0f32;
    let mut peak = 0.0f32;
    let mut won = false;
    while t < script::LENGTH {
        let m = script::at(t);
        e.suspicion = m.suspicion;
        e.engaged = m.engaged;
        if m.won && !won {
            won = true;
            e.end(true);
        }
        peak = peak.max(run(&mut e, 0.1).0);
        t += 0.1;
    }
    assert!(peak <= 1.0, "clipped: {peak}");
}
