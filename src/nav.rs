//! Navigation: a navmesh (vleue_navigator / polyanya) built from the arena's colliders, and
//! route following for anything with a `MoveTo`.

use std::f32::consts::FRAC_PI_2;

use avian3d::prelude::*;
use bevy::{ecs::system::SystemParam, prelude::*};
use vleue_navigator::prelude::*;

use crate::arena::ARENA_HALF;

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

fn plan_routes(mut commands: Commands, nav: Nav, q: Query<(Entity, &Transform, &MoveTo), Changed<MoveTo>>) {
    for (e, t, m) in &q {
        // Fall back to a straight line if the mesh isn't ready or the point is off-mesh.
        let route = nav.path(t.translation, m.dest).unwrap_or_else(|| vec![m.dest]);
        commands.entity(e).insert(Route(route));
    }
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
