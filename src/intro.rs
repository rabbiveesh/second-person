//! Round intro: the view opens high over the whole arena, with you and him marked (and the way
//! he's facing), counts down while you read the layout, then swoops down into his eyes. The game
//! is frozen throughout (virtual time paused, shooter input off). A press means "ready": it cuts
//! the countdown short, and a second one skips the swoop.
//!
//! The countdown adapts: its base length is `Countdown` (the difficulty system's knob), plus a
//! little for a busier arena. Each round reports how much of it the player used
//! (`CountdownDone`), which is a cheap read on how much orienting they still need.
//!
//! The camera is still the `MainCamera` under his head: each frame we pick a world pose and write
//! it as the camera's local transform, after `juice` and before transform propagation. When the
//! intro ends the pose has arrived at his head, so juice takes over with no seam.

use bevy::{
    input::{ButtonState, keyboard::KeyboardInput, mouse::MouseButtonInput, touch::TouchInput},
    prelude::*,
    transform::TransformSystems,
};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use leafwing_input_manager::{plugin::InputManagerSystem, prelude::*};
use std::f32::consts::{FRAC_PI_2, PI};

use crate::{
    arena::Layout,
    layout::ScreenLayout,
    round::{GameState, RoundEntity, SpawnRound},
    shooter::{Shooter, ShooterAction},
    start::{self, Started},
    target::{MainCamera, Target, TargetHead},
};

/// The swoop down into his eyes after the countdown (real seconds).
pub const SWOOP_SECS: f32 = 1.6;
/// Extra countdown per piece of cover, up to `BUSY_MAX`: cluttered arenas take longer to read.
const BUSY_PER_BLOCK: f32 = 0.05;
const BUSY_MAX: f32 = 1.5;
/// A press this soon after the game starts is the one that dismissed the start screen, not a skip.
const SKIP_GRACE: f32 = 0.25;
const MARKER_RADIUS: f32 = 1.6;
const YOU: Color = Color::srgb(0.2, 1.0, 0.3);
const HIM: Color = Color::srgb(1.0, 0.25, 0.2);

/// Off for staged captures and anything else that wants to start in his eyes.
#[derive(Resource)]
pub struct IntroEnabled(pub bool);

impl Default for IntroEnabled {
    fn default() -> Self {
        Self(true)
    }
}

/// Base countdown before the swoop (real seconds), before the allowance for a busy arena.
#[derive(Resource)]
pub struct Countdown(pub f32);

impl Default for Countdown {
    fn default() -> Self {
        Self(3.0)
    }
}

/// This round's intro: real seconds in, and how long the countdown is.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub t: f32,
    pub hold: f32,
}

/// The intro playing now, or `None` once it's over.
#[derive(Resource, Default)]
pub struct Intro(pub Option<Clock>);

/// The countdown ended: how long it offered and how much of it the player waited out.
#[derive(Message, Clone, Copy, Debug)]
pub struct CountdownDone {
    pub offered: f32,
    pub waited: f32,
}

#[derive(Component)]
struct IntroMarker;

pub fn plugin(app: &mut App) {
    app.init_resource::<IntroEnabled>()
        .init_resource::<Countdown>()
        .init_resource::<Intro>()
        .add_message::<CountdownDone>()
        .add_systems(OnEnter(GameState::Playing), begin.after(SpawnRound))
        .add_systems(
            PreUpdate,
            (freeze, advance)
                .chain()
                .after(start::hold_until_started)
                .before(InputManagerSystem::Update),
        )
        .add_systems(PostUpdate, fly_camera.before(TransformSystems::Propagate))
        .add_systems(EguiPrimaryContextPass, (label_markers, count_down));
}

