//! Plays bot rounds against him headless, with and without knockback on your hits, and prints
//! how the rounds went, so a change to his feel can be checked against the game's balance.
//!
//!     cargo run --example knockback_sim -- [rounds per setting] [knockback m/s,...]
//!     cargo run --example knockback_sim -- 60 0,4,7
//!
//! The bot is simple and consistent rather than good: it walks the navmesh toward him, stops
//! and turns to face him (at the tank's turn rate) whenever it has a line on him, and fires as
//! fast as the gun allows once it's on target, each shot a little off centre (`AIM_NOISE`). The same arena seeds play under every setting.

use std::time::Duration;

use avian3d::prelude::*;
use bevy::{input::InputPlugin, prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy};
use second_person::{
    arena::{ArenaMode, Layout},
    combat::{ShooterHit, TargetHit, TargetKnockback},
    nav::Nav,
    round::GameState,
    shooter::{HazardHit, SHOOTER_MAX_HP, Shooter},
    target::{HisHazardHit, Target},
};

const DT: f32 = 1.0 / 60.0;
const ROUND_LIMIT_SECS: f32 = 90.0;
/// The tank's turn rate (rad/s), so the bot can't snap round faster than a player could.
const TURN_RATE: f32 = 2.4;
const FIRE_EVERY: f32 = 0.32;
/// Each shot leaves up to this far off dead centre (rad), like a thumb on a stick.
const AIM_NOISE: f32 = 0.08;

#[derive(Default, Clone, Copy)]
struct Round {
    won: bool,
    lost: bool,
    secs: f32,
    his_hp_left: f32,
    your_hp_left: f32,
    hits: u32,
    his_hits: u32,
    him_into_hazards: u32,
    you_into_hazards: u32,
}

#[derive(Resource, Default)]
struct Tally(Round);

#[derive(Resource, Default)]
struct Bot {
    fire_in: f32,
    replan_in: f32,
    path: Vec<Vec3>,
    aim_off: f32,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let rounds: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(40);
    let settings: Vec<f32> = args
        .next()
        .map(|a| a.split(',').filter_map(|v| v.parse().ok()).collect())
        .unwrap_or_else(|| vec![0.0, 4.0]);

    println!("{rounds} rounds per setting; the same arena seeds under each.\n");
    println!(
        "{:>9} | {:>5} {:>5} {:>5} | {:>6} {:>6} | {:>8} {:>8} | {:>9} {:>9} | {:>7} {:>7}",
        "knockback", "win%", "lose%", "draw%", "secs", "p90", "his hp", "your hp", "your hits", "his hits", "him→hz", "you→hz"
    );
    for &k in &settings {
        let results: Vec<Round> = (0..rounds).map(|seed| play(seed, k)).collect();
        let n = results.len() as f32;
        let pct = |f: fn(&Round) -> bool| 100.0 * results.iter().filter(|r| f(r)).count() as f32 / n;
        let mean = |f: fn(&Round) -> f32| results.iter().map(f).sum::<f32>() / n;
        let mut secs: Vec<f32> = results.iter().map(|r| r.secs).collect();
        secs.sort_by(f32::total_cmp);
        println!(
            "{:>7.1}/s | {:>5.0} {:>5.0} {:>5.0} | {:>6.1} {:>6.1} | {:>7.0}% {:>7.0}% | {:>9.1} {:>9.1} | {:>7.2} {:>7.2}",
            k,
            pct(|r| r.won),
            pct(|r| r.lost),
            pct(|r| !r.won && !r.lost),
            mean(|r| r.secs),
            secs[((n * 0.9) as usize).min(secs.len() - 1)],
            100.0 * mean(|r| r.his_hp_left),
            100.0 * mean(|r| r.your_hp_left),
            mean(|r| r.hits as f32),
            mean(|r| r.his_hits as f32),
            mean(|r| r.him_into_hazards as f32),
            mean(|r| r.you_into_hazards as f32),
        );
    }
}

