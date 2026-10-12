//! Records a whole match from his eyes, with the duel bot (`common/bot.rs`) playing the shooter.
//!
//!     ALSA_CONFIG_PATH=scripts/alsa-null.conf xvfb-run -a -s "-screen 0 1280x720x24" \
//!         cargo run --example duel_clip -- <out dir> --player average --scheme eyes --rounds 2
//!     ffmpeg -framerate 30 -i <out dir>/frame-%05d.png -pix_fmt yuv420p match.mp4
//!
//! Time advances a fixed 1/60s per frame and every other frame is saved, so the clip plays at
//! real speed at 30 fps however slowly the frames render. Each round runs to its end banner.
//! With `--darts-only on`, rounds where he never throws the scramble dart are thrown away (and
//! up to `MAX_TRIES` rounds played), so `--rounds` counts kept rounds. Frames are numbered
//! straight through the kept rounds.

#[path = "common/bot.rs"]
mod bot;

use std::{path::PathBuf, time::Duration};

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    time::TimeUpdateStrategy,
};
use bevy_egui::{EguiGlobalSettings, EguiPlugin};
use bot::{Bot, PLAYERS};
use second_person::{
    arena::ArenaMode, difficulty::Tuning, round::GameState, shooter::ControlScheme, start::Started,
    touch::TouchControls,
};

/// Frames of the end banner kept after a round is decided.
const AFTER_END: u32 = 4 * 60;
/// A round still undecided after this many frames is cut.
const MAX_ROUND: u32 = 75 * 60;
/// `--darts-only`: give up after playing this many rounds.
const MAX_TRIES: u32 = 12;

#[derive(Resource)]
struct Rec {
    dir: PathBuf,
    frame: u32,
    saved: u32,
    round: u32,
    rounds: u32,
    kept: u32,
    darts_only: bool,
    /// Frames saved before this round, and darts thrown before it.
    round_start: (u32, u32),
    seed: u64,
    in_round: u32,
    ended: Option<u32>,
}

fn main() {
    let mut argv = std::env::args().skip(1);
    let dir = PathBuf::from(argv.next().unwrap_or_else(|| "duel_clip".into()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut player, mut scheme, mut gunfight, mut rounds, mut seed) = (PLAYERS[1], ControlScheme::Eyes, 10, 2, 1);
    let mut darts_only = false;
    let rest: Vec<String> = argv.collect();
    for pair in rest.chunks(2) {
        let [k, v] = pair else { panic!("arguments come in --key value pairs") };
        match k.as_str() {
            "--player" => player = *PLAYERS.iter().find(|p| p.name == v).expect("sharp, average or novice"),
            "--scheme" => {
                scheme = *ControlScheme::ALL
                    .iter()
                    .find(|s| format!("{s:?}").eq_ignore_ascii_case(v))
                    .expect("a ControlScheme name")
            }
            "--gunfight" => gunfight = v.parse().unwrap(),
            "--rounds" => rounds = v.parse().unwrap(),
            "--seed" => seed = v.parse().unwrap(),
            "--darts-only" => darts_only = v == "on",
            _ => panic!("unknown argument {k}"),
        }
    }

    App::new()
        .insert_resource(Started(true))
        .insert_resource(TouchControls(false))
        .insert_resource(ArenaMode::Seed(seed))
        .insert_resource(Tuning { stealth: gunfight, gunfight, assists: 0.0 })
        .insert_resource(scheme)
        .insert_resource(Bot::new(player, seed))
        .insert_resource(EguiGlobalSettings { auto_create_primary_context: false, ..default() })
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(1.0 / 60.0)))
        .insert_resource(Rec { dir, frame: 0, saved: 0, round: 0, rounds, kept: 0, darts_only, round_start: (0, 0), seed, in_round: 0, ended: None })
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window { resolution: (1280, 720).into(), resizable: false, ..default() }),
                ..default()
            }),
            EguiPlugin::default(),
            second_person::gameplay,
            second_person::presentation,
            bot::plugin,
        ))
        .add_systems(Last, record)
        .run();
}

fn record(world: &mut World) {
    let state = *world.resource::<State<GameState>>().get();
    let mut rec = world.resource_mut::<Rec>();
    rec.frame += 1;
    rec.in_round += 1;
    let frame = rec.frame;
    if state != GameState::Playing || rec.in_round >= MAX_ROUND {
        rec.ended.get_or_insert(frame);
    }
    let next_round = rec.ended.is_some_and(|e| frame - e >= AFTER_END);
    if next_round {
        let thrown = world.resource::<Bot>().darts.thrown;
        let mut rec = world.resource_mut::<Rec>();
        let (first, before) = rec.round_start;
        if rec.darts_only && thrown == before {
            // Nothing happened worth keeping: drop this round's frames and reuse their numbers.
            for n in first..rec.saved {
                let _ = std::fs::remove_file(rec.dir.join(format!("frame-{n:05}.png")));
            }
            rec.saved = first;
        } else {
            rec.kept += 1;
        }
        println!("round {}: {} darts, kept {}", rec.round + 1, thrown - before, rec.kept);
        rec.round += 1;
        rec.ended = None;
        rec.in_round = 0;
        rec.round_start = (rec.saved, thrown);
        if rec.kept >= rec.rounds || (rec.darts_only && rec.round >= MAX_TRIES) {
            let darts = world.resource::<Bot>().darts;
            let landed = darts.tried_hit + darts.stood_hit;
            println!("{} frames, {} darts thrown, {landed} landed", world.resource::<Rec>().saved, darts.thrown);
            world.write_message(AppExit::Success);
            return;
        }
        let seed = rec.seed * 1000 + rec.round as u64;
        world.insert_resource(ArenaMode::Seed(seed));
        world.resource_mut::<NextState<GameState>>().set(GameState::Playing);
        world.resource_mut::<Bot>().reset();
    }
    // Skip the first frames while the window and assets settle.
    let mut rec = world.resource_mut::<Rec>();
    if frame > 10 && frame % 2 == 0 {
        let path = rec.dir.join(format!("frame-{:05}.png", rec.saved));
        rec.saved += 1;
        world.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
}
