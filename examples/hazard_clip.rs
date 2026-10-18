//! Records a hazard hit from the target's eyes: you walk into a striped hazard in front of him,
//! yell, get thrown back, and he turns his attention on you.
//!
//!     xvfb-run -a cargo run --example hazard_clip -- <out dir>
//!     ffmpeg -framerate 30 -i <out dir>/frame-%04d.png -itsoffset $(cat <out dir>/yell_at) \
//!         -i assets/sfx/my_leg.wav -map 0:v -map 1:a -pix_fmt yuv420p -shortest hazard.mp4
//!
//! Frames render offscreen (no audio device), so `yell_at` records when the yell starts, in
//! seconds of video, for muxing the sound back in. Time advances a fixed 1/60s per frame and
//! every other frame is saved, so the clip plays at real speed at 30 fps.

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
    arena::{ArenaMode, Layout},
    intro::IntroEnabled,
    shooter::{SHOOTER_MAX_HP, Shooter},
    start::Started,
    target::{Suspicion, Target},
    touch::TouchControls,
};

const TARGET_AT: Vec3 = Vec3::new(0.0, 0.9, 0.0);
/// Frames of run-up before you start walking.
const WALK_FROM: u32 = 40;
/// Frames kept after the hit: the yell, the knockback, and his reaction.
const AFTER_HIT: u32 = 200;

/// Where the scene plays: an arena seed with a hazard in plain view of his spawn.
#[derive(Resource, Clone, Copy)]
struct Scene {
    seed: u64,
    hazard: Vec2,
    start: Vec2,
}

#[derive(Resource)]
struct Out(PathBuf);

#[derive(Resource, Default)]
struct HitAt(Option<u32>);

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "hazard_clip".into()));
    std::fs::create_dir_all(&out).unwrap();
    let scene = find_scene();
    App::new()
        .insert_resource(Started(true))
        .insert_resource(IntroEnabled(false))
        .insert_resource(TouchControls(false))
        .insert_resource(ArenaMode::Seed(scene.seed))
        .insert_resource(scene)
        .init_resource::<HitAt>()
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

/// The first generated arena with a hazard 6.5-10m from his spawn, a clear view of it, and room
/// in front of it for a run-up.
fn find_scene() -> Scene {
    (0..5000)
        .find_map(|seed| {
            let layout = Layout::random(seed);
            layout.blocks.iter().filter(|b| b.hazard).find_map(|b| {
                let dist = b.center.length();
                let dir = b.center.normalize_or_zero();
                let reach = b.half.length();
                let face = b.center - dir * (reach + 0.3);
                let start = b.center - dir * (reach + 4.0);
                let ok = (6.5..10.0).contains(&dist)
                    && start.length() > 3.0
                    && layout.is_clear(start, 1.0)
                    && !layout.los_blocked(Vec2::ZERO, face);
                ok.then_some(Scene { seed, hazard: b.center, start })
            })
        })
        .expect("no arena with a hazard in view")
}

fn script(world: &mut World, mut frame: Local<u32>) {
    *frame += 1;
    let f = *frame;
    let scene = *world.resource::<Scene>();
    let hazard = Vec3::new(scene.hazard.x, 0.9, scene.hazard.y);
    let hit = world.resource::<HitAt>().0;

    if hit.is_none() {
        // He watches the hazard, calm, while you line up and walk into it.
        let target = single::<Target>(world);
        place(world, target, TARGET_AT, hazard);
        *world.get_mut::<Suspicion>(target).unwrap() = Suspicion::default();
        if f < WALK_FROM {
            let shooter = single::<Shooter>(world);
            place(world, shooter, Vec3::new(scene.start.x, 0.9, scene.start.y), hazard);
        }
    }
    if f == WALK_FROM {
        KeyCode::ArrowUp.press(world);
    }
    let shooter = single::<Shooter>(world);
    if hit.is_none() && world.get::<Shooter>(shooter).unwrap().hp < SHOOTER_MAX_HP {
        KeyCode::ArrowUp.release(world);
        world.resource_mut::<HitAt>().0 = Some(f);
        let yell_at = (f - 20) as f32 / 60.0;
        std::fs::write(world.resource::<Out>().0.join("yell_at"), format!("{yell_at:.3}")).unwrap();
    }
    if hit.is_some_and(|h| f >= h + AFTER_HIT) || f > 600 {
        world.write_message(AppExit::Success);
        return;
    }

    if f >= 20 && f % 2 == 0 {
        let path = world.resource::<Out>().0.join(format!("frame-{:04}.png", (f - 20) / 2));
        world.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
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
