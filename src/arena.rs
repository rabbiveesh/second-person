//! Level geometry: ground, perimeter walls and cover, all described by a `Layout`.
//!
//! The layout is data: `Layout::classic()` is the original hand-placed arena, and
//! `Layout::random(seed)` generates a new outline (yard, hall, L, cross, octagon, notched)
//! scattered with cover. The arena is respawned from the current layout every round.

use avian3d::prelude::*;
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::RenderLayers,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use rand::{Rng, SeedableRng, rngs::StdRng};

use crate::{
    nav::NavObstacle,
    radar::WORLD_AND_RADAR,
    round::{GameState, RoundEntity, SpawnRound},
};

/// Half-size of the classic square arena (to the middle of its walls).
const CLASSIC_HALF: f32 = 30.0;
const WALL_THICKNESS: f32 = 1.0;
const WALL_HEIGHT: f32 = 3.0;
/// Minimum walkable gap between generated cover blocks, and between cover and walls. Wider
/// than an agent (navmesh radius 0.6) so every open region stays connected.
const COVER_GAP: f32 = 2.6;
/// Generated cover keeps this far from the target's spawn at the origin.
const SPAWN_CLEARANCE: f32 = 4.0;

/// Which layout the next round uses.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug, Reflect)]
#[reflect(Resource)]
pub enum ArenaMode {
    /// The original hand-placed arena.
    Classic,
    /// A freshly generated arena every round.
    #[default]
    Random,
    /// A specific generated arena (replays, levels, tests).
    Seed(u64),
}

impl ArenaMode {
    pub fn next(self) -> Self {
        match self {
            Self::Classic => Self::Random,
            Self::Random | Self::Seed(_) => Self::Classic,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum BlockKind {
    Crate,
    Pillar,
    Barrier,
}

/// An axis-aligned block of cover. All cover is taller than eye height.
#[derive(Clone, Copy, Debug, Reflect)]
pub struct Block {
    pub center: Vec2,
    /// Half extent in X and Z.
    pub half: Vec2,
    pub height: f32,
    pub kind: BlockKind,
}

impl Block {
    /// How far the block extends from its centre along `dir` (a unit vector).
    fn support(&self, dir: Vec2) -> f32 {
        dir.x.abs() * self.half.x + dir.y.abs() * self.half.y
    }

    /// Gap between two blocks along the axis where they're furthest apart (negative = overlap).
    fn gap(&self, other: &Block) -> f32 {
        ((self.center - other.center).abs() - (self.half + other.half)).max_element()
    }
}

/// The arena: a walled floor outline (XZ, counter-clockwise, at the inner face of the walls)
/// and the cover inside it. The target always spawns at the origin, which is kept open.
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource)]
pub struct Layout {
    pub name: String,
    pub outline: Vec<Vec2>,
    pub blocks: Vec<Block>,
}

impl Default for Layout {
    fn default() -> Self {
        Self::classic()
    }
}

impl Layout {
    /// The original hand-placed arena, so rounds are reproducible.
    pub fn classic() -> Self {
        // Crates are (x, z, half-height).
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
        const PILLARS: &[(f32, f32)] = &[(-10.0, -2.0), (10.0, -2.0), (0.0, -18.0), (-18.0, 20.0), (22.0, 12.0)];
        let crates = CRATES.iter().map(|&(x, z, h)| Block {
            center: Vec2::new(x, z),
            half: Vec2::splat(1.0),
            height: h * 2.0,
            kind: BlockKind::Crate,
        });
        let pillars = PILLARS.iter().map(|&(x, z)| Block {
            center: Vec2::new(x, z),
            half: Vec2::splat(0.6),
            height: 5.0,
            kind: BlockKind::Pillar,
        });
        let h = CLASSIC_HALF - WALL_THICKNESS / 2.0;
        Self {
            name: "Classic".into(),
            outline: rect(h, h),
            blocks: crates.chain(pillars).collect(),
        }
    }

    /// A generated arena. The same seed always gives the same layout.
    pub fn random(seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let (shape, mut outline) = random_outline(&mut rng);
        for _ in 0..rng.random_range(0..4) {
            for p in &mut outline {
                *p = p.perp();
            }
        }
        let mut layout = Self {
            name: format!("{shape} #{seed}"),
            outline: counter_clockwise(outline),
            blocks: vec![],
        };
        layout.scatter_cover(&mut rng);
        layout
    }