fn begin(
    mut commands: Commands,
    enabled: Res<IntroEnabled>,
    countdown: Res<Countdown>,
    layout: Res<Layout>,
    mut intro: ResMut<Intro>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    shooter: Option<Single<&Transform, With<Shooter>>>,
    target: Single<&Transform, With<Target>>,
) {
    intro.0 = None;
    // No shooter means the title screen's demo round: nobody to orient.
    let Some(shooter) = shooter.filter(|_| enabled.0) else { return };
    let busy = (layout.blocks.len() as f32 * BUSY_PER_BLOCK).min(BUSY_MAX);
    intro.0 = Some(Clock { t: 0.0, hold: countdown.0 + busy });
    let mut mark = |at: Vec3, color: Color, facing: Option<Quat>| {
        let material = materials.add(StandardMaterial { base_color: color, unlit: true, ..default() });
        let floor = at.with_y(0.03);
        let flat = Quat::from_rotation_x(-FRAC_PI_2);
        let mut e = commands.spawn((
            IntroMarker,
            RoundEntity,
            Mesh3d(meshes.add(Annulus::new(MARKER_RADIUS * 0.75, MARKER_RADIUS))),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(floor).with_rotation(flat),
        ));
        if let Some(facing) = facing {
            // A wedge out front for the way he's looking.
            let tip = Triangle2d::new(Vec2::new(0.0, 2.6), Vec2::new(-0.7, 1.2), Vec2::new(0.7, 1.2));
            e.with_child((Mesh3d(meshes.add(tip)), MeshMaterial3d(material), Transform::default()));
            e.insert(Transform::from_translation(floor).with_rotation(facing * flat));
        }
    };
    mark(shooter.translation, YOU, None);
    mark(target.translation, HIM, Some(target.rotation));
}

/// Count real time once the title screen is gone. A press during the countdown ends it; a press
/// during the swoop finishes it.
fn advance(
    time: Res<Time<Real>>,
    started: Res<Started>,
    mut intro: ResMut<Intro>,
    mut commands: Commands,
    mut done: MessageWriter<CountdownDone>,
    markers: Query<Entity, With<IntroMarker>>,
    mut keys: MessageReader<KeyboardInput>,
    mut clicks: MessageReader<MouseButtonInput>,
    mut touches: MessageReader<TouchInput>,
) {
    let pressed = keys.read().any(|k| k.state == ButtonState::Pressed)
        | clicks.read().any(|c| c.state == ButtonState::Pressed)
        | (touches.read().count() > 0);
    let Some(clock) = intro.0.as_mut() else { return };
    if !started.0 {
        return;
    }
    let pressed = pressed && clock.t > SKIP_GRACE;
    let counting = clock.t < clock.hold;
    clock.t += time.delta_secs();
    if counting && (pressed || clock.t >= clock.hold) {
        done.write(CountdownDone { offered: clock.hold, waited: clock.t.min(clock.hold) });
        clock.t = clock.t.max(clock.hold);
        return;
    }
    if clock.t >= clock.hold + SWOOP_SECS || (pressed && !counting) {
        intro.0 = None;
        for e in &markers {
            commands.entity(e).despawn();
        }
    }
}

/// Hold the game still while the intro plays, and let it go the frame after it ends, so a press
/// that skipped it isn't also "just pressed" for the shooter.
fn freeze(
    intro: Res<Intro>,
    started: Res<Started>,
    mut was_playing: Local<bool>,
    mut time: ResMut<Time<Virtual>>,
    mut actions: Query<&mut ActionState<ShooterAction>, With<Shooter>>,
) {
    let playing = intro.0.is_some();
    if playing {
        time.pause();
        for mut a in &mut actions {
            if !a.disabled() {
                a.disable();
            }
        }
    } else if *was_playing && started.0 {
        time.unpause();
        for mut a in &mut actions {
            a.enable();
        }
    }
    *was_playing = playing;
}

/// Where the camera is at `t` seconds in, given the overhead shot and his eyes (world poses).
pub fn pose(clock: Clock, overhead: Transform, eyes: Transform) -> Transform {
    let u = ((clock.t - clock.hold) / SWOOP_SECS).clamp(0.0, 1.0);
    let s = u * u * (3.0 - 2.0 * u);
    // Arc in from above and behind him rather than dropping straight down the lift shaft.
    let behind = eyes.rotation * Vec3::Z;
    let height = overhead.translation.y - eyes.translation.y;
    let via = eyes.translation + Vec3::Y * height * 0.3 + behind * height * 0.15;
    let (a, b) = (overhead.translation.lerp(via, s), via.lerp(eyes.translation, s));
    Transform::from_translation(a.lerp(b, s)).with_rotation(overhead.rotation.slerp(eyes.rotation, s))
}

