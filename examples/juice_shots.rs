//! Stages the juice moments and screenshots them: the target's hit flinch, his death fall,
//! and the shooter's death, as seen from the target's eyes.
//!
//!     xvfb-run -a -s "-screen 0 1280x720x24" cargo run --example juice_shots -- <out dir>
//!
//! Time advances a fixed 1/60s per frame, so the shots land on the same moments however
//! slowly the frames render (e.g. under software Vulkan).

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
    combat::{ShooterHit, TargetHit},
    round::GameState,
    shooter::Shooter,
    target::{MainCamera, Target},
};

const TARGET_AT: Vec3 = Vec3::new(0.0, 0.9, 0.0);
const SHOOTER_AT: Vec3 = Vec3::new(1.5, 0.9, -8.0);

#[derive(Resource)]
struct Out(PathBuf);

/// Frame of the latest `TargetHit`, so shots are timed from the impact.
#[derive(Resource, Default)]
struct LastHit(Option<u32>);

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "juice_shots".into()));
    std::fs::create_dir_all(&out).unwrap();
    App::new()
        .insert_resource(EguiGlobalSettings {
            auto_create_primary_context: false,
            ..default()
        })
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(1.0 / 60.0)))
        .insert_resource(Out(out))
        .init_resource::<LastHit>()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    resolution: (1280, 720).into(),
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
    if world.resource::<Messages<TargetHit>>().iter_current_update_messages().next().is_some() {
        world.resource_mut::<LastHit>().0 = Some(f);
    }
    let hit = world.resource::<LastHit>().0;

    match f {
        // Scene 1: a hit on a healthy target. Keep him facing the shooter until the shot.
        30..=89 => stage(world),
        90 => {
            stage(world);
            KeyCode::Space.press(world);
        }
        91 => KeyCode::Space.release(world),
        // Hold him still while the bullet is in flight.
        92..=120 | 212..=240 if hit.is_none() => stage(world),
        // Scene 2: the killing shot.
        150..=209 => {
            world.resource_mut::<LastHit>().0 = None;
            stage(world);
            let t = single::<Target>(world);
            world.get_mut::<Target>(t).unwrap().hp = 1;
        }
        210 => {
            stage(world);
            KeyCode::Space.press(world);
        }
        211 => {
            stage(world);
            KeyCode::Space.release(world);
        }
        // Scene 3: restart, then the target's killing shot on the shooter.
        400 => world.resource_mut::<NextState<GameState>>().set(GameState::Playing),
        430..=479 => stage(world),
        480 => {
            let shooter = single::<Shooter>(world);
            world.get_mut::<Shooter>(shooter).unwrap().hp = 0.0;
            let cam = single::<MainCamera>(world);
            let eyes = world.get::<GlobalTransform>(cam).unwrap();
            let from = eyes.translation() + eyes.down() * 0.3 + eyes.right() * 0.2;
            world.write_message(ShooterHit { from, to: SHOOTER_AT });
        }
        700 => {
            world.write_message(AppExit::Success);
        }
        _ => {}
    }

    let shot = |name: &str| Some(name.to_string());
    // With JUICE_VIDEO set, grab every other frame instead (30 fps of game time), e.g.
    //     ffmpeg -framerate 30 -i <out dir>/frame-%04d.png -pix_fmt yuv420p juice.mp4
    if std::env::var_os("JUICE_VIDEO").is_some() {
        if (60..640).contains(&f) && f % 2 == 0 {
            let path = world.resource::<Out>().0.join(format!("frame-{:04}.png", (f - 60) / 2));
            world.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
        }
        return;
    }
    let name = match (f, hit) {
        (89, _) => shot("1-before-hit"),
        (_, Some(h)) if f == h + 4 && f < 150 => shot("2-flinch"),
        (_, Some(h)) if f == h + 4 && f > 210 => shot("3-killing-hit"),
        (_, Some(h)) if f == h + 30 && f > 210 => shot("4-falling"),
        (_, Some(h)) if f == h + 120 && f > 210 => shot("5-down-banner"),
        (479, _) => shot("6-before-your-death"),
        (486, _) => shot("7-your-death"),
        (530, _) => shot("8-you-toppled"),
        (600, _) => shot("9-lost-banner"),
        _ => None,
    };
    if let Some(name) = name {
        let path = world.resource::<Out>().0.join(format!("{name}.png"));
        world.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
}

/// Target at the origin facing the shooter, who faces him from a few metres ahead.
fn stage(world: &mut World) {
    let target = single::<Target>(world);
    let shooter = single::<Shooter>(world);
    place(world, target, TARGET_AT, SHOOTER_AT);
    place(world, shooter, SHOOTER_AT, TARGET_AT);
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