    fn scatter_cover(&mut self, rng: &mut impl Rng) {
        let want = (self.area() / rng.random_range(70.0..110.0)) as usize;
        let bounds = self.bounds();
        for _ in 0..want * 50 {
            if self.blocks.len() >= want {
                break;
            }
            let center = Vec2::new(
                rng.random_range(bounds.min.x..bounds.max.x),
                rng.random_range(bounds.min.y..bounds.max.y),
            );
            let block = random_block(rng, center);
            let away_from_walls = self.contains(center) && self.wall_distance(center) >= block.half.length() + COVER_GAP;
            let spawn_open = (center.abs() - block.half).max_element() >= SPAWN_CLEARANCE;
            if away_from_walls && spawn_open && self.blocks.iter().all(|b| b.gap(&block) >= COVER_GAP) {
                self.blocks.push(block);
            }
        }
    }

    pub fn bounds(&self) -> Rect {
        self.outline
            .iter()
            .fold(Rect::from_center_size(self.outline[0], Vec2::ZERO), |r, &p| {
                r.union_point(p)
            })
    }

    /// Floor area inside the walls.
    pub fn area(&self) -> f32 {
        self.edges().map(|(a, b)| a.perp_dot(b)).sum::<f32>() / 2.0
    }

    fn edges(&self) -> impl Iterator<Item = (Vec2, Vec2)> + '_ {
        let n = self.outline.len();
        (0..n).map(move |i| (self.outline[i], self.outline[(i + 1) % n]))
    }

    /// Is `p` (XZ) inside the walls?
    pub fn contains(&self, p: Vec2) -> bool {
        // Even-odd ray cast towards +X.
        self.edges()
            .filter(|&(a, b)| (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x))
            .count()
            % 2
            == 1
    }

    fn wall_distance(&self, p: Vec2) -> f32 {
        self.edges()
            .map(|(a, b)| {
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
                p.distance(a + ab * t)
            })
            .fold(f32::INFINITY, f32::min)
    }

    /// Is a circle of `radius` at `p` (XZ) inside the walls and clear of all cover?
    pub fn is_clear(&self, p: Vec2, radius: f32) -> bool {
        self.contains(p)
            && self.wall_distance(p) > radius
            && self
                .blocks
                .iter()
                .all(|b| ((p - b.center).abs() - b.half).max_element() > radius)
    }

    /// Does any cover or wall sit between `a` and `b` (top-down)?
    pub fn los_blocked(&self, a: Vec2, b: Vec2) -> bool {
        self.blocks.iter().any(|block| segment_hits_box(a, b, block.center - block.half, block.center + block.half))
            || self.edges().any(|(c, d)| segments_cross(a, b, c, d))
    }

    /// Nearest spot (to `from`) that's hidden from `threat` behind some block, plus a spot
    /// to peek out from.
    pub fn find_cover(&self, from: Vec2, threat: Vec2) -> Option<Cover> {
        self.blocks
            .iter()
            .filter_map(|block| {
                let away = (block.center - threat).normalize_or_zero();
                let spot = block.center + away * (block.support(away) + 0.9);
                if !self.is_clear(spot, 0.5) || !self.los_blocked(threat, spot) || spot.distance(threat) < 5.0 {
                    return None;
                }
                // Step sideways out of cover to see the threat again.
                let perp = away.perp();
                let side = perp * (block.support(perp) + 1.0);
                let peek = [spot + side, spot - side]
                    .into_iter()
                    .find(|&p| self.is_clear(p, 0.5) && !self.los_blocked(threat, p))
                    .unwrap_or(spot);
                Some(Cover { spot, peek })
            })
            .min_by(|a, b| from.distance(a.spot).total_cmp(&from.distance(b.spot)))
    }

    /// A random point where a circle of `radius` is clear and `accept` holds, if one turns up.
    pub fn random_point(&self, rng: &mut impl Rng, radius: f32, accept: impl Fn(Vec2) -> bool) -> Option<Vec2> {
        let b = self.bounds();
        (0..5000)
            .map(|_| Vec2::new(rng.random_range(b.min.x..b.max.x), rng.random_range(b.min.y..b.max.y)))
            .find(|&p| self.is_clear(p, radius) && accept(p))
    }
}

/// A place to hide from a threat, plus a spot to peek out from.
#[derive(Clone, Copy, Debug, Reflect)]
pub struct Cover {
    pub spot: Vec2,
    pub peek: Vec2,
}

fn rect(hx: f32, hz: f32) -> Vec<Vec2> {
    vec![Vec2::new(-hx, -hz), Vec2::new(hx, -hz), Vec2::new(hx, hz), Vec2::new(-hx, hz)]
}

fn counter_clockwise(mut outline: Vec<Vec2>) -> Vec<Vec2> {
    let n = outline.len();
    let twice_area: f32 = (0..n).map(|i| outline[i].perp_dot(outline[(i + 1) % n])).sum();
    if twice_area < 0.0 {
        outline.reverse();
    }
    outline
}

