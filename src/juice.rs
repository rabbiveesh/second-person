//! Camera and body "juice" that only moves transforms, so it runs headless and is testable:
//! the target's view flinches when he's hit and drops to the floor when he dies, and the
//! shooter topples when he's killed. Purely cosmetic: gameplay never reads these offsets.

use bevy::prelude::*;

use crate::{
    combat::TargetHit,
    round::GameState,
    shooter::Shooter,
    target::{MainCamera, ROLL_SECS, Roll, Target, TargetHead},
};

/// Eye height above the floor, and how far above it his view ends up when he's down.
const EYE_HEIGHT: f32 = 1.6;
const FLOOR_EYE_HEIGHT: f32 = 0.25;
const FALL_TIME: f32 = 0.7;
/// Head-snap kick (rad/s) per hit, and the spring that pulls the view back.
const KICK_YAW: f32 = 5.0;
const KICK_PITCH: f32 = 4.0;
const KICK_ROLL: f32 = 4.0;
const SPRING_STIFFNESS: f32 = 180.0;
const SPRING_DAMPING: f32 = 14.0;
/// Camera shake: trauma is set to 1 on a hit and decays; shake scales with trauma².
const SHAKE_ANGLE: f32 = 0.04;
const TRAUMA_DECAY: f32 = 2.5;
/// His dodge roll tilts the view this far (rad) and dips it this much (m) at its peak. Kept small:
/// his eyes are the player's screen.
const ROLL_TILT: f32 = 0.45;
const ROLL_DIP: f32 = 0.5;
/// Shooter capsule: half its height (centre to feet) and its radius.
const SHOOTER_HALF_HEIGHT: f32 = 0.85;
const SHOOTER_RADIUS: f32 = 0.35;
const TOPPLE_TIME: f32 = 0.8;

/// Offsets layered on the target's eyes. Lives on the `MainCamera`, whose own transform is
/// otherwise identity (the AI aims the body and head, never the camera).
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct CameraJuice {
    /// Flinch spring state, as (pitch, yaw, roll) angles and their velocities.
    angle: Vec3,
    velocity: Vec3,
    trauma: f32,
    /// Which way he falls: +1 tips the view left, -1 right. Set by the latest hit.
    side: f32,
    /// Seconds since he died, once he has.
    pub fall: Option<f32>,
    clock: f32,
}

/// The shooter falling over after being shot dead, pivoting on the edge of his feet.
#[derive(Component)]
struct Topple {
    t: f32,
    start: Transform,
    pivot: Vec3,
    axis: Vec3,
}

pub fn plugin(app: &mut App) {
    app.register_required_components::<MainCamera, CameraJuice>()
        .add_systems(OnEnter(GameState::Won), start_fall)
        .add_systems(OnEnter(GameState::Lost), start_topple)
        .add_systems(Update, (camera_juice, topple));
}

fn start_fall(mut cam: Single<&mut CameraJuice>) {
    cam.fall = Some(0.0);
}

fn start_topple(
    mut commands: Commands,
    shooter: Single<(Entity, &Transform), With<Shooter>>,
    target: Single<&Transform, With<Target>>,
) {
    let (e, t) = *shooter;
    // He's knocked away from the target, and partly sideways so the fall reads from his view
    // (falling straight along the line of sight just looks like he shrank).
    let back = (t.translation - target.translation).with_y(0.0).normalize_or(Vec3::X);
    let away = (back + Vec3::Y.cross(back) * 1.2).normalize();
    commands.entity(e).insert(Topple {
        t: 0.0,
        start: *t,
        pivot: t.translation - Vec3::Y * SHOOTER_HALF_HEIGHT + away * SHOOTER_RADIUS,
        axis: Vec3::Y.cross(away),
    });
}