/// Straight down over the arena, just high enough to fit all of it, turned so he faces up the
/// screen and the swoop only has to pitch, not spin.
pub fn overhead(layout: &Layout, facing: Quat, vfov: f32, aspect: f32) -> Transform {
    let c = layout.bounds().center();
    let (yaw, _, _) = facing.to_euler(EulerRot::YXZ);
    let rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-PI / 2.0);
    // Each wall corner in screen axes (x across, y up the screen), against the view's half-extents.
    let (right, up) = ((rotation * Vec3::X).xz(), (rotation * Vec3::Y).xz());
    let half_v = (vfov / 2.0).tan();
    let height = layout
        .outline
        .iter()
        .map(|&p| ((p - c).dot(right).abs() / (half_v * aspect)).max((p - c).dot(up).abs() / half_v))
        .fold(0.0, f32::max)
        * 1.12;
    Transform::from_xyz(c.x, height, c.y).with_rotation(rotation)
}

fn fly_camera(
    intro: Res<Intro>,
    layout: Res<Layout>,
    target: Single<(&Transform, &Children), (With<Target>, Without<MainCamera>)>,
    heads: Query<&Transform, (With<TargetHead>, Without<MainCamera>)>,
    mut cam: Single<(&mut Transform, &Projection), With<MainCamera>>,
) {
    let Some(t) = intro.0 else { return };
    let (body, children) = *target;
    let Some(head) = children.iter().find_map(|c| heads.get(c).ok()) else { return };
    // Computed from local transforms, so it's right on a round's first frame too.
    let eyes = body.mul_transform(*head);
    let Projection::Perspective(p) = cam.1 else { return };
    let from = overhead(&layout, body.rotation, p.fov, p.aspect_ratio);
    let world = pose(t, from, eyes);
    *cam.0 = Transform::from_matrix((eyes.compute_affine().inverse() * world.compute_affine()).into());
}

/// "YOU" and "HIM" over the markers while the overhead shot holds, each on its own dark pill so
/// it reads on any floor.
fn label_markers(
    mut contexts: EguiContexts,
    intro: Res<Intro>,
    screen: Res<ScreenLayout>,
    cam: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    shooter: Single<&Transform, With<Shooter>>,
    target: Single<&Transform, With<Target>>,
) -> Result {
    let Some(clock) = intro.0 else { return Ok(()) };
    let fade = 1.0 - ((clock.t - clock.hold) / (SWOOP_SECS * 0.3)).clamp(0.0, 1.0);
    if fade <= 0.0 {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, "intro labels".into()));
    let (camera, eyes) = *cam;
    for (at, text, color) in [(shooter.translation, "YOU", YOU), (target.translation, "HIM", HIM)] {
        let Ok(p) = camera.world_to_viewport(eyes, at) else { continue };
        let p = p + screen.view.min;
        let [r, g, b, _] = color.to_srgba().to_u8_array();
        let alpha = (255.0 * fade) as u8;
        let galley = painter.layout_no_wrap(
            text.into(),
            egui::FontId::proportional(20.0),
            egui::Color32::from_rgba_unmultiplied(r, g, b, alpha),
        );
        let pill = egui::Rect::from_center_size(egui::pos2(p.x, p.y - 40.0), galley.size() + egui::vec2(20.0, 8.0));
        painter.rect_filled(pill, pill.height() / 2.0, egui::Color32::from_black_alpha((205.0 * fade) as u8));
        painter.galley(pill.center() - galley.size() / 2.0, galley, egui::Color32::WHITE);
    }
    Ok(())
}

/// The countdown, big, on a card near the top, clear of him (he's always mid-arena).
fn count_down(mut contexts: EguiContexts, intro: Res<Intro>) -> Result {
    let Some(clock) = intro.0.filter(|c| c.t < c.hold) else { return Ok(()) };
    let ctx = contexts.ctx_mut()?;
    let left = (clock.hold - clock.t).ceil().max(1.0) as u32;
    egui::Area::new("countdown".into())
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_TOP, [0.0, 24.0])
        .interactable(false)
        .show(ctx, |ui| {
            crate::hud::card().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(left.to_string()).size(72.0).color(egui::Color32::WHITE).strong());
                    ui.label(egui::RichText::new("tap or press any key when ready").size(15.0).color(egui::Color32::LIGHT_GRAY));
                });
            });
        });
    Ok(())
}
