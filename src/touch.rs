//! Touch controls for phones and tablets: a floating stick on the left half drives the
//! shooter (tank controls, snapped to the 8 arrow-key directions), and a tap anywhere on the
//! right half fires.
//!
//! Nothing spawns until the first touch, so mouse-and-keyboard players never see it. Touch
//! writes into the same leafwing `ActionState` the keyboard does, so gameplay can't tell the
//! two apart.

use bevy::{
    asset::RenderAssetUsages,
    input::touch::{Touch, TouchInput},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    window::PrimaryWindow,
};
use bevy_egui::input::EguiWantsInput;
use leafwing_input_manager::{plugin::InputManagerSystem, prelude::*};
use virtual_joystick::{
    JoystickFloating, JoystickInvisible, NoAction, VirtualJoystickPlugin, VirtualJoystickState, create_joystick,
};

use crate::shooter::{Shooter, ShooterAction};

/// Set once the player first touches the screen; the HUD switches to touch buttons.
#[derive(Resource, Default)]
pub struct TouchControls(pub bool);

/// The HUD's whistle button: `pressed` is set when it's clicked and pressed into the shooter's
/// actions on the next frame. `button` is where it was last drawn (logical px), so a tap on it
/// doesn't also fire. (`EguiWantsInput` lags a frame, too late for a touch's first frame.)
#[derive(Resource, Default)]
pub struct TouchWhistle {
    pub pressed: bool,
    pub button: Rect,
}

const STICK_SIZE: f32 = 150.0;
const KNOB_SIZE: f32 = 70.0;
/// Stick travel (0..1) below which it does nothing.
const DEAD_ZONE: f32 = 0.3;

pub fn plugin(app: &mut App) {
    app.add_plugins(VirtualJoystickPlugin::<String>::default())
        .init_resource::<TouchControls>()
        .init_resource::<TouchWhistle>()
        .add_systems(Update, enable_on_first_touch)
        .add_systems(PreUpdate, touch_to_actions.in_set(InputManagerSystem::ManualControl));
}

fn enable_on_first_touch(
    mut touches: MessageReader<TouchInput>,
    mut enabled: ResMut<TouchControls>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
) {
    if enabled.0 || touches.read().next().is_none() {
        return;
    }
    enabled.0 = true;
    create_joystick(
        &mut commands,
        "drive".to_string(),
        images.add(disc(KNOB_SIZE as u32, 0.0)),
        images.add(disc(STICK_SIZE as u32, 0.88)),
        Some(Color::srgba(0.55, 1.0, 0.7, 0.6)),
        Some(Color::srgba(0.55, 1.0, 0.7, 0.35)),
        None,
        Vec2::splat(KNOB_SIZE),
        Vec2::splat(STICK_SIZE),
        Node {
            width: Val::Percent(50.0),
            height: Val::Percent(100.0),
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            bottom: Val::Px(0.0),
            ..default()
        },
        // Floats to wherever the thumb lands, and stays hidden until then.
        (JoystickFloating, JoystickInvisible),
        NoAction,
    );
}

/// Runs after leafwing has applied the keyboard, so an active touch overrides it.
fn touch_to_actions(
    enabled: Res<TouchControls>,
    touches: Res<Touches>,
    window: Single<&Window, With<PrimaryWindow>>,
    egui: Res<EguiWantsInput>,
    mut whistle: ResMut<TouchWhistle>,
    sticks: Query<&VirtualJoystickState>,
    mut actions: Query<&mut ActionState<ShooterAction>, With<Shooter>>,
) {
    if !enabled.0 {
        return;
    }
    let Ok(mut actions) = actions.single_mut() else {
        return;
    };
    if let Some(stick) = sticks.iter().find(|s| s.pointer_state.is_some()) {
        actions.set_axis_pair(&ShooterAction::Drive, snap_8way(stick.delta));
    }
    // One shot per tap, like Space: press only on the touch's first frame and let the
    // keyboard pass release it on the next.
    let fires = |t: &Touch| tap_fires(t.position(), window.width(), whistle.button);
    if !egui.wants_pointer_input() && touches.iter_just_pressed().any(fires) {
        actions.press(&ShooterAction::Fire);
    }
    if std::mem::take(&mut whistle.pressed) {
        actions.press(&ShooterAction::Whistle);
    }
}

/// A new touch fires if it lands on the right half of the screen, but not on the whistle button.
pub fn tap_fires(at: Vec2, window_width: f32, whistle_button: Rect) -> bool {
    at.x > window_width / 2.0 && !whistle_button.contains(at)
}

/// Tank controls are hard enough already, so the stick acts like the arrow keys: 8 directions at
/// full strength, so a diagonal is exactly two arrows held (forward while turning). The straight
/// directions get 60° each and the diagonals 30°, so a sloppy "up" still drives dead straight.
pub fn snap_8way(delta: Vec2) -> Vec2 {
    if delta.length() < DEAD_ZONE {
        return Vec2::ZERO;
    }
    // An axis counts once the stick is within 60° of it (cos 60° = 0.5).
    let held = |v: f32| if v.abs() > 0.5 { v.signum() } else { 0.0 };
    let n = delta.normalize();
    Vec2::new(held(n.x), held(n.y))
}

/// An anti-aliased white disc (or ring, if `hole` > 0) tinted per use by `ImageNode::color`.
fn disc(size: u32, hole: f32) -> Image {
    let r = size as f32 / 2.0;
    let data = (0..size * size)
        .flat_map(|i| {
            let p = Vec2::new((i % size) as f32 + 0.5, (i / size) as f32 + 0.5) - Vec2::splat(r);
            let d = p.length() / r;
            let outer = ((1.0 - d) * r).clamp(0.0, 1.0);
            let inner = if hole > 0.0 { ((d - hole) * r).clamp(0.0, 1.0) } else { 1.0 };
            [255, 255, 255, (outer * inner * 255.0) as u8]
        })
        .collect();
    Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}