fn play(seed: u64, knockback: f32) -> Round {
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
    .add_plugins(second_person::gameplay)
    .insert_resource(ArenaMode::Seed(seed))
    .insert_resource(TargetKnockback(knockback))
    .init_resource::<Tally>()
    .init_resource::<Bot>()
    .add_systems(Update, bot.run_if(in_state(GameState::Playing)))
    .add_systems(Last, count);
    app.finish();
    app.cleanup();
    app.update();
    app.update();

    let frames = (ROUND_LIMIT_SECS / DT) as usize;
    for i in 0..frames {
        app.update();
        let state = *app.world().resource::<State<GameState>>().get();
        if state != GameState::Playing {
            let tally = &mut app.world_mut().resource_mut::<Tally>().0;
            tally.won = state == GameState::Won;
            tally.lost = state == GameState::Lost;
            tally.secs = i as f32 * DT;
            break;
        }
        app.world_mut().resource_mut::<Tally>().0.secs = i as f32 * DT;
    }
    let world = app.world_mut();
    let target = world.query::<&Target>().single(world).map(|t| t.hp / t.max_hp.max(1.0)).unwrap_or(0.0);
    let shooter = world.query::<&Shooter>().single(world).map(|s| s.hp / SHOOTER_MAX_HP).unwrap_or(0.0);
    let mut round = world.resource::<Tally>().0;
    round.his_hp_left = target;
    round.your_hp_left = shooter;
    round
}

fn count(
    mut tally: ResMut<Tally>,
    mut hits: MessageReader<TargetHit>,
    mut his_hits: MessageReader<ShooterHit>,
    mut him_hz: MessageReader<HisHazardHit>,
    mut you_hz: MessageReader<HazardHit>,
) {
    tally.0.hits += hits.read().count() as u32;
    tally.0.his_hits += his_hits.read().count() as u32;
    tally.0.him_into_hazards += him_hz.read().count() as u32;
    tally.0.you_into_hazards += you_hz.read().count() as u32;
}

#[allow(clippy::too_many_arguments)]
fn bot(
    time: Res<Time>,
    nav: Nav,
    layout: Res<Layout>,
    mut bot: ResMut<Bot>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut shooter: Query<(&Transform, &mut Rotation), With<Shooter>>,
    target: Query<&Transform, With<Target>>,
) {
    let (Ok((me, mut rot)), Ok(him)) = (shooter.single_mut(), target.single()) else { return };
    let dt = time.delta_secs();
    let (here, there) = (me.translation, him.translation);
    let sees = !layout.los_blocked(here.xz(), there.xz());

    bot.replan_in -= dt;
    if bot.replan_in <= 0.0 {
        bot.replan_in = 0.5;
        bot.path = nav.path(here, there).unwrap_or_default();
    }
    while bot.path.first().is_some_and(|w| w.xz().distance(here.xz()) < 0.6) {
        bot.path.remove(0);
    }
    let goal = if sees { there } else { bot.path.first().copied().unwrap_or(there) };

    // Turn toward the goal at the tank's rate.
    let d = (goal - here).with_y(0.0);
    let want = f32::atan2(-d.x, -d.z) + if sees { bot.aim_off } else { 0.0 };
    let (yaw, _, _) = rot.0.to_euler(EulerRot::YXZ);
    let off = (want - yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    let step = off.clamp(-TURN_RATE * dt, TURN_RATE * dt);
    rot.0 = Quat::from_rotation_y(yaw + step);

    keys.release(KeyCode::Space);
    bot.fire_in -= dt;
    if sees {
        keys.release(KeyCode::ArrowUp);
        if off.abs() < 0.05 && bot.fire_in <= 0.0 {
            bot.fire_in = FIRE_EVERY;
            keys.press(KeyCode::Space);
            bot.aim_off = rand::random_range(-AIM_NOISE..AIM_NOISE);
        }
    } else if off.abs() < 0.5 {
        keys.press(KeyCode::ArrowUp);
    } else {
        keys.release(KeyCode::ArrowUp);
    }
}
