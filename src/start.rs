//! Start screen and input mode. The game waits, frozen, behind a "tap to play" overlay (drawn by
//! the HUD) until the first touch, key or click. That first press also lets the browser start
//! audio. From then on the controls follow the last input: a touch switches to touch controls,
//! and a key or click switches back to keyboard.
//!
//! Presentation only: the headless tests and scripts never see the overlay unless they add this.

use bevy::{
    input::{ButtonState, keyboard::KeyboardInput, mouse::MouseButtonInput, touch::TouchInput},
    prelude::*,
};
use leafwing_input_manager::{plugin::InputManagerSystem, prelude::*};

use crate::{
    shooter::{Shooter, ShooterAction},
    touch::TouchControls,
};

/// False while the start screen is up.
#[derive(Resource, Default)]
pub struct Started(pub bool);

pub fn plugin(app: &mut App) {
    app.init_resource::<Started>()
        .init_resource::<TouchControls>()
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

/// Before the start: time stands still and the shooter ignores input, so the press that
/// dismisses the overlay doesn't also fire. This runs before `follow_last_input`, so the actions
/// are enabled a frame after that press, when it's no longer "just pressed".
fn hold_until_started(
    started: Res<Started>,
    mut was_started: Local<bool>,
    mut time: ResMut<Time<Virtual>>,
    mut actions: Query<&mut ActionState<ShooterAction>, With<Shooter>>,
) {
    if *was_started {
        return;
    }
    if started.0 {
        *was_started = true;
        time.unpause();
        for mut a in &mut actions {
            a.enable();
        }
    } else {
        time.pause();
        for mut a in &mut actions {
            if !a.disabled() {
                a.disable();
            }
        }
    }
}