/// A random floor shape around the origin (which always stays well inside).
fn random_outline(rng: &mut impl Rng) -> (&'static str, Vec<Vec2>) {
    let v = Vec2::new;
    match rng.random_range(0..6) {
        0 => ("Yard", rect(rng.random_range(14.0..36.0), rng.random_range(14.0..36.0))),
        1 => ("Hall", rect(rng.random_range(34.0..50.0), rng.random_range(8.0..13.0))),
        2 => {
            // A rectangle with one corner cut away.
            let (hx, hz) = (rng.random_range(20.0..36.0), rng.random_range(20.0..36.0));
            let (cx, cz) = (rng.random_range(8.0..hx - 8.0), rng.random_range(8.0..hz - 8.0));
            let outline = vec![
                v(-hx, -hz),
                v(hx, -hz),
                v(hx, hz - cz),
                v(hx - cx, hz - cz),
                v(hx - cx, hz),
                v(-hx, hz),
            ];
            ("L", outline)
        }
        3 => {
            // Four arms of different lengths.
            let a = rng.random_range(7.0..12.0);
            let mut arm = || rng.random_range(a + 10.0..36.0);
            let (e, n, w, s) = (arm(), arm(), arm(), arm());
            let outline = vec![
                v(-a, -s),
                v(a, -s),
                v(a, -a),
                v(e, -a),
                v(e, a),
                v(a, a),
                v(a, n),
                v(-a, n),
                v(-a, a),
                v(-w, a),
                v(-w, -a),
                v(-a, -a),
            ];
            ("Cross", outline)
        }
        4 => {
            // A rectangle with chamfered corners.
            let (hx, hz): (f32, f32) = (rng.random_range(18.0..36.0), rng.random_range(18.0..36.0));
            let c = rng.random_range(6.0..hx.min(hz) - 6.0);
            let outline = vec![
                v(-hx + c, -hz),
                v(hx - c, -hz),
                v(hx, -hz + c),
                v(hx, hz - c),
                v(hx - c, hz),
                v(-hx + c, hz),
                v(-hx, hz - c),
                v(-hx, -hz + c),
            ];
            ("Octagon", outline)
        }
        _ => {
            // A rectangle with a notch bitten out of one side.
            let (hx, hz) = (rng.random_range(20.0..36.0), rng.random_range(20.0..36.0));
            let (nw, d) = (rng.random_range(4.0..hx - 8.0), rng.random_range(6.0..hz - 8.0));
            let outline = vec![
                v(-hx, -hz),
                v(hx, -hz),
                v(hx, hz),
                v(nw, hz),
                v(nw, hz - d),
                v(-nw, hz - d),
                v(-nw, hz),
                v(-hx, hz),
            ];
            ("Notch", outline)
        }
    }
}

fn random_block(rng: &mut impl Rng, center: Vec2) -> Block {
    let roll: f32 = rng.random();
    if roll < 0.5 {
        Block {
            center,
            half: Vec2::splat(rng.random_range(0.8..1.5)),
            height: rng.random_range(2.0..3.0),
            kind: BlockKind::Crate,
        }
    } else if roll < 0.7 {
        Block {
            center,
            half: Vec2::splat(rng.random_range(0.5..0.8)),
            height: 5.0,
            kind: BlockKind::Pillar,
        }
    } else {
        let (long, thin) = (rng.random_range(2.0..5.0), rng.random_range(0.35..0.6));
        Block {
            center,
            half: if rng.random() { Vec2::new(long, thin) } else { Vec2::new(thin, long) },
            height: rng.random_range(2.2..3.0),
            kind: BlockKind::Barrier,
        }
    }
}

/// Slab test: does the segment `a`→`b` touch the box `lo`..`hi`?
fn segment_hits_box(a: Vec2, b: Vec2, lo: Vec2, hi: Vec2) -> bool {
    let d = b - a;
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
}

