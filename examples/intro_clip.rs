//! Records the way into a round: the title over the demo round, a press, the countdown over the
//! arena, then the swoop into his eyes.
//!
//!     xvfb-run -a cargo run --example intro_clip -- <out dir> [seed]
//!     ffmpeg -framerate 30 -i <out dir>/frame-%04d.png -pix_fmt yuv420p intro.mp4
//!
//! Without a seed it uses the classic arena. Time advances a fixed 1/60s per frame and every
//! other frame is saved, so the clip plays at real speed however slowly the frames render.

use std::{path::PathBuf, time::Duration};

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    time::TimeUpdateStrategy,
};
use bevy_egui::{EguiGlobalSettings, EguiPlugin};
use leafwing_input_manager::prelude::*;
use second_person::{
    arena::ArenaMode,
    intro::Intro,
    touch::TouchControls,
};

#[derive(Resource)]
struct Out(PathBuf);

fn main() {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().unwrap_or_else(|| "intro_clip".into()));
    let arena = args.next().map_or(ArenaMode::Classic, |s| ArenaMode::Seed(s.parse().expect("seed")));
    std::fs::create_dir_all(&out).unwrap();
    App::new()
        .insert_resource(TouchControls(false))
        .insert_resource(arena)
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
        .add_systems(Last, record)
        .run();
}

/// Title for 4s, then a press; stop half a second after he's in his eyes.
fn record(world: &mut World, mut frame: Local<u32>, mut ended: Local<Option<u32>>) {
    *frame += 1;
    let f = *frame;
    match f {
        240 => KeyCode::Space.press(world),
        244 => KeyCode::Space.release(world),
        _ => {}
    }
    if f > 300 && ended.is_none() && world.resource::<Intro>().0.is_none() {
        *ended = Some(f);
    }
    if ended.is_some_and(|e| f >= e + 30) {
        world.write_message(AppExit::Success);
    } else if f >= 4 && f % 2 == 0 {
        let path = world.resource::<Out>().0.join(format!("frame-{:04}.png", (f - 4) / 2));
        world.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
}
