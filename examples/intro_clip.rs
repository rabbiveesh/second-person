//! Records a round's intro: the overhead shot of the arena, then the swoop into his eyes.
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
use second_person::{
    arena::ArenaMode,
    intro::{HOLD_SECS, SWOOP_SECS},
    start::Started,
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
        .insert_resource(Started(true))
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

fn record(world: &mut World, mut frame: Local<u32>) {
    *frame += 1;
    let f = *frame;
    let last = ((HOLD_SECS + SWOOP_SECS + 1.0) * 60.0) as u32;
    if f >= last {
        world.write_message(AppExit::Success);
    } else if f >= 4 && f % 2 == 0 {
        let path = world.resource::<Out>().0.join(format!("frame-{:04}.png", (f - 4) / 2));
        world.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
}