/// Do segments `a`→`b` and `c`→`d` properly cross?
fn segments_cross(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let side = |p: Vec2, q: Vec2, r: Vec2| (q - p).perp_dot(r - p);
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

pub fn plugin(app: &mut App) {
    app.insert_resource(ClearColor(Color::srgb(0.55, 0.7, 0.85)))
        .init_resource::<ArenaMode>()
        .init_resource::<Layout>()
        .add_systems(Startup, spawn_lighting)
        .add_systems(
            OnEnter(GameState::Playing),
            (choose_layout.before(SpawnRound), spawn_arena.in_set(SpawnRound)),
        );
}

pub(crate) fn choose_layout(mode: Res<ArenaMode>, mut layout: ResMut<Layout>) {
    *layout = match *mode {
        ArenaMode::Classic => Layout::classic(),
        // Short seeds read better on the HUD.
        ArenaMode::Random => Layout::random(rand::rng().random_range(0..100_000)),
        ArenaMode::Seed(seed) => Layout::random(seed),
    };
}

fn spawn_lighting(mut commands: Commands) {
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
}

pub(crate) fn spawn_arena(
    mut commands: Commands,
    layout: Res<Layout>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Physics floor: a slab under the whole outline (nothing can get outside the walls).
    let bounds = layout.bounds();
    let size = bounds.size() + WALL_THICKNESS * 2.0;
    commands.spawn((
        Name::new("Ground"),
        RoundEntity,
        RigidBody::Static,
        Collider::cuboid(size.x, 0.2, size.y),
        Transform::from_xyz(bounds.center().x, -0.1, bounds.center().y),
    ));
    commands.spawn((
        Name::new("Floor"),
        RoundEntity,
        Mesh3d(meshes.add(floor_mesh(&layout.outline))),
        MeshMaterial3d(materials.add(Color::srgb(0.32, 0.45, 0.28))),
        Transform::default(),
        RenderLayers::from_layers(WORLD_AND_RADAR),
    ));

    let wall_mat = materials.add(Color::srgb(0.5, 0.5, 0.55));
    let n = layout.outline.len();
    let corner = |i: usize| layout.outline[(i + n) % n];
    // Interior angle < 180°: extend the walls past the corner so they meet.
    let convex = |i: usize| (corner(i) - corner(i + n - 1)).perp_dot(corner(i + 1) - corner(i)) > 0.0;
    for i in 0..n {
        let (a, b) = (corner(i), corner(i + 1));
        let dir = (b - a).normalize();
        // Counter-clockwise outline: outward is to the right of each edge.
        let out = Vec2::new(dir.y, -dir.x);
        let ext = |c: bool| if c { WALL_THICKNESS / 2.0 } else { 0.0 };
        let (ext_a, ext_b) = (ext(convex(i)), ext(convex(i + 1)));
        let len = a.distance(b) + ext_a + ext_b;
        let mid = (a - dir * ext_a + b + dir * ext_b) / 2.0 + out * (WALL_THICKNESS / 2.0);
        spawn_block(
            &mut commands,
            &mut meshes,
            wall_mat.clone(),
            "Wall",
            Transform::from_xyz(mid.x, WALL_HEIGHT / 2.0, mid.y).with_rotation(Quat::from_rotation_y(f32::atan2(-dir.y, dir.x))),
            Vec3::new(len / 2.0, WALL_HEIGHT / 2.0, WALL_THICKNESS / 2.0),
        );
    }

    let crate_mat = materials.add(Color::srgb(0.6, 0.42, 0.25));
    let pillar_mat = materials.add(Color::srgb(0.7, 0.7, 0.72));
    let barrier_mat = materials.add(Color::srgb(0.45, 0.48, 0.55));
    for block in &layout.blocks {
        let (name, material) = match block.kind {
            BlockKind::Crate => ("Crate", &crate_mat),
            BlockKind::Pillar => ("Pillar", &pillar_mat),
            BlockKind::Barrier => ("Barrier", &barrier_mat),
        };
        spawn_block(
            &mut commands,
            &mut meshes,
            material.clone(),
            name,
            Transform::from_xyz(block.center.x, block.height / 2.0, block.center.y),
            Vec3::new(block.half.x, block.height / 2.0, block.half.y),
        );
    }
}

/// The floor polygon, triangulated, facing up at y = 0.
fn floor_mesh(outline: &[Vec2]) -> Mesh {
    let mut indices: Vec<u32> = vec![];
    earcut::Earcut::new().earcut(outline.iter().map(|p| [p.x, p.y]), &[], &mut indices);
    // Face +Y: seen from above, front faces wind clockwise in (x, z).
    for tri in indices.chunks_exact_mut(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| outline[i as usize]);
        if (b - a).perp_dot(c - a) > 0.0 {
            tri.swap(1, 2);
        }
    }
    let positions: Vec<[f32; 3]> = outline.iter().map(|p| [p.x, 0.0, p.y]).collect();
    let uvs: Vec<[f32; 2]> = outline.iter().map(|p| [p.x / 4.0, p.y / 4.0]).collect();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; outline.len()])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

fn spawn_block(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    name: &'static str,
    transform: Transform,
    half: Vec3,
) {
    let full = half * 2.0;
    commands.spawn((
        Name::new(name),
        RoundEntity,
        RigidBody::Static,
        Collider::cuboid(full.x, full.y, full.z),
        Mesh3d(meshes.add(Cuboid::new(full.x, full.y, full.z))),
        MeshMaterial3d(material),
        transform,
        RenderLayers::from_layers(WORLD_AND_RADAR),
        NavObstacle,
    ));
}
