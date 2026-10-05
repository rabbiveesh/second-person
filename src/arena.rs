//! Static level geometry: ground, perimeter walls and cover.

use avian3d::prelude::*;
use bevy::{camera::visibility::RenderLayers, prelude::*};

use crate::{nav::NavObstacle, radar::WORLD_AND_RADAR};

pub const ARENA_HALF: f32 = 30.0;

/// Hand-placed cover so rounds are reproducible: crates are (x, z, half-height).
const CRATES: &[(f32, f32, f32)] = &[
    (-6.0, -8.0, 1.0),
    (7.0, -10.0, 1.2),
    (12.0, 4.0, 1.0),
    (-14.0, 6.0, 1.5),
    (-3.0, 14.0, 1.0),
    (18.0, -18.0, 1.3),
    (-20.0, -16.0, 1.0),
    (4.0, 20.0, 1.4),
];
const CRATE_HALF: f32 = 1.0;
const PILLARS: &[(f32, f32)] = &[(-10.0, -2.0), (10.0, -2.0), (0.0, -18.0), (-18.0, 20.0), (22.0, 12.0)];
const PILLAR_HALF: f32 = 0.6;

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
        // Main view only: on the radar its shadow map would reveal the actors' shadows.
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

    let crate_mat = materials.add(Color::srgb(0.6, 0.42, 0.25));
    let pillar_mat = materials.add(Color::srgb(0.7, 0.7, 0.72));
    for &(x, z, h) in CRATES {
        spawn_block(
            &mut commands,
            &mut meshes,
            crate_mat.clone(),
            "Crate",
            Vec3::new(x, h, z),
            Vec3::new(CRATE_HALF, h, CRATE_HALF),
        );
    }
    for &(x, z) in PILLARS {
        spawn_block(
            &mut commands,
            &mut meshes,
            pillar_mat.clone(),
            "Pillar",
            Vec3::new(x, 2.5, z),
            Vec3::new(PILLAR_HALF, 2.5, PILLAR_HALF),
        );
    }
}

/// Is a circle of `radius` at `p` (XZ) inside the walls and clear of all cover?
pub fn is_clear(p: Vec2, radius: f32) -> bool {
    let inside = p.abs().max_element() < ARENA_HALF - 0.5 - radius;
    inside && cover_blocks().all(|(c, half)| ((p - c).abs() - Vec2::splat(half)).max_element() > radius)
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
        NavObstacle,
    ));
}

/// Cover blocks as (centre XZ, half extent XZ). All of them are taller than eye height.
pub fn cover_blocks() -> impl Iterator<Item = (Vec2, f32)> {
    CRATES
        .iter()
        .map(|&(x, z, _)| (Vec2::new(x, z), CRATE_HALF))
        .chain(PILLARS.iter().map(|&(x, z)| (Vec2::new(x, z), PILLAR_HALF)))
}

/// Does any cover block sit between `a` and `b` (top-down)? Slab test against each block's square.
pub fn los_blocked(a: Vec2, b: Vec2) -> bool {
    let d = b - a;
    cover_blocks().any(|(c, half)| {
        let (lo, hi) = (c - half, c + half);
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for i in 0..2 {
            if d[i].abs() < 1e-6 {
                if a[i] < lo[i] || a[i] > hi[i] {
                    return false;
                }
            } else {
                let (mut ta, mut tb) = ((lo[i] - a[i]) / d[i], (hi[i] - a[i]) / d[i]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
                if t0 > t1 {
                    return false;
                }
            }
        }
        true
    })
}

/// A place to hide from a threat, plus a spot to peek out from.
#[derive(Clone, Copy, Debug, Reflect)]
pub struct Cover {
    pub spot: Vec2,
    pub peek: Vec2,
}

/// Nearest spot (to `from`) that's hidden from `threat` behind some block.
pub fn find_cover(from: Vec2, threat: Vec2) -> Option<Cover> {
    cover_blocks()
        .filter_map(|(c, half)| {
            let away = (c - threat).normalize_or_zero();
            let spot = c + away * (half + 0.9);
            if !is_clear(spot, 0.5) || !los_blocked(threat, spot) || spot.distance(threat) < 5.0 {
                return None;
            }
            // Step sideways out of cover to see the threat again.
            let side = away.perp() * (half + 1.0);
            let peek = [spot + side, spot - side]
                .into_iter()
                .find(|&p| is_clear(p, 0.5) && !los_blocked(threat, p))
                .unwrap_or(spot);
            Some(Cover { spot, peek })
        })
        .min_by(|a, b| from.distance(a.spot).total_cmp(&from.distance(b.spot)))
}
