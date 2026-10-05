//! The target: owns the main camera (his eyes), perceives the shooter, and is
//! driven by a behaviour tree (bevior_tree).
//!
//! Tree tasks are thin: they set intent components (`LookGoal`, `WanderTo`,
//! `Activity`), and plain systems turn intent into motion. Higher-priority
//! branches preempt lower ones by making the running task *fail*.

use std::f32::consts::PI;

use avian3d::prelude::*;
use bevior_tree::{
    conditional::{CondChecker, CondCheckerBuilder},
    node::Node,
    prelude::*,
};
use bevy::{camera::visibility::RenderLayers, prelude::*};
use rand::Rng;

use crate::{
    Layer,
    arena::ARENA_HALF,
    radar::RADAR_LAYER,
    round::{GameState, RoundEntity, SpawnRound, TargetMobile},
    shooter::Shooter,
};

/// Body capsule center height; eyes sit `EYE_OFFSET` above it (~1.6m).
const BODY_CENTER: f32 = 0.9;
const EYE_OFFSET: f32 = 0.7;
pub const TARGET_MAX_HP: u32 = 3;

const VIEW_RANGE: f32 = 40.0;
const VIEW_HALF_ANGLE: f32 = 0.6; // ~34°, a bit narrower than the camera so "seen" means clearly on screen
const WALK_SPEED: f32 = 2.2;

#[derive(Component)]
pub struct Target {
    pub hp: u32,
}

/// Pitch node between the body (yaw) and the camera.
#[derive(Component)]
pub struct TargetHead;

/// The camera that looks out of the target's eyes.
#[derive(Component)]
pub struct MainCamera;

/// Where the target wants to look (world space) and how fast he turns.
#[derive(Component)]
pub struct LookGoal {
    pub point: Vec3,
    pub turn_speed: f32,
}

/// Something got his attention: look toward `at` until the timer runs out.
#[derive(Component)]
pub struct Alert {
    pub at: Vec3,
    pub timer: Timer,
}

impl Alert {
    pub fn new(at: Vec3, secs: f32) -> Self {
        Self {
            at,
            timer: Timer::from_seconds(secs, TimerMode::Once),
        }
    }
}

/// How sure he is someone's out there. At 1.0 he engages, and keeps engaging until it drains to 0.
#[derive(Component, Default)]
pub struct Suspicion {
    pub level: f32,
    pub sees_shooter: bool,
    pub engaged: bool,
}

impl Suspicion {
    pub fn bump(&mut self, amount: f32) {
        self.level = (self.level + amount).min(1.0);
        if self.level >= 1.0 {
            self.engaged = true;
        }
    }
}

/// What the behaviour tree is currently doing (inserted while each task runs).
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Activity {
    Scanning,
    Wandering,
    Investigating,
    Engaging,
}

#[derive(Component)]
struct ScanPlan {
    yaw: f32,
    dwell: Timer,
}

#[derive(Component)]
pub struct WanderTo(pub Vec3);

pub fn plugin(app: &mut App) {
    app.add_systems(OnEnter(GameState::Playing), spawn_target.in_set(SpawnRound))
        .add_systems(
            Update,
            (perceive, gaze, walk).chain().run_if(in_state(GameState::Playing)),
        );
}

