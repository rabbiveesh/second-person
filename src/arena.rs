//! Static level geometry: ground, perimeter walls and cover.

use avian3d::prelude::*;
use bevy::{camera::visibility::RenderLayers, prelude::*};

use crate::radar::WORLD_AND_RADAR;

pub const ARENA_HALF: f32 = 30.0;

pub fn plugin(app: &mut App) {
    app.insert_resource(ClearColor(Color::srgb(0.55, 0.7, 0.85)))
        .add_systems(Startup, spawn_arena);
}

fn spawn_arena(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let size = ARENA_HALF * 2.0;
    commands.spawn((
        Name::new("Ground"),
        RigidBody::Static,
        Collider::cuboid(size, 0.2, size),
        Mesh3d(meshes.add(Cuboid::new(size, 0.2, size))),
        MeshMaterial3d(materials.add(Color::srgb(0.32, 0.45, 0.28))),
        Transform::from_xyz(0.0, -0.1, 0.0),
        RenderLayers::from_layers(WORLD_AND_RADAR),
    ));

    commands.spawn((
        Name::new("Sun"),
        DirectionalLight {
            illuminance: 12_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(20.0, 40.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        brightness: 400.0,
        ..default()
    });

    let wall_mat = materials.add(Color::srgb(0.5, 0.5, 0.55));
    let wall_h = 3.0;
    for (pos, half_extents) in [
        (Vec3::new(0.0, wall_h / 2.0, -ARENA_HALF), Vec3::new(ARENA_HALF, wall_h / 2.0, 0.5)),
        (Vec3::new(0.0, wall_h / 2.0, ARENA_HALF), Vec3::new(ARENA_HALF, wall_h / 2.0, 0.5)),
        (Vec3::new(-ARENA_HALF, wall_h / 2.0, 0.0), Vec3::new(0.5, wall_h / 2.0, ARENA_HALF)),
        (Vec3::new(ARENA_HALF, wall_h / 2.0, 0.0), Vec3::new(0.5, wall_h / 2.0, ARENA_HALF)),
    ] {
        spawn_block(&mut commands, &mut meshes, wall_mat.clone(), "Wall", pos, half_extents);
    }

    // Hand-placed cover so rounds are reproducible.
    let crate_mat = materials.add(Color::srgb(0.6, 0.42, 0.25));
    let pillar_mat = materials.add(Color::srgb(0.7, 0.7, 0.72));
    let crates = [
        (-6.0, -8.0, 1.0),
        (7.0, -10.0, 1.2),
        (12.0, 4.0, 1.0),
        (-14.0, 6.0, 1.5),
        (-3.0, 14.0, 1.0),
        (18.0, -18.0, 1.3),
        (-20.0, -16.0, 1.0),
        (4.0, 20.0, 1.4),
    ];
    for (x, z, h) in crates {
        spawn_block(
            &mut commands,
            &mut meshes,
            crate_mat.clone(),
            "Crate",
            Vec3::new(x, h, z),
            Vec3::new(1.0, h, 1.0),
        );
    }
    for (x, z) in [(-10.0, -2.0), (10.0, -2.0), (0.0, -18.0), (-18.0, 20.0), (22.0, 12.0)] {
        spawn_block(
            &mut commands,
            &mut meshes,
            pillar_mat.clone(),
            "Pillar",
            Vec3::new(x, 2.5, z),
            Vec3::new(0.6, 2.5, 0.6),
        );
    }
}

fn spawn_block(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    name: &'static str,
    pos: Vec3,
    half: Vec3,
) {
    let full = half * 2.0;
    commands.spawn((
        Name::new(name),
        RigidBody::Static,
        Collider::cuboid(full.x, full.y, full.z),
        Mesh3d(meshes.add(Cuboid::new(full.x, full.y, full.z))),
        MeshMaterial3d(material),
        Transform::from_translation(pos),
        RenderLayers::from_layers(WORLD_AND_RADAR),
    ));
}
