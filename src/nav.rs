//! Navigation: a navmesh (vleue_navigator / polyanya) built from the arena's colliders, and
//! route following for anything with a `MoveTo`.

use std::f32::consts::FRAC_PI_2;

use avian3d::prelude::*;
use bevy::{ecs::system::SystemParam, prelude::*};
use vleue_navigator::prelude::*;

use crate::arena::{self, ARENA_HALF};

/// Marks colliders the navmesh should route around.
#[derive(Component, Default)]
pub struct NavObstacle;

/// Go to `dest` at `speed`. Routed via the navmesh; removing it stops the mover.
#[derive(Component, Reflect, Clone, Copy)]
#[reflect(Component)]
pub struct MoveTo {
    pub dest: Vec3,
    pub speed: f32,
    /// Move without turning to face the path (e.g. side-stepping while watching a threat).
    pub strafe: bool,
}

/// Zig-zag across the line of fire from this threat (XZ) on legs that run along it. Goes with a
/// `MoveTo`, e.g. while running for cover.
#[derive(Component, Reflect, Clone, Copy)]
#[reflect(Component)]
pub struct Evade(pub Vec2);

/// Zig-zag: distance between swerves, and how far to each side.
const WEAVE_STEP: f32 = 2.5;
const WEAVE_SIDE: f32 = 1.4;

/// Remaining waypoints (XZ; y is ignored) for the current `MoveTo`.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Route(pub Vec<Vec3>);

impl Route {
    pub fn next(&self) -> Option<Vec3> {
        self.0.first().copied()
    }
}

pub fn plugin(app: &mut App) {
    app.add_plugins((
        VleueNavigatorPlugin,
        NavmeshUpdaterPlugin::<Collider, NavObstacle>::default(),
    ))
    .add_systems(Startup, spawn_navmesh)
    .add_systems(Update, (clear_routes, plan_routes).chain());
}

fn spawn_navmesh(mut commands: Commands) {
    let h = ARENA_HALF - 0.5;
    commands.spawn((
        Name::new("Navmesh"),
        ManagedNavMesh::single(),
        NavMeshSettings {
            fixed: Triangulation::from_outer_edges(&[
                Vec2::new(-h, -h),
                Vec2::new(h, -h),
                Vec2::new(h, h),
                Vec2::new(-h, h),
            ]),
            agent_radius: 0.6,
            simplify: 0.01,
            ..default()
        },
        // The arena is static: build synchronously whenever obstacles change (i.e. once).
        NavMeshUpdateMode::Direct,
        NavMeshUpdateModeBlocking,
        // Navmesh lives in its local XY plane; lay it on the ground (XZ).
        Transform::from_xyz(0.0, 0.1, 0.0).with_rotation(Quat::from_rotation_x(FRAC_PI_2)),
    ));
}

/// Read access to the arena navmesh.
#[derive(SystemParam)]
pub struct Nav<'w, 's> {
    navmeshes: Res<'w, Assets<NavMesh>>,
    status: Query<'w, 's, &'static NavMeshStatus>,
}

impl Nav<'_, '_> {
    pub fn ready(&self) -> bool {
        self.status.iter().any(|s| *s == NavMeshStatus::Built)
    }

    /// Waypoints from `from` to `to`, or `None` if there's no navmesh yet / no path.
    pub fn path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        let mesh = self.navmeshes.get(&ManagedNavMesh::get_single())?;
        mesh.transformed_path(from.with_y(0.1), to.with_y(0.1)).map(|p| p.path)
    }
}

fn plan_routes(
    mut commands: Commands,
    nav: Nav,
    q: Query<(Entity, &Transform, &MoveTo, Option<&Evade>), Changed<MoveTo>>,
) {
    for (e, t, m, evade) in &q {
        // Fall back to a straight line if the mesh isn't ready or the point is off-mesh.
        let mut route = nav.path(t.translation, m.dest).unwrap_or_else(|| vec![m.dest]);
        if let Some(Evade(threat)) = evade {
            route = weave(t.translation, &route, *threat);
        }
        commands.entity(e).insert(Route(route));
    }
}

/// Breaks up legs that run along the line of fire from `threat` into swerves either side of it,
/// so he never holds a straight line a shooter can just keep firing down. Swerves that would
/// leave the arena, clip cover or cut behind a block are skipped (that stretch stays straight).
pub fn weave(from: Vec3, route: &[Vec3], threat: Vec2) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(route.len());
    let mut a = from.xz();
    let mut side = 1.0;
    for &w in route {
        let b = w.xz();
        let leg = b - a;
        let len = leg.length();
        let dir = leg / len.max(1e-6);
        let along_fire = dir.dot((a - threat).normalize_or_zero()).abs() > 0.6;
        if along_fire && len > WEAVE_STEP * 1.5 {
            let mut prev = a;
            let swerves = (len / WEAVE_STEP) as usize;
            for i in 1..swerves {
                let p = a + dir * (i as f32 * WEAVE_STEP) + dir.perp() * WEAVE_SIDE * side;
                if arena::is_clear(p, 0.6) && !arena::los_blocked(prev, p) && !arena::los_blocked(p, b) {
                    out.push(p.extend(w.y).xzy());
                    prev = p;
                    side = -side;
                }
            }
        }
        out.push(w);
        a = b;
    }
    out
}

fn clear_routes(
    mut commands: Commands,
    mut removed: RemovedComponents<MoveTo>,
    still_moving: Query<(), With<MoveTo>>,
) {
    // Skip entities that got a new `MoveTo` in the same frame (remove + re-insert).
    for e in removed.read().filter(|e| !still_moving.contains(*e)) {
        if let Ok(mut ec) = commands.get_entity(e) {
            ec.try_remove::<Route>();
        }
    }
}
