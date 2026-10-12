//! Headless bot rounds against him, for balancing his moves (the scramble dart first).
//!
//! The bot plays the real gameplay plugins through the shooter's `ActionState`, like the touch
//! controls do. It knows where he is (an upper bound on a player), walks the navmesh until it
//! has a clear shot, aims with some error and fires. When he throws a dart it may sidestep
//! after a reaction time; when scrambled it keeps driving with the controls it knows for a while
//! (confusion), then adapts to the scheme it was put in, reading his view a little late.
//!
//! ```sh
//! cargo run --example duel_sim -- --player sharp --scheme tank --gunfight 10 --rounds 40
//! cargo run --example duel_sim -- --dart off ...      # the same without the dart, as a baseline
//! ```
//!
//! Prints one summary line per run (see `Summary::print`).

use std::{collections::VecDeque, time::Duration};

use bevy::{input::InputPlugin, prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy};
use leafwing_input_manager::{plugin::InputManagerSystem, prelude::*};
use rand::{Rng, SeedableRng, rngs::StdRng};
use second_person::{
    arena::{ArenaMode, Layout},
    combat::TargetHit,
    difficulty::Tuning,
    nav::Nav,
    round::GameState,
    scramble::{Armed, DartCharging, DartMissed, DartThrown, ScrambledHit},
    shooter::{ControlScheme, Scrambled, Shooter, ShooterAction},
    target::{MainCamera, Suspicion, Target},
};

const DT: f32 = 1.0 / 60.0;
/// Rounds longer than this are called a draw.
const ROUND_SECS: f32 = 90.0;
/// The bot stops to shoot from inside this range.
const SHOOT_RANGE: f32 = 16.0;
/// A death this soon after a scramble ends still counts as one while scrambled.
const SCRAMBLE_DEATH_GRACE: f32 = 1.0;

#[derive(Clone, Copy, Debug)]
struct Player {
    name: &'static str,
    /// Seconds from his throw to starting a sidestep.
    reaction: f32,
    /// Chance of trying to sidestep a dart at all.
    dodge: f32,
    /// Seconds a scramble goes unnoticed (driving with your own scheme).
    confusion: f32,
    /// After that, how stale the view the bot steers by is (seconds).
    lag: f32,
    /// Aim error per shot, degrees either side.
    aim_deg: f32,
    /// Seconds of clear sight of him before the first shot (finding him on screen, lining up).
    acquire: f32,
    /// Tank players who've learned the whine time the hop to its end instead of reacting to
    /// the throw: the spread (seconds) of that timing. 0 means they only react.
    timing: f32,
}

const PLAYERS: [Player; 3] = [
    Player { name: "sharp", reaction: 0.2, dodge: 0.9, confusion: 0.7, lag: 0.2, aim_deg: 3.0, acquire: 0.3, timing: 0.07 },
    Player { name: "average", reaction: 0.35, dodge: 0.6, confusion: 1.3, lag: 0.35, aim_deg: 5.0, acquire: 0.6, timing: 0.15 },
    Player { name: "novice", reaction: 0.5, dodge: 0.3, confusion: 2.0, lag: 0.5, aim_deg: 8.0, acquire: 1.0, timing: 0.0 },
];

#[derive(Clone, Copy)]
enum Cue {
    Windup,
    Thrown(Vec3),
    /// A hop timed to the end of the wind-up.
    Hop,
}

struct Dodge {
    /// World direction to step.
    side: Vec3,
    elapsed: f32,
    /// How long to keep walking that way (direct schemes; a tank hop is a fixed double tap).
    secs: f32,
}

#[derive(Resource)]
struct Bot {
    p: Player,
    rng: StdRng,
    route: Vec<Vec3>,
    replan: f32,
    /// Darts seen, with when to react to them.
    pending: Vec<(f32, Cue)>,
    /// Whether the bot read this dart's wind-up.
    noticed: bool,
    dodge: Option<Dodge>,
    fired_last: bool,
    /// Seconds he's been in a clear shot.
    sight: f32,
    /// This shot's aim error (radians), re-rolled after each shot.
    aim_off: f32,
    scrambled_since: Option<f32>,
    /// His view forward over the last second, for steering by a stale view.
    eyes: VecDeque<(f32, Vec3)>,
    /// Whether the bot tried to dodge the dart in flight.
    tried: bool,
    darts: DartTally,
}