fn spawn_target(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut trees: ResMut<Assets<BehaviorTreeRoot>>,
) {
    let blip = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.2, 0.2),
        unlit: true,
        ..default()
    });
    let cone = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 0.9, 0.3, 0.25),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        cull_mode: None,
        ..default()
    });

    commands.spawn((
        Name::new("Target"),
        RoundEntity,
        Target { hp: TARGET_MAX_HP },
        Suspicion::default(),
        LookGoal {
            point: Vec3::new(0.0, BODY_CENTER + EYE_OFFSET, -10.0),
            turn_speed: 1.0,
        },
        RigidBody::Kinematic,
        Collider::capsule(0.4, 1.0),
        CollisionLayers::new(Layer::Target, [Layer::World, Layer::Shooter, Layer::Bullet]),
        Transform::from_xyz(0.0, BODY_CENTER, 0.0),
        Visibility::default(),
        BehaviorTree::from_node(behaviour(), &mut trees),
        children![
            (
                Name::new("Head"),
                TargetHead,
                Transform::from_xyz(0.0, EYE_OFFSET, 0.0),
                Visibility::default(),
                children![(
                    Name::new("Eyes"),
                    MainCamera,
                    Camera3d::default(),
                    Camera {
                        order: 0,
                        ..default()
                    },
                    Projection::Perspective(PerspectiveProjection {
                        fov: 70f32.to_radians(),
                        ..default()
                    }),
                )],
            ),
            (
                Name::new("Radar blip"),
                Mesh3d(meshes.add(Sphere::new(0.9))),
                MeshMaterial3d(blip),
                Transform::from_xyz(0.0, 4.0, 0.0),
                RenderLayers::layer(RADAR_LAYER),
            ),
            (
                Name::new("Radar view cone"),
                // Sector points +Y in its own plane; lay it flat, pointing forward (-Z).
                Mesh3d(meshes.add(CircularSector::new(VIEW_RANGE * 0.5, VIEW_HALF_ANGLE))),
                MeshMaterial3d(cone),
                Transform::from_xyz(0.0, 3.0, 0.0).with_rotation(Quat::from_rotation_x(-PI / 2.0)),
                RenderLayers::layer(RADAR_LAYER),
            ),
        ],
    ));
}

// ---------------------------------------------------------------------------
// Behaviour tree
// ---------------------------------------------------------------------------

fn behaviour() -> impl Node {
    InfiniteLoop::new(Selector::new(vec![
        Box::new(Sequence::new(vec![
            Box::new(CheckIf::new(Cond(|| cond(is_engaged)))),
            task(|| checker(engage), || insert_while_running(Activity::Engaging)),
        ])),
        Box::new(Sequence::new(vec![
            Box::new(CheckIf::new(Cond(|| cond(is_alerted)))),
            task(|| checker(investigate), || insert_while_running(Activity::Investigating)),
        ])),
        Box::new(Sequence::new(vec![
            Box::new(CheckIf::new(Cond(|| cond(is_mobile)))),
            task(|| checker(wander), wander_listeners),
        ])),
        task(|| checker(scan), scan_listeners),
    ]))
}

type Listeners = Vec<(TaskEvent, Box<TaskEventListener>)>;

/// A task defined by plain fn pointers, so each behaviour is just a system or two.
#[derive(Debug)]
struct FnTask {
    checker: fn() -> Box<TaskChecker>,
    listeners: fn() -> Listeners,
}

impl TaskDefinition for FnTask {
    fn build_checker(&self) -> Box<TaskChecker> {
        (self.checker)()
    }
    fn build_event_listeners(&self) -> Listeners {
        (self.listeners)()
    }
}

fn task(checker: fn() -> Box<TaskChecker>, listeners: fn() -> Listeners) -> Box<dyn Node> {
    Box::new(TaskBridge::new(Box::new(FnTask { checker, listeners })))
}

fn checker<M>(s: impl IntoSystem<In<Entity>, TaskStatus, M>) -> Box<TaskChecker> {
    Box::new(IntoSystem::into_system(s))
}

fn listener<M>(s: impl IntoSystem<In<Entity>, (), M>) -> Box<TaskEventListener> {
    Box::new(IntoSystem::into_system(s))
}

#[derive(Debug)]
struct Cond(fn() -> Box<CondChecker>);

impl CondCheckerBuilder for Cond {
    fn build(&self) -> Box<CondChecker> {
        (self.0)()
    }
}

fn cond<M>(s: impl IntoSystem<In<Entity>, bool, M>) -> Box<CondChecker> {
    Box::new(IntoSystem::into_system(s))
}

const RUNNING: TaskStatus = TaskStatus::Running;
const SUCCESS: TaskStatus = TaskStatus::Complete(NodeResult::Success);
const FAILURE: TaskStatus = TaskStatus::Complete(NodeResult::Failure);

fn is_engaged(In(e): In<Entity>, q: Query<&Suspicion>) -> bool {
    q.get(e).is_ok_and(|s| s.engaged)
}

fn is_alerted(In(e): In<Entity>, q: Query<(), With<Alert>>) -> bool {
    q.contains(e)
}

fn is_mobile(In(_): In<Entity>, mobile: Res<TargetMobile>) -> bool {
    mobile.0
}

/// True if a higher-priority branch wants control.
fn interrupted(e: Entity, suspicion: &Query<&Suspicion>, alerts: &Query<(), With<Alert>>) -> bool {
    suspicion.get(e).is_ok_and(|s| s.engaged) || alerts.contains(e)
}

