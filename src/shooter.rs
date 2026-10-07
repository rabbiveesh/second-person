//! The shooter: the body you control, seen (mostly) through the target's eyes.

use std::f32::consts::TAU;

use avian3d::prelude::*;
use bevy::{camera::visibility::RenderLayers, prelude::*};
use leafwing_input_manager::prelude::*;
use rand::Rng;

use crate::{
    Layer,
    arena::{self, ARENA_HALF},
    radar::{LiveBlip, RADAR_LAYER, RadarContact},
    round::{GameState, RoundEntity, SpawnRound},
};

pub const SHOOTER_MAX_HP: f32 = 100.0;
/// Minimum start distance from the target (who starts at the origin).
pub const MIN_START_DISTANCE: f32 = 12.0;
const RADIUS: f32 = 0.35;
const MOVE_SPEED: f32 = 5.0;
const TURN_SPEED: f32 = 2.4;

#[derive(Actionlike, PartialEq, Eq, Clone, Copy, Hash, Debug, Reflect)]
pub enum ShooterAction {
    /// x: turn (right positive), y: throttle (forward positive). Analog, so a touch stick or
    /// gamepad gives partial speed; arrow keys drive it as a d-pad.
    #[actionlike(DualAxis)]
    Drive,
    Fire,
}

#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct Shooter {
    pub hp: f32,
}

/// The shooter took a step (heard by the target).
#[derive(Message, Clone, Copy)]
pub struct Footstep {
    pub at: Vec3,
}

const STEP_INTERVAL: f32 = 0.42;

pub fn plugin(app: &mut App) {
    app.add_plugins(InputManagerPlugin::<ShooterAction>::default())
        .add_message::<Footstep>()
        .add_systems(OnEnter(GameState::Playing), spawn_shooter.in_set(SpawnRound))
        .add_systems(Update, drive.run_if(in_state(GameState::Playing)));
}

fn spawn_shooter(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let blip = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 1.0, 0.3),
        unlit: true,
        ..default()
    });

    commands.spawn((
        Name::new("Shooter"),
        RoundEntity,
        Shooter { hp: SHOOTER_MAX_HP },
        InputMap::default()
            .with_dual_axis(ShooterAction::Drive, VirtualDPad::arrow_keys())
            .with(ShooterAction::Fire, KeyCode::Space),
        RigidBody::Dynamic,
        Collider::capsule(RADIUS, 1.0),
        LockedAxes::ROTATION_LOCKED,
        Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
        CollisionLayers::new(Layer::Shooter, [Layer::World, Layer::Target]),
        random_start(&mut rand::rng()),
        Mesh3d(meshes.add(Capsule3d::new(0.35, 1.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.95, 0.5, 0.1))),
        RadarContact(Color::srgb(0.2, 1.0, 0.3)),
        children![
            (
                Name::new("Visor"),
                Mesh3d(meshes.add(Cuboid::new(0.5, 0.15, 0.2))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(0.1, 0.9, 1.0),
                    emissive: LinearRgba::rgb(0.2, 1.5, 2.0),
                    ..default()
                })),
                Transform::from_xyz(0.0, 0.45, -0.28),
            ),
            (
                Name::new("Gun"),
                Mesh3d(meshes.add(Cuboid::new(0.12, 0.12, 0.8))),
                MeshMaterial3d(materials.add(Color::srgb(0.15, 0.15, 0.15))),
                Transform::from_xyz(0.3, 0.15, -0.45),
            ),
            (
                Name::new("Radar blip"),
                Mesh3d(meshes.add(Sphere::new(0.8))),
                MeshMaterial3d(blip.clone()),
                Transform::from_xyz(0.0, 4.0, 0.0),
                RenderLayers::layer(RADAR_LAYER),
                LiveBlip,
            ),
            (
                Name::new("Radar heading"),
                Mesh3d(meshes.add(Cuboid::new(0.4, 0.4, 2.2))),
                MeshMaterial3d(blip),
                Transform::from_xyz(0.0, 4.0, -1.3),
                RenderLayers::layer(RADAR_LAYER),
                LiveBlip,
            ),
        ],
    ));
}

/// A random start: clear of cover, away from the target, facing anywhere.
pub fn random_start(rng: &mut impl Rng) -> Transform {
    let r = ARENA_HALF - 2.0;
    loop {
        let p = Vec2::new(rng.random_range(-r..r), rng.random_range(-r..r));
        if p.length() >= MIN_START_DISTANCE && arena::is_clear(p, RADIUS + 0.3) {
            return Transform::from_xyz(p.x, 0.9, p.y)
                .with_rotation(Quat::from_rotation_y(rng.random_range(0.0..TAU)));
        }
    }
}

/// Tank controls, relative to the shooter's own facing.
fn drive(
    time: Res<Time>,
    mut step: Local<f32>,
    mut steps: MessageWriter<Footstep>,
    mut q: Query<(&ActionState<ShooterAction>, &Transform, &mut Rotation, &mut LinearVelocity), With<Shooter>>,
) {
    for (actions, t, mut rot, mut vel) in &mut q {
        let input = actions.clamped_axis_pair(&ShooterAction::Drive);
        rot.0 = Quat::from_rotation_y(-input.x * TURN_SPEED * time.delta_secs()) * rot.0;

        let throttle = input.y;
        let forward = rot.0 * Vec3::NEG_Z;
        let planar = forward * throttle * MOVE_SPEED;
        vel.0 = Vec3::new(planar.x, vel.0.y, planar.z);

        if throttle.abs() > 0.1 {
            *step -= time.delta_secs();
            if *step <= 0.0 {
                *step = STEP_INTERVAL;
                steps.write(Footstep { at: t.translation });
            }
        } else {
            *step = 0.0;
        }
    }
}
