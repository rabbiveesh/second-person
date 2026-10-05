//! Second-person shooter: you see the world through your target's eyes while
//! driving the shooter who's hunting him.

mod arena;
mod audio;
mod combat;
mod fx;
mod hud;
mod radar;
mod round;
mod shooter;
mod target;

use avian3d::prelude::*;
use bevy::{input::common_conditions::input_toggle_active, prelude::*};
use bevy_egui::{EguiGlobalSettings, EguiPlugin};
use bevy_inspector_egui::quick::WorldInspectorPlugin;

fn main() {
    App::new()
        // The HUD camera owns the egui context, since game cameras get respawned per round.
        // Insert before EguiPlugin (it only init_resource's) so no camera ever races auto-creation.
        .insert_resource(EguiGlobalSettings {
            auto_create_primary_context: false,
            ..default()
        })
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Second Person Shooter".into(),
                    // Web: render into the page's canvas and keep arrows/space from scrolling it.
                    canvas: Some("#game".into()),
                    fit_canvas_to_parent: true,
                    prevent_default_event_handling: true,
                    ..default()
                }),
                ..default()
            }),
            PhysicsPlugins::default(),
            EguiPlugin::default(),
            WorldInspectorPlugin::default().run_if(input_toggle_active(false, KeyCode::F1)),
            bevior_tree::BehaviorTreePlugin::default(),
        ))
        .add_plugins((
            round::plugin,
            arena::plugin,
            target::plugin,
            shooter::plugin,
            combat::plugin,
            fx::plugin,
            audio::plugin,
            radar::plugin,
            hud::plugin,
        ))
        .run();
}

/// Physics layers. World geometry uses the default layer.
#[derive(PhysicsLayer, Clone, Copy, Debug, Default)]
pub enum Layer {
    #[default]
    World,
    Shooter,
    Target,
    Bullet,
}