#[derive(Default, Clone, Copy)]
struct DartTally {
    thrown: u32,
    /// (tried to dodge, hit) counts.
    tried_hit: u32,
    tried_missed: u32,
    stood_hit: u32,
    stood_missed: u32,
}

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
    .insert_resource(Bot {
        p: a.player,
        rng: StdRng::seed_from_u64(a.seed),
        route: vec![],
        replan: 0.0,
        pending: vec![],
        noticed: false,
        dodge: None,
        fired_last: false,
        sight: 0.0,
        aim_off: 0.0,
        scrambled_since: None,
        eyes: VecDeque::new(),
        tried: false,
        darts: DartTally::default(),
    })
    .add_systems(PreUpdate, bot.in_set(InputManagerSystem::ManualControl))
    .add_systems(PostUpdate, tally);
    app.finish();
    app.cleanup();
    app.update();

    let mut s = Summary { min_gap: f32::INFINITY, ..default() };
    for round in 0..a.rounds {
        if round > 0 {
            app.insert_resource(ArenaMode::Seed(a.seed * 1000 + round as u64));
            app.world_mut().resource_mut::<NextState<GameState>>().set(GameState::Playing);
            app.update();
            let mut bot = app.world_mut().resource_mut::<Bot>();
            bot.route.clear();
            bot.pending.clear();
            bot.dodge = None;
            bot.scrambled_since = None;
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

#[derive(Resource, Default)]
struct HitsLanded(u32);

fn tally(
    mut commands: Commands,
    mut bot: ResMut<Bot>,
    landed: Option<ResMut<HitsLanded>>,
    mut thrown: MessageReader<DartThrown>,
    mut hits: MessageReader<ScrambledHit>,
    mut misses: MessageReader<DartMissed>,
    mut target_hits: MessageReader<TargetHit>,
    you: Query<(&Transform, &avian3d::prelude::LinearVelocity), With<Shooter>>,
) {
    let trace = std::env::var("TRACE").is_ok();
    let you = you.single().ok().map(|(t, v)| (t.translation, v.0));
    let n = target_hits.read().count() as u32;
    match landed {
        Some(mut l) => l.0 += n,
        None => commands.insert_resource(HitsLanded(n)),
    }
    for m in thrown.read() {
        bot.darts.thrown += 1;
        if trace && let Some((p, v)) = you {
            eprintln!(
                "dart from {:.1?} at you {:.1?} ({:.1}m) moving {:.1?}",
                m.from.xz(),
                p.xz(),
                m.from.xz().distance(p.xz()),
                v.xz()
            );
        }
    }
    let tried = bot.tried;
    for _ in hits.read() {
        if trace {
            eprintln!("  hit (tried {tried})");
        }
        if tried { bot.darts.tried_hit += 1 } else { bot.darts.stood_hit += 1 }
    }
    for m in misses.read() {
        if trace && let Some((p, _)) = you {
            eprintln!("  missed at {:.1?}, you {:.1?} (tried {tried})", m.at.xz(), p.xz());
        }
        if tried { bot.darts.tried_missed += 1 } else { bot.darts.stood_missed += 1 }
    }
}

/// Signed angle (radians, right positive seen from above) from `fwd` to `dir`.
fn angle_to(fwd: Vec3, dir: Vec3) -> f32 {
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
    f32::atan2(dir.dot(right), dir.dot(fwd))
}

#[allow(clippy::too_many_arguments)]
fn bot(
    time: Res<Time>,
    mut bot: ResMut<Bot>,
    nav: Nav,
    layout: Res<Layout>,
    chosen: Res<ControlScheme>,
    state: Res<State<GameState>>,
    mut charging: MessageReader<DartCharging>,
    mut thrown: MessageReader<DartThrown>,
    mut shooter: Query<(&mut ActionState<ShooterAction>, &Transform, &Scrambled), With<Shooter>>,
    him: Query<&Transform, With<Target>>,
    eyes: Query<&GlobalTransform, With<MainCamera>>,
) {
    let bot = &mut *bot;
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    if *state.get() != GameState::Playing {
        thrown.clear();
        charging.clear();
        return;
    }
    let (Ok((mut act, t, scrambled)), Ok(him)) = (shooter.single_mut(), him.single()) else {
        return;
    };
    let me = t.translation;
    let him = him.translation;
    let fwd = (t.rotation * Vec3::NEG_Z).with_y(0.0).normalize_or_zero();

    if let Ok(e) = eyes.single() {
        bot.eyes.push_back((now, e.forward().as_vec3().with_y(0.0).normalize_or_zero()));
    }
    while bot.eyes.front().is_some_and(|(at, _)| now - at > 1.0) {
        bot.eyes.pop_front();
    }

    // Which controls the bot believes it has, and the view it steers them by.
    let scheme = if scrambled.active() {
        let since = *bot.scrambled_since.get_or_insert(now);
        if now - since < bot.p.confusion { *chosen } else { scrambled.into }
    } else {
        bot.scrambled_since = None;
        *chosen
    };
    let stale = bot
        .eyes
        .iter()
        .find(|(at, _)| now - at <= bot.p.lag)
        .or(bot.eyes.back())
        .map(|(_, f)| *f)
        .unwrap_or(Vec3::NEG_Z);
    let with_right = |f: Vec3| (f, Vec3::new(-f.z, 0.0, f.x));
    let basis = match scheme {
        ControlScheme::Tank | ControlScheme::Snake => None,
        ControlScheme::Compass => Some((Vec3::NEG_Z, Vec3::X)),
        ControlScheme::Eyes | ControlScheme::Ice => Some(with_right(stale)),
        ControlScheme::Mirror => Some(with_right(-stale)),
        ControlScheme::Magnetic => Some(with_right((him - me).with_y(0.0).normalize_or(Vec3::NEG_Z))),
    };

    // Darts. A player who reads the wind-up (glow, whine) starts sidestepping on it; when the
    // dart leaves his hand they cut back the other way, since he leads a runner. A tank player
    // holds the hop for the throw. Everything waits out the reaction time.
    let timed = basis.is_none() && bot.p.timing > 0.0;
    for m in charging.read() {
        bot.noticed = bot.rng.random::<f32>() < bot.p.dodge;
        bot.tried = bot.noticed;
        if bot.noticed && timed {
            // The second tap (which starts the hop) lands 0.1s after the first.
            let err = (bot.rng.random::<f32>() + bot.rng.random::<f32>() + bot.rng.random::<f32>() - 1.5) * 2.0;
            let at = now + m.windup - 0.1 + err * bot.p.timing;
            bot.pending.push((at.max(now + bot.p.reaction), Cue::Hop));
        } else if bot.noticed {
            bot.pending.push((now + bot.p.reaction, Cue::Windup));
        }
    }
    for m in thrown.read() {
        if bot.noticed && !timed {
            bot.pending.push((now + bot.p.reaction, Cue::Thrown(m.velocity)));
        }
    }
    let clear = |side: Vec3| {
        let p = (me + side * 2.4).xz();
        layout.is_clear(p, 0.5) && !layout.los_blocked(me.xz(), p)
    };
    while let Some(i) = bot.pending.iter().position(|(at, _)| *at <= now) {
        let (_, cue) = bot.pending.swap_remove(i);
        let line = match cue {
            Cue::Thrown(v) => v,
            _ => him - me,
        }
        .with_y(0.0);
        let perp = Vec3::new(-line.z, 0.0, line.x).normalize_or_zero();
        let pick = |rng: &mut StdRng| {
            let first = if rng.random_bool(0.5) { perp } else { -perp };
            if clear(first) { first } else { -first }
        };
        match (cue, basis.is_some(), bot.dodge.as_mut()) {
            // Wind-up: walkers start moving, tanks wait.
            (Cue::Windup, true, _) => bot.dodge = Some(Dodge { side: pick(&mut bot.rng), elapsed: 0.0, secs: 2.0 }),
            (Cue::Windup, false, _) => {}
            // Thrown while already moving: cut back.
            (Cue::Thrown(_), true, Some(d)) => {
                let back = -d.side;
                *d = Dodge { side: if clear(back) { back } else { d.side }, elapsed: 0.0, secs: 0.5 };
            }
            (Cue::Thrown(_) | Cue::Hop, _, _) => bot.dodge = Some(Dodge { side: pick(&mut bot.rng), elapsed: 0.0, secs: 0.5 }),
        }
    }

    let mut fire = false;
    let input = if let Some(d) = &mut bot.dodge {
        d.elapsed += dt;
        let side = d.side;
        let elapsed = d.elapsed;
        match basis {
            None => {
                // Double-tap toward the side: on, off, on, then let the hop run.
                let s = side.dot(Vec3::new(-fwd.z, 0.0, fwd.x)).signum();
                if elapsed > 0.2 {
                    bot.dodge = None;
                }
                Vec2::new(if elapsed < 0.05 || (0.1..0.15).contains(&elapsed) { s } else { 0.0 }, 0.0)
            }
            Some((f, r)) => {
                if elapsed > d.secs {
                    bot.dodge = None;
                }
                Vec2::new(side.dot(r), side.dot(f))
            }
        }
    } else {
        let to_him = (him - me).with_y(0.0);
        let dist = to_him.length();
        let clear_shot = !layout.los_blocked_with_margin(me.xz(), him.xz(), 0.3);
        if clear_shot && dist < SHOOT_RANGE {
            bot.sight += dt;
            // Aim: at him, off by this shot's error.
            bot.route.clear();
            let goal = Quat::from_rotation_y(-bot.aim_off) * to_him.normalize_or_zero();
            let err = angle_to(fwd, goal);
            if err.abs() < 1.5_f32.to_radians() && !bot.fired_last && bot.sight >= bot.p.acquire {
                fire = true;
                let max = bot.p.aim_deg.to_radians();
                bot.aim_off = bot.rng.random_range(-max..=max);
            }
            match basis {
                None if scheme == ControlScheme::Snake => Vec2::new((err * 3.0).clamp(-0.45, 0.45), -1.0),
                None if err.abs() > 0.8 => Vec2::new(err.signum(), 0.5),
                None => Vec2::new((err * 3.0).clamp(-0.45, 0.45), 0.0),
                Some((f, r)) => {
                    // Nudge the stick toward him: enough to turn, barely enough to walk.
                    if err.abs() < 1.0_f32.to_radians() {
                        Vec2::ZERO
                    } else {
                        Vec2::new(goal.dot(r), goal.dot(f)) * 0.25
                    }
                }
            }
        } else {
            bot.sight = 0.0;
            bot.replan -= dt;
            if bot.route.is_empty() || bot.replan <= 0.0 {
                bot.route = nav.path(me, him).unwrap_or_else(|| vec![him]);
                bot.replan = 0.5;
            }
            while bot.route.len() > 1 && bot.route[0].xz().distance(me.xz()) < 0.7 {
                bot.route.remove(0);
            }
            let dir = bot.route.first().map_or(to_him, |w| (*w - me).with_y(0.0)).normalize_or_zero();
            let err = angle_to(fwd, dir);
            match basis {
                None if err.abs() > 1.2 => Vec2::new(err.signum(), 0.5),
                None => Vec2::new((err * 3.0).clamp(-1.0, 1.0), if err.abs() < 0.35 { 1.0 } else { 0.6 }),
                Some((f, r)) => Vec2::new(dir.dot(r), dir.dot(f)),
            }
        }
    };
    act.set_axis_pair(&ShooterAction::Drive, input);
    if fire {
        act.press(&ShooterAction::Fire);
    }
    bot.fired_last = fire;
}
