//! The target: owns the main camera (his eyes), perceives the shooter, and is
//! driven by a behaviour tree (bevior_tree).
//!
//! Tree tasks are thin: they set intent components (`LookGoal`, `MoveTo`,
//! `Activity`), and plain systems turn intent into motion (`nav` routes, `walk` follows).
//! Higher-priority branches preempt lower ones by making the running task *fail*.
//!
//! Once engaged he breaks line of sight: runs to cover (no shooting while running), then
//! fights from it, peeking out to shoot and ducking back. Getting hit makes him relocate.

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
    arena::{self, ARENA_HALF, Cover},
    nav::{MoveTo, Route},
    radar::{LiveBlip, RADAR_LAYER, RadarContact},
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
const RUN_SPEED: f32 = 5.5;
/// "Close enough" for every arrival check (walk itself homes in to 0.2m).
const ARRIVE: f32 = 0.5;

#[derive(Component, Reflect)]
#[reflect(Component)]
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
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct LookGoal {
    pub point: Vec3,
    pub turn_speed: f32,
}

/// Something got his attention: look toward `at` until the timer runs out.
#[derive(Component, Reflect)]
#[reflect(Component)]
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
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Suspicion {
    pub level: f32,
    pub sees_shooter: bool,
    pub engaged: bool,
    /// Where he last saw or heard the shooter.
    pub last_known: Option<Vec3>,
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
#[derive(Component, Reflect, Clone, Copy, PartialEq, Eq, Debug)]
#[reflect(Component)]
pub enum Activity {
    Scanning,
    Wandering,
    Investigating,
    TakingCover,
    Engaging,
}

#[derive(Component)]
struct ScanPlan {
    yaw: f32,
    dwell: Timer,
}

/// The cover he's using while engaged.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct CoverPlan(pub Cover);

/// Fighting from cover: hide, peek, repeat.
#[derive(Component, Reflect)]
#[reflect(Component)]
struct Fighting {
    peeking: bool,
    timer: Timer,
    hp_at_start: u32,
    /// The side he's peeking from this time.
    peek: Vec2,
    /// Peeks left before he moves to a different cover.
    peeks_left: u32,
}

/// Leave this cover spot: the next cover plan avoids it.
#[derive(Component)]
struct Relocate(Vec2);

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
        RadarContact(Color::srgb(1.0, 0.2, 0.2)),
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
                LiveBlip,
            ),
            (
                Name::new("Radar view cone"),
                // Sector points +Y in its own plane; lay it flat, pointing forward (-Z).
                Mesh3d(meshes.add(CircularSector::new(VIEW_RANGE * 0.5, VIEW_HALF_ANGLE))),
                MeshMaterial3d(cone),
                Transform::from_xyz(0.0, 3.0, 0.0).with_rotation(Quat::from_rotation_x(-PI / 2.0)),
                RenderLayers::layer(RADAR_LAYER),
                LiveBlip,
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
            task(|| checker(take_cover), take_cover_listeners),
            task(|| checker(fight), fight_listeners),
        ])),
        Box::new(Sequence::new(vec![
            Box::new(CheckIf::new(Cond(|| cond(is_alerted)))),
            task(|| checker(investigate), investigate_listeners),
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

fn take_cover_listeners() -> Listeners {
    let mut l = insert_while_running(Activity::TakingCover);
    l.push((TaskEvent::Enter, listener(plan_cover)));
    l.push((
        TaskEvent::Exit,
        listener(|In(e): In<Entity>, mut commands: Commands| {
            commands.entity(e).try_remove::<MoveTo>();
        }),
    ));
    l
}

fn plan_cover(
    In(e): In<Entity>,
    mut commands: Commands,
    q: Query<(&Transform, &Suspicion, Option<&Relocate>)>,
) {
    let Ok((t, s, relocate)) = q.get(e) else { return };
    let here = t.translation.xz();
    let threat = s.last_known.map_or(here + t.forward().xz() * 10.0, |p| p.xz());
    let cover = arena::find_cover_avoiding(here, threat, relocate.map(|r| r.0))
        .or_else(|| arena::find_cover(here, threat))
        .unwrap_or(Cover { spot: here, peek: here, alt_peek: here });
    commands.entity(e).try_remove::<Relocate>();
    commands.entity(e).insert((
        CoverPlan(cover),
        MoveTo {
            dest: cover.spot.extend(BODY_CENTER).xzy(),
            speed: RUN_SPEED,
            strafe: false,
        },
    ));
}

fn take_cover(In(e): In<Entity>, q: Query<(&Transform, Option<&CoverPlan>)>) -> TaskStatus {
    let Ok((t, Some(plan))) = q.get(e) else {
        return RUNNING; // plan not inserted yet
    };
    if t.translation.xz().distance(plan.0.spot) < ARRIVE { SUCCESS } else { RUNNING }
}

fn fight_listeners() -> Listeners {
    let mut l = insert_while_running(Activity::Engaging);
    l.push((
        TaskEvent::Enter,
        listener(|In(e): In<Entity>, mut commands: Commands, q: Query<&Target>| {
            let hp = q.get(e).map_or(0, |t| t.hp);
            let mut rng = rand::rng();
            commands.entity(e).insert(Fighting {
                peeking: false,
                timer: Timer::from_seconds(rng.random_range(0.5..1.6), TimerMode::Once),
                hp_at_start: hp,
                peek: Vec2::ZERO,
                peeks_left: rng.random_range(1..=3),
            });
        }),
    ));
    l.push((
        TaskEvent::Exit,
        listener(|In(e): In<Entity>, mut commands: Commands| {
            commands.entity(e).try_remove::<(Fighting, MoveTo)>();
        }),
    ));
    l
}

/// Hide behind cover, then peek out to shoot, then hide again. Each peek picks a random side
/// and duration (sometimes just a quick glance). Succeeds (so the tree re-plans cover) when
/// he's no longer engaged, or, moving to a *different* cover, when he gets hit or after a few peeks.
fn fight(
    In(e): In<Entity>,
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(
        &Transform,
        &Target,
        &Suspicion,
        &CoverPlan,
        &mut LookGoal,
        Option<&mut Fighting>,
        Has<MoveTo>,
    )>,
) -> TaskStatus {
    let Ok((t, target, suspicion, plan, mut look, Some(mut fighting), moving)) = q.get_mut(e) else {
        return RUNNING;
    };
    if !suspicion.engaged {
        return SUCCESS;
    }
    if target.hp < fighting.hp_at_start {
        commands.entity(e).insert(Relocate(plan.0.spot));
        return SUCCESS;
    }
    if let Some(p) = suspicion.last_known {
        look.point = p + Vec3::Y * EYE_OFFSET;
        look.turn_speed = 4.0;
    }
    let goal = if fighting.peeking { fighting.peek } else { plan.0.spot };
    if t.translation.xz().distance(goal) >= ARRIVE {
        if !moving {
            commands.entity(e).insert(MoveTo {
                dest: goal.extend(BODY_CENTER).xzy(),
                speed: RUN_SPEED,
                strafe: true,
            });
        }
        return RUNNING;
    }
    if fighting.timer.tick(time.delta()).is_finished() {
        let mut rng = rand::rng();
        let peeking = !fighting.peeking;
        if !peeking {
            fighting.peeks_left = fighting.peeks_left.saturating_sub(1);
        } else if fighting.peeks_left == 0 {
            // Done here: run to another cover instead of peeking from the same spot again.
            commands.entity(e).insert(Relocate(plan.0.spot));
            return SUCCESS;
        }
        if peeking {
            fighting.peek = if rng.random_bool(0.5) { plan.0.peek } else { plan.0.alt_peek };
        }
        let next = if peeking { fighting.peek } else { plan.0.spot };
        fighting.peeking = peeking;
        let secs = if !peeking {
            rng.random_range(0.5..2.2)
        } else if rng.random_bool(0.3) {
            rng.random_range(0.3..0.6) // quick glance
        } else {
            rng.random_range(1.2..2.6)
        };
        fighting.timer = Timer::from_seconds(secs, TimerMode::Once);
        commands.entity(e).insert(MoveTo {
            dest: next.extend(BODY_CENTER).xzy(),
            speed: RUN_SPEED,
            strafe: true,
        });
    }
    RUNNING
}

fn investigate_listeners() -> Listeners {
    let mut l = insert_while_running(Activity::Investigating);
    l.push((
        TaskEvent::Exit,
        listener(|In(e): In<Entity>, mut commands: Commands| {
            commands.entity(e).try_remove::<MoveTo>();
        }),
    ));
    l
}

/// Look toward the noise. If cover blocks the view (say he's hiding behind a pillar), walk
/// toward it until he can see the spot, so he never stares at a wall forever.
fn investigate(
    In(e): In<Entity>,
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(&Transform, &Suspicion, &mut Alert, &mut LookGoal, Has<MoveTo>)>,
) -> TaskStatus {
    let Ok((t, suspicion, mut alert, mut goal, moving)) = q.get_mut(e) else {
        return FAILURE;
    };
    if suspicion.engaged {
        return FAILURE;
    }
    let (here, there) = (t.translation.xz(), alert.at.xz());
    if arena::los_blocked(here, there) && here.distance(there) > 3.0 {
        if !moving {
            // Head for a clear spot near the noise (it's often right up against cover).
            let dest = (0..=20)
                .map(|i| there.lerp(here, i as f32 / 20.0))
                .find(|&p| arena::is_clear(p, 0.8))
                .unwrap_or(here);
            commands.entity(e).insert(MoveTo {
                dest: dest.extend(BODY_CENTER).xzy(),
                speed: WALK_SPEED,
                strafe: false,
            });
        }
        return RUNNING;
    }
    if moving {
        commands.entity(e).remove::<MoveTo>();
    }
    // Only once he's stopped: while walking, `walk` steers his gaze along the route.
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
            commands.entity(e).insert(MoveTo {
                dest,
                speed: WALK_SPEED,
                strafe: false,
            });
        }),
    ));
    l.push((
        TaskEvent::Exit,
        listener(|In(e): In<Entity>, mut commands: Commands| {
            commands.entity(e).try_remove::<MoveTo>();
        }),
    ));
    l
}