fn camera_juice(
    time: Res<Time>,
    mut hits: MessageReader<TargetHit>,
    head: Single<&GlobalTransform, With<TargetHead>>,
    cam: Single<(&mut Transform, &mut CameraJuice), With<MainCamera>>,
    roll: Option<Single<&Roll>>,
) {
    let (mut transform, mut j) = cam.into_inner();
    let dt = time.delta_secs();
    let head_rot = head.rotation();

    for hit in hits.read() {
        // The bullet's direction in view space: his head snaps along it.
        let d = (head_rot * transform.rotation).inverse() * hit.dir;
        j.velocity += Vec3::new(d.z * KICK_PITCH, -d.x * KICK_YAW, -d.x * KICK_ROLL);
        j.trauma = 1.0;
        if d.x.abs() > 0.1 {
            j.side = -d.x.signum();
        }
    }

    // Damped spring back to rest (semi-implicit Euler is stable at these rates).
    let accel = -SPRING_STIFFNESS * j.angle - SPRING_DAMPING * j.velocity;
    j.velocity += accel * dt;
    let v = j.velocity;
    j.angle += v * dt;
    j.trauma = (j.trauma - TRAUMA_DECAY * dt).max(0.0);
    j.clock += dt;

    let c = j.clock;
    let shake = SHAKE_ANGLE
        * j.trauma
        * j.trauma
        * Vec3::new((c * 37.0).sin(), (c * 29.0 + 1.3).sin(), (c * 23.0 + 2.1).sin());
    let mut a = j.angle + shake;
    // Dodge roll: lean into the dive and duck, out and back over the roll.
    let mut dip = 0.0;
    if let Some(roll) = roll {
        let side = roll.velocity.dot(*head.right()).signum();
        let arc = (std::f32::consts::PI * (roll.t / ROLL_SECS).clamp(0.0, 1.0)).sin();
        a.z -= side * ROLL_TILT * arc;
        dip = ROLL_DIP * arc;
    }
    let flinch = Quat::from_euler(EulerRot::YXZ, a.y, a.x, a.z);

    let Some(t) = j.fall.as_mut() else {
        *transform = Transform::from_rotation(flinch).with_translation(head_rot.inverse() * Vec3::NEG_Y * dip);
        return;
    };
    *t += dt;
    let t = *t;
    let side = if j.side == 0.0 { 1.0 } else { j.side };

    // Gravity-ish drop, a little bounce off the floor, and a roll onto his side.
    let drop = (t / FALL_TIME).min(1.0).powi(2);
    let bounce = if t > FALL_TIME {
        let s = t - FALL_TIME;
        0.08 * (-8.0 * s).exp() * (14.0 * s).sin().abs()
    } else {
        0.0
    };
    let roll = smoothstep((t / (FALL_TIME + 0.1)).min(1.0));

    // Work in world terms (level, facing his way), then express it in the head's frame.
    let (yaw, _, _) = head_rot.to_euler(EulerRot::YXZ);
    let facing = Quat::from_rotation_y(yaw);
    let lying = facing * Quat::from_rotation_z(side * 1.35) * Quat::from_rotation_x(0.1);
    let offset = facing * Vec3::new(-side * 0.4 * drop, -(EYE_HEIGHT - FLOOR_EYE_HEIGHT) * drop + bounce, 0.0);
    *transform = Transform {
        translation: head_rot.inverse() * offset,
        rotation: flinch.slerp(head_rot.inverse() * lying, roll),
        ..default()
    };
}

fn topple(time: Res<Time>, mut q: Query<(&mut Transform, &mut Topple)>) {
    for (mut transform, mut tp) in &mut q {
        tp.t += time.delta_secs();
        let p = (tp.t / TOPPLE_TIME).min(1.0).powi(2);
        let settle = if tp.t > TOPPLE_TIME {
            let s = tp.t - TOPPLE_TIME;
            0.12 * (-9.0 * s).exp() * (16.0 * s).sin().abs()
        } else {
            0.0
        };
        let r = Quat::from_axis_angle(tp.axis, std::f32::consts::FRAC_PI_2 * p - settle);
        let from_pivot = tp.start.translation - tp.pivot;
        transform.translation = tp.pivot + r * from_pivot;
        transform.rotation = r * tp.start.rotation;
    }
}

fn smoothstep(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}
