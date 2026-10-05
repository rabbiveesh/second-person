//! The shooter: the body you control, seen (mostly) through the target's eyes.

use std::f32::consts::FRAC_PI_2;

use avian3d::prelude::*;
use bevy::{camera::visibility::RenderLayers, prelude::*};
use leafwing_input_manager::prelude::*;

use crate::{
    Layer,
    radar::{RADAR_LAYER, WORLD_AND_RADAR},
    round::{GameState, RoundEntity, SpawnRound},
};

pub const SHOOTER_MAX_HP: f32 = 100.0;
const MOVE_SPEED: f32 = 5.0;
const TURN_SPEED: f32 = 2.4;

#[derive(Actionlike, PartialEq, Eq, Clone, Copy, Hash, Debug, Reflect)]
pub enum ShooterAction {
    Forward,
    Back,
    TurnLeft,
    TurnRight,
    Fire,
}

#[derive(Component)]
pub struct Shooter {
    pub hp: f32,
}

pub fn plugin(app: &mut App) {
    app.add_plugins(InputManagerPlugin::<ShooterAction>::default())
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
    let layers = RenderLayers::from_layers(WORLD_AND_RADAR);

    commands.spawn((
        Name::new("Shooter"),
        RoundEntity,
        Shooter { hp: SHOOTER_MAX_HP },
        InputMap::new([
            (ShooterAction::Forward, KeyCode::ArrowUp),
            (ShooterAction::Back, KeyCode::ArrowDown),
            (ShooterAction::TurnLeft, KeyCode::ArrowLeft),
            (ShooterAction::TurnRight, KeyCode::ArrowRight),
            (ShooterAction::Fire, KeyCode::Space),
        ]),
        RigidBody::Dynamic,
        Collider::capsule(0.35, 1.0),
        LockedAxes::ROTATION_LOCKED,
        Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
        CollisionLayers::new(Layer::Shooter, [Layer::World, Layer::Target]),
        // Start on screen but just outside his attention cone, facing across his view.
        Transform::from_xyz(-12.5, 0.9, -13.5).with_rotation(Quat::from_rotation_y(-FRAC_PI_2)),
        Mesh3d(meshes.add(Capsule3d::new(0.35, 1.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.95, 0.5, 0.1))),
        layers.clone(),
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
                layers.clone(),
            ),
            (
                Name::new("Gun"),
                Mesh3d(meshes.add(Cuboid::new(0.12, 0.12, 0.8))),
                MeshMaterial3d(materials.add(Color::srgb(0.15, 0.15, 0.15))),
                Transform::from_xyz(0.3, 0.15, -0.45),
                layers.clone(),
            ),
            (
                Name::new("Radar blip"),
                Mesh3d(meshes.add(Sphere::new(0.8))),
                MeshMaterial3d(blip.clone()),
                Transform::from_xyz(0.0, 4.0, 0.0),
                RenderLayers::layer(RADAR_LAYER),
            ),
            (
                Name::new("Radar heading"),
                Mesh3d(meshes.add(Cuboid::new(0.4, 0.4, 2.2))),
                MeshMaterial3d(blip),
                Transform::from_xyz(0.0, 4.0, -1.3),
                RenderLayers::layer(RADAR_LAYER),
            ),
        ],
    ));
}

/// Tank controls, relative to the shooter's own facing.
fn drive(
    time: Res<Time>,
    mut q: Query<(&ActionState<ShooterAction>, &mut Rotation, &mut LinearVelocity), With<Shooter>>,
) {
    for (actions, mut rot, mut vel) in &mut q {
        let turn = actions.pressed(&ShooterAction::TurnLeft) as i8 as f32
            - actions.pressed(&ShooterAction::TurnRight) as i8 as f32;
        rot.0 = Quat::from_rotation_y(turn * TURN_SPEED * time.delta_secs()) * rot.0;

        let throttle = actions.pressed(&ShooterAction::Forward) as i8 as f32
            - actions.pressed(&ShooterAction::Back) as i8 as f32;
        let forward = rot.0 * Vec3::NEG_Z;
        let planar = forward * throttle * MOVE_SPEED;
        vel.0 = Vec3::new(planar.x, vel.0.y, planar.z);
    }
}
