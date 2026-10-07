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

/// What the floor is made of. Footsteps sound different on each, so you can tell where you are
/// by ear. Grass is the base ground; the others are painted zones (also visible on the radar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect)]
pub enum Floor {
    Grass,
    Gravel,
    Metal,
    Wood,
}

impl Floor {
    pub const ALL: [Floor; 4] = [Floor::Grass, Floor::Gravel, Floor::Metal, Floor::Wood];

    /// How far away he can hear your footsteps on this floor.
    pub fn hearing_range(self) -> f32 {
        match self {
            Floor::Grass => 4.0,
            Floor::Wood => 7.0,
            Floor::Gravel => 8.0,
            Floor::Metal => 10.0,
        }
    }

    /// How it's painted (the base floor is the ground itself, which is grass-green).
    pub fn color(self) -> Color {
        match self {
            Floor::Grass => Color::srgb(0.32, 0.45, 0.28),
            Floor::Gravel => Color::srgb(0.55, 0.52, 0.47),
            Floor::Metal => Color::srgb(0.38, 0.42, 0.47),
            Floor::Wood => Color::srgb(0.4, 0.26, 0.17),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Floor::Grass => "grass",
            Floor::Gravel => "gravel",
            Floor::Metal => "metal",
            Floor::Wood => "wood",
        }
    }
}

/// A patch of floor. Later zones paint over earlier ones.
#[derive(Clone, Copy, Debug, Reflect)]
pub struct FloorZone {
    /// Axis-aligned, in XZ.
    pub area: Rect,
    pub floor: Floor,
}

/// What the arena's floor is made of: a base material plus painted zones. It's data, so a
/// generated arena can bring its own; `classic()` is the hand-made one.
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource)]
pub struct Floors {
    pub base: Floor,
    pub zones: Vec<FloorZone>,
}

impl Default for Floors {
    fn default() -> Self {
        Self::classic()
    }
}

impl Floors {
    /// Wooden plaza in the middle; the far half (-Z) split into gravel (+X) and metal (-X); grass elsewhere.
    pub fn classic() -> Self {
        let h = ARENA_HALF;
        let zone = |min: Vec2, max: Vec2, floor| FloorZone { area: Rect::from_corners(min, max), floor };
        Self {
            base: Floor::Grass,
            zones: vec![
                zone(Vec2::new(0.0, -h), Vec2::new(h, 0.0), Floor::Gravel),
                zone(Vec2::new(-h, -h), Vec2::new(0.0, 0.0), Floor::Metal),
                zone(Vec2::splat(-7.0), Vec2::splat(7.0), Floor::Wood),
            ],
        }
    }

    pub fn at(&self, p: Vec2) -> Floor {
        self.zones.iter().rev().find(|z| z.area.contains(p)).map_or(self.base, |z| z.floor)
    }
}

pub fn plugin(app: &mut App) {
    app.insert_resource(ClearColor(Color::srgb(0.55, 0.7, 0.85)))
        .init_resource::<Floors>()
        .add_systems(Startup, spawn_arena);
}

fn spawn_arena(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    floors: Res<Floors>,
) {
    let size = ARENA_HALF * 2.0;
    commands.spawn((
        Name::new("Ground"),
        RigidBody::Static,
        Collider::cuboid(size, 0.2, size),
        Mesh3d(meshes.add(Cuboid::new(size, 0.2, size))),
        MeshMaterial3d(materials.add(floors.base.color())),
        Transform::from_xyz(0.0, -0.1, 0.0),
        RenderLayers::from_layers(WORLD_AND_RADAR),
    ));

    // Floor zones: thin painted slabs on the ground, visual only (`Floors::at` is the truth).
    // Each sits a hair above the last, so later zones paint over earlier ones.
    for (i, zone) in floors.zones.iter().enumerate() {
        let (c, size) = (zone.area.center(), zone.area.size());
        commands.spawn((
            Name::new(format!("Floor: {}", zone.floor.name())),
            Mesh3d(meshes.add(Cuboid::new(size.x, 0.01, size.y))),
            MeshMaterial3d(materials.add(zone.floor.color())),
            Transform::from_xyz(c.x, 0.005 + 0.004 * i as f32, c.y),
            RenderLayers::from_layers(WORLD_AND_RADAR),
        ));
    }

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
    /// Where to step out to see the threat again.
    pub peek: Vec2,
    /// The other side, if that one also works (else same as `peek`).
    pub alt_peek: Vec2,
}

/// Nearest spot (to `from`) that's hidden from `threat` behind some block.
pub fn find_cover(from: Vec2, threat: Vec2) -> Option<Cover> {
    find_cover_avoiding(from, threat, None)
}

/// Like [`find_cover`], but not within a few metres of `avoid` (the cover he's leaving).
pub fn find_cover_avoiding(from: Vec2, threat: Vec2, avoid: Option<Vec2>) -> Option<Cover> {
    cover_blocks()
        .filter_map(|(c, half)| {
            let away = (c - threat).normalize_or_zero();
            let spot = c + away * (half + 0.9);
            if !is_clear(spot, 0.5)
                || !los_blocked(threat, spot)
                || spot.distance(threat) < 5.0
                || avoid.is_some_and(|a| a.distance(spot) < 3.0)
            {
                return None;
            }
            // Step sideways out of cover to see the threat again.
            let side = away.perp() * (half + 1.0);
            let mut peeks = [spot + side, spot - side]
                .into_iter()
                .filter(|&p| is_clear(p, 0.5) && !los_blocked(threat, p));
            // Cover he can't shoot back from is no use to him.
            let peek = peeks.next()?;
            let alt_peek = peeks.next().unwrap_or(peek);
            Some(Cover { spot, peek, alt_peek })
        })
        .min_by(|a, b| from.distance(a.spot).total_cmp(&from.distance(b.spot)))
}
