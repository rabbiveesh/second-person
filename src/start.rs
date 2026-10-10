//! Title screen and input mode. Until the first touch, key or click, the title card sits over a
//! live demo round (`Attract`): no shooter, just him wandering an arena, seen through his eyes,
//! which says what the game is before anyone reads a word. That first press also lets the browser
//! start audio, and starts a fresh round for real. From then on the controls follow the last
//! input: a touch switches to touch controls, and a key or click switches back to keyboard.
//!
//! Presentation only: the headless tests and scripts never see the title unless they add this.

use bevy::{
    input::{ButtonState, keyboard::KeyboardInput, mouse::MouseButtonInput, touch::TouchInput},
    prelude::*,
};
use leafwing_input_manager::{plugin::InputManagerSystem, prelude::*};

use crate::{
    round::{Attract, GameState},
    shooter::{Shooter, ShooterAction},
    touch::TouchControls,
};

/// False while the title screen is up.
#[derive(Resource, Default)]
pub struct Started(pub bool);

pub fn plugin(app: &mut App) {
    // Something that starts already started (a staged capture) skips the demo round too.
    let started = app.world().get_resource::<Started>().is_some_and(|s| s.0);
    app.init_resource::<Started>()
        .init_resource::<TouchControls>()
        .insert_resource(Attract(!started))
        .add_systems(PreUpdate, (hold_until_started, follow_last_input).chain().before(InputManagerSystem::Update));
}

/// A touch means touch controls; a key or mouse press means keyboard.
fn follow_last_input(
    mut touches: MessageReader<TouchInput>,
    mut keys: MessageReader<KeyboardInput>,
    mut clicks: MessageReader<MouseButtonInput>,
    mut started: ResMut<Started>,
    mut touch: ResMut<TouchControls>,
) {
    let touched = touches.read().count() > 0;
    let keyed = keys.read().any(|k| k.state == ButtonState::Pressed)
        || clicks.read().any(|c| c.state == ButtonState::Pressed);
    if touched || keyed {
        // Both in one frame shouldn't happen; if it does, keyboard wins.
        touch.set_if_neq(TouchControls(!keyed));
        if !started.0 {
            started.0 = true;
        }
    }
}

/// On the first press, end the demo and start a real round. The shooter ignores input until
/// everything is let go, so the press that dismissed the title doesn't also fire or drive.
pub fn hold_until_started(
    started: Res<Started>,
    mut was_started: Local<bool>,
    mut released: Local<bool>,
    mut attract: ResMut<Attract>,
    mut next: ResMut<NextState<GameState>>,
    mut time: ResMut<Time<Virtual>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    mut actions: Query<&mut ActionState<ShooterAction>, With<Shooter>>,
) {
    if started.0 && !*was_started {
        *was_started = true;
        if attract.0 {
            attract.0 = false;
            next.set(GameState::Playing);
        }
        time.unpause();
    }
    if *released {
        return;
    }
    let held = keys.get_pressed().next().is_some()
        || mouse.get_pressed().next().is_some()
        || touches.iter().next().is_some();
    if *was_started && !held {
        *released = true;
        for mut a in &mut actions {
            a.enable();
        }
    } else {
        for mut a in &mut actions {
            if !a.disabled() {
                a.disable();
            }
        }
    }
}
