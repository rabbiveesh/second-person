//! The duel bot: plays the shooter through its `ActionState`, like the touch controls do. Shared
//! by `duel_sim` (headless rounds, numbers) and `duel_clip` (a rendered match).
//!
//! It knows where he is (an upper bound on a player), walks the navmesh until it has a clear
//! shot, aims with some error and fires. It reads his dart's wind-up and sidesteps after a
//! reaction time; when scrambled it keeps driving with the controls it knows for a while
//! (confusion), then adapts to the scheme it was put in, reading his view a little late.

use std::collections::VecDeque;

use bevy::prelude::*;
use leafwing_input_manager::{plugin::InputManagerSystem, prelude::*};
use rand::{Rng, SeedableRng, rngs::StdRng};
use second_person::{
    arena::Layout,
    combat::TargetHit,
    nav::Nav,
    round::GameState,
    scramble::{DartCharging, DartMissed, DartThrown, ScrambledHit},
    shooter::{ControlScheme, Scrambled, Shooter, ShooterAction},
    target::{MainCamera, Target},
};

pub fn plugin(app: &mut App) {
    app.init_resource::<HitsLanded>()
        .add_systems(PreUpdate, bot.in_set(InputManagerSystem::ManualControl))
        .add_systems(PostUpdate, tally);
}

/// The bot stops to shoot from inside this range.
pub const SHOOT_RANGE: f32 = 16.0;

#[derive(Clone, Copy, Debug)]
pub struct Player {
    pub name: &'static str,
    /// Seconds from his throw to starting a sidestep.
    pub reaction: f32,
    /// Chance of trying to sidestep a dart at all.
    pub dodge: f32,
    /// Seconds a scramble goes unnoticed (driving with your own scheme).
    pub confusion: f32,
    /// After that, how stale the view the bot steers by is (seconds).
    pub lag: f32,
    /// Aim error per shot, degrees either side.
    pub aim_deg: f32,
    /// Seconds of clear sight of him before the first shot (finding him on screen, lining up).
    pub acquire: f32,
    /// Tank players who've learned the whine time the hop to its end instead of reacting to
    /// the throw: the spread (seconds) of that timing. 0 means they only react.
    pub timing: f32,
}

pub const PLAYERS: [Player; 3] = [
    Player { name: "sharp", reaction: 0.2, dodge: 0.9, confusion: 0.7, lag: 0.2, aim_deg: 3.0, acquire: 0.3, timing: 0.07 },
    Player { name: "average", reaction: 0.35, dodge: 0.6, confusion: 1.3, lag: 0.35, aim_deg: 5.0, acquire: 0.6, timing: 0.15 },
    Player { name: "novice", reaction: 0.5, dodge: 0.3, confusion: 2.0, lag: 0.5, aim_deg: 8.0, acquire: 1.0, timing: 0.0 },
];

#[derive(Clone, Copy)]
pub enum Cue {
    Windup,
    Thrown(Vec3),
    /// A hop timed to the end of the wind-up.
    Hop,
}

pub struct Dodge {
    /// World direction to step.
    pub side: Vec3,
    pub elapsed: f32,
    /// How long to keep walking that way (direct schemes; a tank hop is a fixed double tap).
    pub secs: f32,
}

#[derive(Resource)]
pub struct Bot {
    pub p: Player,
    pub rng: StdRng,
    pub route: Vec<Vec3>,
    pub replan: f32,
    /// Darts seen, with when to react to them.
    pub pending: Vec<(f32, Cue)>,
    /// Whether the bot read this dart's wind-up.
    pub noticed: bool,
    pub dodge: Option<Dodge>,
    pub fired_last: bool,
    /// Seconds he's been in a clear shot.
    pub sight: f32,
    /// This shot's aim error (radians), re-rolled after each shot.
    pub aim_off: f32,
    pub scrambled_since: Option<f32>,
    /// His view forward over the last second, for steering by a stale view.
    pub eyes: VecDeque<(f32, Vec3)>,
    /// Whether the bot tried to dodge the dart in flight.
    pub tried: bool,
    pub darts: DartTally,
}

#[derive(Default, Clone, Copy)]
pub struct DartTally {
    pub thrown: u32,
    /// (tried to dodge, hit) counts.
    pub tried_hit: u32,
    pub tried_missed: u32,
    pub stood_hit: u32,
    pub stood_missed: u32,
}

#[derive(Resource, Default)]
pub struct HitsLanded(pub u32);

pub fn tally(
    mut bot: ResMut<Bot>,
    mut landed: ResMut<HitsLanded>,
    mut thrown: MessageReader<DartThrown>,
    mut hits: MessageReader<ScrambledHit>,
    mut misses: MessageReader<DartMissed>,
    mut target_hits: MessageReader<TargetHit>,
    you: Query<(&Transform, &avian3d::prelude::LinearVelocity), With<Shooter>>,
) {
    let trace = std::env::var("TRACE").is_ok();
    let you = you.single().ok().map(|(t, v)| (t.translation, v.0));
    let n = target_hits.read().count() as u32;
    landed.0 += n;
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
pub fn angle_to(fwd: Vec3, dir: Vec3) -> f32 {
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
    f32::atan2(dir.dot(right), dir.dot(fwd))
}

#[allow(clippy::too_many_arguments)]
pub fn bot(
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
        ControlScheme::Tank => None,
        ControlScheme::Mirror => Some(with_right(-stale)),
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

impl Bot {
    pub fn new(p: Player, seed: u64) -> Self {
        Self {
            p,
            rng: StdRng::seed_from_u64(seed),
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
        }
    }

    /// Forget the last round.
    pub fn reset(&mut self) {
        self.route.clear();
        self.pending.clear();
        self.dodge = None;
        self.scrambled_since = None;
    }
}
