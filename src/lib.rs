//! Second-person shooter: you see the world through your target's eyes while
//! driving the shooter who's hunting him.
//!
//! Split into `gameplay` (headless-testable simulation) and `presentation`
//! (rendering-only: HUD, radar, FX, audio). Tests build an app from `gameplay` alone.

pub mod arena;
pub mod audio;
pub mod combat;
pub mod fx;
pub mod hud;
pub mod nav;
pub mod radar;
pub mod round;
pub mod shooter;
pub mod target;
pub mod touch;

use avian3d::prelude::*;
use bevy::prelude::*;

/// Physics layers. World geometry uses the default layer.
#[derive(PhysicsLayer, Clone, Copy, Debug, Default)]
pub enum Layer {
    #[default]
    World,
    Shooter,
    Target,
    Bullet,
}

/// The simulation: physics, AI, input, combat, round flow. Runs without a window.
pub fn gameplay(app: &mut App) {
    app.add_plugins((
        PhysicsPlugins::default(),
        bevior_tree::BehaviorTreePlugin::default(),
    ))
    .add_plugins((
        round::plugin,
        arena::plugin,
        nav::plugin,
        target::plugin,
        shooter::plugin,
        combat::plugin,
    ));
}

/// Everything that only matters when there's a screen and speakers.
pub fn presentation(app: &mut App) {
    app.add_plugins((fx::plugin, audio::plugin, radar::plugin, hud::plugin, touch::plugin));
}