fn engage(
    In(e): In<Entity>,
    mut q: Query<(&Suspicion, &mut LookGoal)>,
    shooter: Query<&Transform, With<Shooter>>,
) -> TaskStatus {
    let Ok((suspicion, mut goal)) = q.get_mut(e) else {
        return FAILURE;
    };
    if !suspicion.engaged {
        return SUCCESS;
    }
    if let Ok(shooter) = shooter.single() {
        goal.point = shooter.translation;
        goal.turn_speed = 4.0;
    }
    RUNNING
}

fn investigate(
    In(e): In<Entity>,
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(&Suspicion, &mut Alert, &mut LookGoal)>,
) -> TaskStatus {
    let Ok((suspicion, mut alert, mut goal)) = q.get_mut(e) else {
        return FAILURE;
    };
    if suspicion.engaged {
        return FAILURE;
    }
    goal.point = alert.at;
    goal.turn_speed = 3.5;
    if alert.timer.tick(time.delta()).is_finished() {
        commands.entity(e).remove::<Alert>();
        return SUCCESS;
    }
    RUNNING
}

fn scan_listeners() -> Listeners {
    let mut l = insert_while_running(Activity::Scanning);
    l.push((TaskEvent::Enter, listener(plan_scan)));
    l.push((
        TaskEvent::Exit,
        listener(|In(e): In<Entity>, mut commands: Commands| {
            commands.entity(e).try_remove::<ScanPlan>();
        }),
    ));
    l
}

fn plan_scan(In(e): In<Entity>, mut commands: Commands, q: Query<&Transform>) {
    let Ok(t) = q.get(e) else { return };
    let mut rng = rand::rng();
    let swing = rng.random_range(0.6..2.6) * if rng.random_bool(0.5) { 1.0 } else { -1.0 };
    commands.entity(e).insert(ScanPlan {
        yaw: yaw_of(t) + swing,
        dwell: Timer::from_seconds(rng.random_range(0.6..2.2), TimerMode::Once),
    });
}

fn scan(
    In(e): In<Entity>,
    time: Res<Time>,
    suspicion: Query<&Suspicion>,
    alerts: Query<(), With<Alert>>,
    mut q: Query<(&Transform, &mut LookGoal, Option<&mut ScanPlan>)>,
) -> TaskStatus {
    if interrupted(e, &suspicion, &alerts) {
        return FAILURE;
    }
    let Ok((t, mut goal, Some(mut plan))) = q.get_mut(e) else {
        return RUNNING; // plan not inserted yet
    };
    let eye = t.translation + Vec3::Y * EYE_OFFSET;
    goal.point = eye + Quat::from_rotation_y(plan.yaw) * Vec3::NEG_Z * 10.0;
    goal.turn_speed = 1.2;
    if angle_diff(yaw_of(t), plan.yaw).abs() < 0.05 && plan.dwell.tick(time.delta()).is_finished() {
        return SUCCESS;
    }
    RUNNING
}

fn wander_listeners() -> Listeners {
    let mut l = insert_while_running(Activity::Wandering);
    l.push((
        TaskEvent::Enter,
        listener(|In(e): In<Entity>, mut commands: Commands| {
            let mut rng = rand::rng();
            let r = ARENA_HALF - 4.0;
            let dest = Vec3::new(rng.random_range(-r..r), BODY_CENTER, rng.random_range(-r..r));
            commands.entity(e).insert(WanderTo(dest));
        }),
    ));
    l.push((
        TaskEvent::Exit,
        listener(|In(e): In<Entity>, mut commands: Commands| {
            commands.entity(e).try_remove::<WanderTo>();
        }),
    ));
    l
}