fn wander(
    In(e): In<Entity>,
    mobile: Res<TargetMobile>,
    suspicion: Query<&Suspicion>,
    alerts: Query<(), With<Alert>>,
    q: Query<(&Transform, Option<&MoveTo>)>,
) -> TaskStatus {
    if interrupted(e, &suspicion, &alerts) {
        return FAILURE;
    }
    if !mobile.0 {
        return SUCCESS;
    }
    let Ok((t, Some(m))) = q.get(e) else {
        return RUNNING;
    };
    if t.translation.xz().distance(m.dest.xz()) < ARRIVE {
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
    if sees {
        suspicion.last_known = Some(shooter_t.translation);
    }

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
        // Mid-fight he stays keyed up for longer: hiding and switching cover shouldn't make him forget you.
        let decay = if suspicion.engaged { 0.05 } else { 0.12 };
        suspicion.level = (suspicion.level - decay * dt).max(0.0);
        if suspicion.level <= 0.0 && suspicion.engaged {
            suspicion.engaged = false;
            // Lost you: go check where you were last.
            if let Some(p) = suspicion.last_known {
                commands.entity(target_e).insert(Alert::new(p, 3.0));
            }
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

/// Follow the `Route` planned for `MoveTo`, looking where he's going; turn before moving.
fn walk(
    mut q: Query<
        (&Transform, &mut LinearVelocity, &mut LookGoal, Option<&MoveTo>, Option<&mut Route>),
        With<Target>,
    >,
) {
    for (t, mut v, mut look, m, route) in &mut q {
        let (Some(m), Some(mut route)) = (m, route) else {
            v.0 = Vec3::ZERO;
            continue;
        };
        let here = t.translation.xz();
        while route.0.len() > 1 && route.next().is_some_and(|w| w.xz().distance(here) < 0.35) {
            route.0.remove(0);
        }
        let w = route.next().unwrap_or(m.dest).xz();
        let d = w - here;
        if d.length() < 0.2 {
            v.0 = Vec3::ZERO;
            continue;
        }
        let d3 = Vec3::new(d.x, 0.0, d.y);
        if m.strafe {
            v.0 = d3.normalize() * m.speed;
            continue;
        }
        look.point = Vec3::new(w.x, t.translation.y + EYE_OFFSET, w.y);
        look.turn_speed = 7.0;
        v.0 = if t.forward().angle_between(d3) < 0.5 { d3.normalize() * m.speed } else { Vec3::ZERO };
    }
}

fn yaw_of(t: &Transform) -> f32 {
    t.rotation.to_euler(EulerRot::YXZ).0
}

/// Shortest signed angle from `a` to `b`.
fn angle_diff(a: f32, b: f32) -> f32 {
    (b - a + PI).rem_euclid(2.0 * PI) - PI
}
