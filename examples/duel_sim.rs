//! Headless bot rounds against him, for balancing his moves (the scramble dart first).
//!
//! The bot is `common/bot.rs`.
//!
//! ```sh
//! cargo run --example duel_sim -- --player sharp --scheme tank --gunfight 10 --rounds 40
//! cargo run --example duel_sim -- --dart off ...      # the same without the dart, as a baseline
//! ```
//!
//! Prints one summary line per run (see `Summary::print`).

#[path = "common/bot.rs"]
mod bot;

use std::time::Duration;

use bevy::{input::InputPlugin, prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy};
use bot::{Bot, DartTally, HitsLanded, PLAYERS, Player};
use second_person::{
    arena::ArenaMode,
    difficulty::Tuning,
    round::GameState,
    experiments::Experiments,
    scramble::Armed,
    shooter::{ControlScheme, Scrambled, Shooter},
    target::{Suspicion, Target},
};

const DT: f32 = 1.0 / 60.0;
/// Rounds longer than this are called a draw.
const ROUND_SECS: f32 = 90.0;
/// A death this soon after a scramble ends still counts as one while scrambled.
const SCRAMBLE_DEATH_GRACE: f32 = 1.0;

struct Args {
    player: Player,
    scheme: ControlScheme,
    tuning: Tuning,
    dart: bool,
    rounds: u32,
    seed: u64,
    /// Nobody dies: long fights, for the dart's own numbers (hits, dodges, time scrambled).
    immortal: bool,
}

