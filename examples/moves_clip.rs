//! Records the evasive moves from the target's eyes: his dodge roll when you fire straight at
//! him, then your sidestep hop (double-tap Right, then Left).
//!
//!     xvfb-run -a cargo run --example moves_clip -- <out dir>
//!     ffmpeg -framerate 30 -i <out dir>/frame-%04d.png -pix_fmt yuv420p moves.mp4
//!
//! Time advances a fixed 1/60s per frame and every other frame is saved, so the clip plays at
//! real speed at 30 fps however slowly the frames render.

use std::{path::PathBuf, time::Duration};

use avian3d::prelude::*;
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    time::TimeUpdateStrategy,
};
use bevy_egui::{EguiGlobalSettings, EguiPlugin};
use leafwing_input_manager::prelude::*;
use second_person::{
    arena::ArenaMode,
    round::GameState,
    shooter::Shooter,
    start::Started,
    target::{Suspicion, Target},
    touch::TouchControls,
};

const TARGET_AT: Vec3 = Vec3::new(0.0, 0.9, 0.0);
/// Far enough that he sees the shot coming and rolls.
const SHOOTER_FAR: Vec3 = Vec3::new(0.0, 0.9, -12.0);
/// Close enough that the hop reads clearly across his view.
const SHOOTER_NEAR: Vec3 = Vec3::new(0.0, 0.9, -8.0);
const LAST_FRAME: u32 = 420;

#[derive(Resource)]
struct Out(PathBuf);

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "moves_clip".into()));
    std::fs::create_dir_all(&out).unwrap();
    App::new()
        .insert_resource(Started(true))
        .insert_resource(TouchControls(false))
        .insert_resource(ArenaMode::Classic)
        .insert_resource(EguiGlobalSettings {
            auto_create_primary_context: false,
            ..default()
        })
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(1.0 / 60.0)))
        .insert_resource(Out(out))
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    resolution: (1280, 720).into(),
                    resizable: false,
                    ..default()
                }),
                ..default()
            }),
            EguiPlugin::default(),
            second_person::gameplay,
            second_person::presentation,
        ))
        .add_systems(Last, script)
        .run();
}

fn script(world: &mut World, mut frame: Local<u32>) {
    *frame += 1;
    let f = *frame;
    match f {
        // Scene 1: he has you in sight and is engaged; you fire straight at him and he dives.
        20..=79 => {
            stage(world, SHOOTER_FAR);
            if f >= 40 {
                let t = single::<Target>(world);
                world.get_mut::<Suspicion>(t).unwrap().bump(1.0);
            }
        }
        80 => {
            stage(world, SHOOTER_FAR);
            KeyCode::Space.press(world);
        }
        81 => KeyCode::Space.release(world),
        // Scene 2: a fresh round. He watches, unbothered, while you hop right, then left.
        200 => world.resource_mut::<NextState<GameState>>().set(GameState::Playing),
        201..=LAST_FRAME => {
            stage_target(world);
            if f < 230 {
                stage_shooter(world, SHOOTER_NEAR);
            }
            let tap = |start: u32, key: KeyCode, world: &mut World| match f.wrapping_sub(start) {
                0 | 8 => key.press(world),
                4 | 12 => key.release(world),
                _ => {}
            };
            tap(260, KeyCode::ArrowRight, world);
            tap(330, KeyCode::ArrowLeft, world);
            if f == LAST_FRAME {
                world.write_message(AppExit::Success);
            }
        }
        _ => {}
    }

    if (20..LAST_FRAME).contains(&f) && f % 2 == 0 && !(140..226).contains(&f) {
        let n = if f < 140 { (f - 20) / 2 } else { (f - 20 - 86) / 2 };
        let path = world.resource::<Out>().0.join(format!("frame-{n:04}.png"));
        world.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
}

/// Target at the origin facing the shooter, who faces him.
fn stage(world: &mut World, shooter_at: Vec3) {
    let target = single::<Target>(world);
    place(world, target, TARGET_AT, shooter_at);
    stage_shooter(world, shooter_at);
}

fn stage_shooter(world: &mut World, at: Vec3) {
    let shooter = single::<Shooter>(world);
    place(world, shooter, at, TARGET_AT);
}

/// Hold him still, looking down the line, and calm, so he just watches.
fn stage_target(world: &mut World) {
    let target = single::<Target>(world);
    place(world, target, TARGET_AT, SHOOTER_NEAR);
    *world.get_mut::<Suspicion>(target).unwrap() = Suspicion::default();
}

fn place(world: &mut World, e: Entity, pos: Vec3, facing: Vec3) {
    let d = facing - pos;
    let rot = Quat::from_rotation_y(f32::atan2(-d.x, -d.z));
    world.entity_mut(e).insert((
        Transform::from_translation(pos).with_rotation(rot),
        Position(pos),
        Rotation(rot),
        LinearVelocity::ZERO,
    ));
}

fn single<C: Component>(world: &mut World) -> Entity {
    world.query_filtered::<Entity, With<C>>().single(world).unwrap()
}