fn wander(
    In(e): In<Entity>,
    mobile: Res<TargetMobile>,
    suspicion: Query<&Suspicion>,
    alerts: Query<(), With<Alert>>,
    mut q: Query<(&Transform, &mut LookGoal, Option<&WanderTo>)>,
) -> TaskStatus {
    if interrupted(e, &suspicion, &alerts) {
        return FAILURE;
    }
    if !mobile.0 {
        return SUCCESS;
    }
    let Ok((t, mut goal, Some(dest))) = q.get_mut(e) else {
        return RUNNING;
    };
    goal.point = dest.0 + Vec3::Y * EYE_OFFSET;
    goal.turn_speed = 2.0;
    if t.translation.xz().distance(dest.0.xz()) < 0.6 {
        return SUCCESS;
    }
    RUNNING
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Vision: is the shooter inside the view cone with clear line of sight? Feeds `Suspicion`.
fn perceive(
    time: Res<Time>,
    spatial: SpatialQuery,
    eyes: Single<&GlobalTransform, With<MainCamera>>,
    mut target: Single<(Entity, &mut Suspicion), With<Target>>,
    shooter: Single<(Entity, &Transform, &LinearVelocity), With<Shooter>>,
    mut commands: Commands,
) {
    let (target_e, ref mut suspicion) = *target;
    let (shooter_e, shooter_t, shooter_v) = *shooter;
    let eye = eyes.translation();
    let to_shooter = shooter_t.translation - eye;
    let dist = to_shooter.length();
    let angle = eyes.forward().angle_between(to_shooter);

    let sees = dist < VIEW_RANGE
        && angle < VIEW_HALF_ANGLE
        && Dir3::new(to_shooter).is_ok_and(|dir| {
            spatial
                .cast_ray(
                    eye,
                    dir,
                    dist + 1.0,
                    true,
                    &SpatialQueryFilter::from_mask([Layer::World, Layer::Shooter]),
                )
                .is_some_and(|hit| hit.entity == shooter_e)
        });
    suspicion.sees_shooter = sees;

    let dt = time.delta_secs();
    if sees {
        let closeness = 1.0 - dist / VIEW_RANGE;
        let centred = 1.0 - angle / VIEW_HALF_ANGLE;
        let moving = if shooter_v.length() > 0.5 { 1.0 } else { 0.25 };
        suspicion.bump((0.08 + 0.6 * closeness) * (0.4 + 0.6 * centred) * moving * dt);
        // Half-sure: glance over.
        if suspicion.level > 0.5 && !suspicion.engaged {
            commands.entity(target_e).insert(Alert::new(shooter_t.translation, 1.5));
        }
    } else {
        suspicion.level = (suspicion.level - 0.12 * dt).max(0.0);
        if suspicion.level <= 0.0 {
            suspicion.engaged = false;
        }
    }
}

/// Turn the body (yaw) and head (pitch) toward the `LookGoal`.
fn gaze(
    time: Res<Time>,
    mut body: Single<(&mut Transform, &LookGoal), With<Target>>,
    mut head: Single<&mut Transform, (With<TargetHead>, Without<Target>)>,
) {
    let (ref mut t, goal) = *body;
    let eye = t.translation + Vec3::Y * EYE_OFFSET;
    let d = goal.point - eye;
    let step = goal.turn_speed * time.delta_secs();

    let yaw = yaw_of(t);
    let desired_yaw = f32::atan2(-d.x, -d.z);
    let new_yaw = yaw + angle_diff(yaw, desired_yaw).clamp(-step, step);
    t.rotation = Quat::from_rotation_y(new_yaw);

    // Pitch, plus a little idle sway so the view feels alive.
    let desired_pitch = f32::atan2(d.y, d.xz().length()).clamp(-0.6, 0.6)
        + 0.03 * (time.elapsed_secs() * 0.7).sin();
    let (pitch, _, _) = head.rotation.to_euler(EulerRot::XYZ);
    let new_pitch = pitch + (desired_pitch - pitch).clamp(-step, step);
    head.rotation = Quat::from_rotation_x(new_pitch);
}

/// Walk toward `WanderTo`, once roughly facing it.
fn walk(mut q: Query<(&Transform, &mut LinearVelocity, Option<&WanderTo>), With<Target>>) {
    for (t, mut v, dest) in &mut q {
        v.0 = match dest {
            Some(dest) => {
                let d = (dest.0 - t.translation).with_y(0.0);
                let facing = t.forward().angle_between(d) < 0.4;
                if facing { d.normalize_or_zero() * WALK_SPEED } else { Vec3::ZERO }
            }
            None => Vec3::ZERO,
        };
    }
}

fn yaw_of(t: &Transform) -> f32 {
    t.rotation.to_euler(EulerRot::YXZ).0
}

/// Shortest signed angle from `a` to `b`.
fn angle_diff(a: f32, b: f32) -> f32 {
    (b - a + PI).rem_euclid(2.0 * PI) - PI
}