fn parse() -> Args {
    let mut a = Args {
        player: PLAYERS[0],
        scheme: ControlScheme::Tank,
        tuning: Tuning { stealth: 10, gunfight: 10, assists: 0.0 },
        dart: true,
        rounds: 20,
        seed: 1,
        immortal: false,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    for pair in argv.chunks(2) {
        let [k, v] = pair else { panic!("arguments come in --key value pairs") };
        match k.as_str() {
            "--player" => a.player = *PLAYERS.iter().find(|p| p.name == v).expect("sharp, average or novice"),
            "--scheme" => {
                a.scheme = *ControlScheme::ALL
                    .iter()
                    .find(|s| format!("{s:?}").eq_ignore_ascii_case(v))
                    .expect("a ControlScheme name")
            }
            "--gunfight" => a.tuning.gunfight = v.parse().unwrap(),
            "--stealth" => a.tuning.stealth = v.parse().unwrap(),
            "--assists" => a.tuning.assists = v.parse().unwrap(),
            "--dart" => a.dart = v == "on",
            "--rounds" => a.rounds = v.parse().unwrap(),
            "--seed" => a.seed = v.parse().unwrap(),
            "--immortal" => a.immortal = v == "on",
            "--aim" => a.player.aim_deg = v.parse().unwrap(),
            "--acquire" => a.player.acquire = v.parse().unwrap(),
            "--reaction" => a.player.reaction = v.parse().unwrap(),
            "--dodge" => a.player.dodge = v.parse().unwrap(),
            "--timing" => a.player.timing = v.parse().unwrap(),
            "--confusion" => a.player.confusion = v.parse().unwrap(),
            _ => panic!("unknown argument {k}"),
        }
    }
    a
}

#[derive(Default)]
struct Summary {
    rounds: u32,
    won: u32,
    lost: u32,
    draws: u32,
    secs: f32,
    /// His HP left (fraction) summed over rounds you lost.
    his_hp_left: f32,
    darts: DartTally,
    scrambled_secs: f32,
    playing_secs: f32,
    dmg_scrambled: f32,
    dmg_clear: f32,
    deaths_scrambled: u32,
    /// Shortest gap between one scramble ending and the next starting.
    min_gap: f32,
    longest_scramble: f32,
    hits_landed: u32,
    engaged_secs: f32,
}

impl Summary {
    fn print(&self, a: &Args) {
        let n = self.rounds.max(1) as f32;
        let thrown = self.darts.thrown.max(1) as f32;
        let tried = (self.darts.tried_hit + self.darts.tried_missed).max(1) as f32;
        let stood = (self.darts.stood_hit + self.darts.stood_missed).max(1) as f32;
        let hits = self.darts.tried_hit + self.darts.stood_hit;
        println!(
            "{:<7} {:<8} gf{:<2} dart={:<3} | win {:>3.0}% lose {:>3.0}% draw {:>3.0}% | {:>4.1}s/round, {:>4.2} hits | \
             engaged {:>3.0}% | darts {:>4.1}/engaged min, hit {:>3.0}% (dodging {:>3.0}%, standing {:>3.0}%) | scrambled {:>4.1}% of play, \
             dmg/s scr {:>4.1} vs {:>4.1} | deaths scr {:>3.0}% of losses | min gap {:>4.1}s",
            a.player.name,
            format!("{:?}", a.scheme),
            a.tuning.gunfight,
            if a.dart { "on" } else { "off" },
            100.0 * self.won as f32 / n,
            100.0 * self.lost as f32 / n,
            100.0 * self.draws as f32 / n,
            self.secs / n,
            self.hits_landed as f32 / n,
            100.0 * self.engaged_secs / self.playing_secs.max(1e-3),
            60.0 * self.darts.thrown as f32 / self.engaged_secs.max(1e-3),
            100.0 * hits as f32 / thrown,
            100.0 * self.darts.tried_hit as f32 / tried,
            100.0 * self.darts.stood_hit as f32 / stood,
            100.0 * self.scrambled_secs / self.playing_secs.max(1e-3),
            self.dmg_scrambled / self.scrambled_secs.max(1e-3),
            self.dmg_clear / (self.playing_secs - self.scrambled_secs).max(1e-3),
            100.0 * self.deaths_scrambled as f32 / self.lost.max(1) as f32,
            if self.min_gap.is_finite() { self.min_gap } else { f32::NAN },
        );
    }
}

fn main() {
    let a = parse();
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        InputPlugin,
        AssetPlugin::default(),
        TransformPlugin,
        bevy::mesh::MeshPlugin,
    ))
    .init_asset::<StandardMaterial>()
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(DT)))
    .add_plugins((second_person::gameplay, second_person::juice::plugin))
    .insert_resource(ArenaMode::Seed(a.seed))
    .insert_resource(a.tuning)
    .insert_resource(a.scheme)
    .insert_resource(Armed(a.dart))
    .insert_resource(Experiments { scramble_dart: true })
    .insert_resource(Bot::new(a.player, a.seed))
    .add_plugins(bot::plugin);
    app.finish();
    app.cleanup();
    app.update();

    let mut s = Summary { min_gap: f32::INFINITY, ..default() };
    for round in 0..a.rounds {
        if round > 0 {
            app.insert_resource(ArenaMode::Seed(a.seed * 1000 + round as u64));
            app.world_mut().resource_mut::<NextState<GameState>>().set(GameState::Playing);
            app.update();
            app.world_mut().resource_mut::<Bot>().reset();
        }
        let mut t = 0.0;
        let mut was_scrambled = false;
        let mut scramble_began = 0.0;
        let mut last_scramble_end: Option<f32> = None;
        let outcome = loop {
            if a.immortal {
                keep_alive(&mut app);
            }
            let hp = shooter_hp(&mut app);
            app.update();
            t += DT;
            let state = *app.world().resource::<State<GameState>>().get();
            let Some((hp_now, scrambled)) = shooter_state(&mut app) else {
                break state;
            };
            let lost = (hp - hp_now).max(0.0);
            s.playing_secs += DT;
            let mut q = app.world_mut().query::<&Suspicion>();
            if q.single(app.world()).is_ok_and(|s| s.engaged) {
                s.engaged_secs += DT;
            }
            if scrambled {
                s.scrambled_secs += DT;
                s.dmg_scrambled += lost;
                if !was_scrambled {
                    scramble_began = t;
                    if let Some(end) = last_scramble_end {
                        s.min_gap = s.min_gap.min(t - end);
                    }
                }
            } else {
                s.dmg_clear += lost;
                if was_scrambled {
                    last_scramble_end = Some(t);
                    s.longest_scramble = s.longest_scramble.max(t - scramble_began);
                }
            }
            was_scrambled = scrambled;
            if state != GameState::Playing {
                if state == GameState::Lost
                    && (scrambled || last_scramble_end.is_some_and(|e| t - e <= SCRAMBLE_DEATH_GRACE))
                {
                    s.deaths_scrambled += 1;
                }
                break state;
            }
            if t >= ROUND_SECS {
                break GameState::Playing;
            }
        };
        s.rounds += 1;
        s.secs += t;
        match outcome {
            GameState::Won => s.won += 1,
            GameState::Lost => {
                s.lost += 1;
                let mut q = app.world_mut().query::<&Target>();
                if let Ok(him) = q.single(app.world()) {
                    s.his_hp_left += him.hp / him.max_hp;
                }
            }
            _ => s.draws += 1,
        }
    }
    s.darts = app.world().resource::<Bot>().darts;
    s.hits_landed = app.world().resource::<HitsLanded>().0;
    s.print(&a);
}

fn keep_alive(app: &mut App) {
    let mut q = app.world_mut().query::<&mut Shooter>();
    if let Ok(mut s) = q.single_mut(app.world_mut()) {
        s.hp = 1e6;
    }
    let mut q = app.world_mut().query::<&mut Target>();
    if let Ok(mut t) = q.single_mut(app.world_mut()) {
        t.hp = 1e6;
    }
}

fn shooter_hp(app: &mut App) -> f32 {
    shooter_state(app).map_or(0.0, |(hp, _)| hp)
}

fn shooter_state(app: &mut App) -> Option<(f32, bool)> {
    let mut q = app.world_mut().query::<(&Shooter, &Scrambled)>();
    q.single(app.world()).ok().map(|(s, sc)| (s.hp, sc.active()))
}

