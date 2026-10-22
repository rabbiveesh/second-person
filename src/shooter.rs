//! The shooter: the body you control, seen (mostly) through the target's eyes.

use std::f32::consts::TAU;

use avian3d::prelude::*;
use bevy::{camera::visibility::RenderLayers, prelude::*};
use leafwing_input_manager::prelude::*;
use rand::Rng;

use crate::{
    Layer,
    arena::Layout,
    radar::{LiveBlip, RADAR_LAYER, RadarContact},
    round::{Attract, GameState, Online, RoundEntity, SpawnRound},
    target::MainCamera,
};

pub const SHOOTER_MAX_HP: f32 = 100.0;
/// Minimum start distance from the target (who starts at the origin).
pub const MIN_START_DISTANCE: f32 = 12.0;
const RADIUS: f32 = 0.35;
const MOVE_SPEED: f32 = 5.0;
const TURN_SPEED: f32 = 2.4;

/// How the arrows (or the stick) map to movement. You always drive `Tank`; his scramble dart
/// flips you into `Mirror` for a while.
#[derive(Resource, Reflect, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[reflect(Resource)]
pub enum ControlScheme {
    /// Up/down drive along your own facing, left/right turn.
    #[default]
    Tank,
    /// Relative to what's on screen, turned round, as if you were the one facing the camera: up
    /// comes towards him, right is screen-left. When he turns his head, your directions turn with it.
    Mirror,
}

impl ControlScheme {
    pub const ALL: [Self; 2] = [Self::Tank, Self::Mirror];

    /// The key hint for the arrows.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Tank => "Up/Down move   Left/Right turn (double-tap: hop)",
            Self::Mirror => "Arrows walk mirrored: Up comes at him",
        }
    }

    /// Tank keeps turning on left/right (and the double-tap hop).
    fn turns(self) -> bool {
        self == Self::Tank
    }

    /// For `Mirror`: the world directions (forward, right) that Up and Right push along, on the
    /// floor plane, from his view. `None` for `Tank`.
    pub fn basis(self, eyes: Option<&GlobalTransform>) -> Option<(Vec3, Vec3)> {
        match self {
            Self::Tank => None,
            Self::Mirror => {
                let f = eyes.and_then(|e| Dir3::new(e.forward().with_y(0.0)).ok()).map_or(Vec3::NEG_Z, |f| f.as_vec3());
                // Screen-right is forward turned a quarter clockwise, seen from above; Mirror flips both.
                Some((-f, Vec3::new(f.z, 0.0, -f.x)))
            }
        }
    }
}

/// His scramble dart hit you: until `left` runs out you drive with `into` instead of your own
/// `ControlScheme` (see `scramble`).
#[derive(Component, Reflect, Default, Clone, Copy, PartialEq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[reflect(Component)]
pub struct Scrambled {
    pub left: f32,
    pub into: ControlScheme,
}

impl Scrambled {
    pub fn active(&self) -> bool {
        self.left > 0.0
    }

    /// The scheme you're driving with right now.
    pub fn scheme(&self, chosen: ControlScheme) -> ControlScheme {
        if self.active() { self.into } else { chosen }
    }
}

/// How fast the direct schemes swing you round to face where you're walking (rad/s).
const FACE_TURN_SPEED: f32 = 10.0;

#[derive(Actionlike, PartialEq, Eq, Clone, Copy, Hash, Debug, Reflect)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum ShooterAction {
    /// x: turn (right positive), y: throttle (forward positive). Analog, so a touch stick or
    /// gamepad gives partial speed; arrow keys drive it as a d-pad.
    #[actionlike(DualAxis)]
    Drive,
    Fire,
    Whistle,
    /// Duel only: shut your eyes, which blinds the other player (they see through them).
    CloseEyes,
}

#[derive(Component, Reflect)]
#[reflect(Component)]
#[require(Stride)]
pub struct Shooter {
    pub hp: f32,
}

/// The shooter took a step (heard by the target).
#[derive(Message, Clone, Copy, Debug)]
pub struct Footstep {
    pub at: Vec3,
}

/// The shooter walked into a wall or cover: a thud (so you notice you're stuck) and a knock back.
#[derive(Message, Clone, Copy, Debug)]
pub struct Bump {
    pub at: Vec3,
}

/// You whistled: a loud, far-carrying sound so you can find yourself by ear. He can hear it too.
#[derive(Message, Clone, Copy, Debug)]
pub struct Whistle {
    pub at: Vec3,
}

const WHISTLE_COOLDOWN: f32 = 2.0;

/// While > 0, the shooter is being knocked back by `push` and can't walk (only turn).
#[derive(Component, Reflect, Default, Clone, PartialEq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[reflect(Component)]
pub struct Stagger {
    pub left: f32,
    pub push: Vec3,
    /// How many times you've walked into something. Counted rather than flagged, so `make_noise`
    /// thuds once per bump even when the movement is re-run.
    pub bumps: u32,
}

/// Caught by his grappling hook: no walking, turning or firing until `left` runs out. While
/// `pull_to` is set you're being reeled toward that point.
#[derive(Component, Reflect, Default, Clone, PartialEq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[reflect(Component)]
pub struct Stunned {
    pub left: f32,
    pub pull_to: Option<Vec3>,
}

const PULL_SPEED: f32 = 14.0;

impl Stagger {
    /// Knocked back at `push` (m/s), easing off over the stagger.
    pub fn knock(self, push: Vec3) -> Self {
        Self { left: STAGGER_SECS, push, ..self }
    }
}

/// Double-tap left or right (arrows, or a double flick of the touch stick) to hop sideways
/// without turning.
#[derive(Component, Reflect, Default, Clone, PartialEq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[reflect(Component)]
pub struct Sidestep {
    /// The first tap of a possible double: its side (-1 left, +1 right), how long ago, and the
    /// facing before it, since a tap on a turn key also turns you a little.
    first: Option<(f32, f32, Quat)>,
    /// Whether the stick is pushed left/right right now (to catch each fresh push).
    held: f32,
    /// Seconds of hop left, and its velocity.
    pub left: f32,
    pub velocity: Vec3,
    cooldown: f32,
}

impl Sidestep {
    /// Mid-hop, or landed too recently to hop again.
    pub fn spent(&self) -> bool {
        self.left > 0.0 || self.cooldown > 0.0
    }
}

/// Both taps have to land within this.
const DOUBLE_TAP_WINDOW: f32 = 0.3;
const HOP_SPEED: f32 = 12.0;
const HOP_SECS: f32 = 0.2;
const HOP_COOLDOWN: f32 = 0.8;

const STEP_INTERVAL: f32 = 0.42;
const BUMP_REACH: f32 = 0.15;
const KNOCKBACK_SPEED: f32 = 4.0;
const STAGGER_SECS: f32 = 0.25;

/// The shooter's movement: a pure function of its inputs and movement state, run on the fixed
/// tick, so it can be re-run (a duel client replays it to predict its own body).
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Movement;

/// Turns movement into sounds (`Footstep`, `Bump`), after `Movement`.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Noise;

/// What `Mirror` steers by: his eyes. Taken before each tick's movement, so `drive` reads
/// nothing but the shooter itself.
#[derive(Resource, Default, Clone, Copy)]
pub struct Bearings {
    pub eyes: Option<GlobalTransform>,
}

pub fn plugin(app: &mut App) {
    // A duel server brings its own (server-mode) input plugin.
    if !app.is_plugin_added::<InputManagerPlugin<ShooterAction>>() {
        app.add_plugins(InputManagerPlugin::<ShooterAction>::default());
    }
    // There's no him to take bearings on in a duel, so its movement never reads the world.
    if !app.world().contains_resource::<Online>() {
        app.add_systems(FixedUpdate, take_bearings.before(Movement));
    }
    app.init_resource::<Bearings>()
        .add_message::<Footstep>()
        .add_message::<Bump>()
        .add_message::<Whistle>()
        .init_resource::<ControlScheme>()
        .register_type::<ControlScheme>()
        .add_systems(OnEnter(GameState::Playing), spawn_shooter.in_set(SpawnRound).run_if(|attract: Res<Attract>| !attract.0).run_if(not(resource_exists::<Online>)))
        // Movement runs on the fixed tick, so it's the same at any frame rate (and can be re-run).
        .add_systems(
            FixedUpdate,
            (
                (drive, bump).chain().in_set(Movement),
                make_noise.in_set(Noise).after(Movement),
            )
                .run_if(in_state(GameState::Playing)),
        )
        .add_systems(Update, whistle.run_if(in_state(GameState::Playing)));
}

fn spawn_shooter(
    mut commands: Commands,
    layout: Res<Layout>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Name::new("Shooter"),
        RoundEntity,
        body(random_start(&mut rand::rng(), &layout)),
        controls(),
        looks(&mut meshes, &mut materials),
    ));
}

/// What the simulation needs of a shooter: health, movement state and the physics body.
/// A duel server spawns one per player.
pub fn body(at: Transform) -> impl Bundle {
    (
        Shooter { hp: SHOOTER_MAX_HP },
        (Stagger::default(), Stunned::default(), Sidestep::default(), Scrambled::default()),
        physics(),
        at,
    )
}

/// The rigid body alone: a duel client adds this to the body it predicts.
pub fn physics() -> impl Bundle {
    (
        RigidBody::Dynamic,
        Collider::capsule(RADIUS, 1.0),
        LockedAxes::ROTATION_LOCKED,
        Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
        CollisionLayers::new(Layer::Shooter, [Layer::World, Layer::Target]),
    )
}

/// The keyboard bindings (touch writes the same `ActionState`).
pub fn controls() -> InputMap<ShooterAction> {
    InputMap::default()
        .with_dual_axis(ShooterAction::Drive, VirtualDPad::arrow_keys())
        .with(ShooterAction::Fire, KeyCode::Space)
        .with(ShooterAction::Whistle, KeyCode::KeyW)
}

/// How a shooter looks: orange capsule, visor, gun, and its radar marks.
pub fn looks(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> impl Bundle {
    let blip = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 1.0, 0.3),
        unlit: true,
        ..default()
    });

    (
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
    )
}

/// A random start: clear of cover, away from the target, facing anywhere.
pub fn random_start(rng: &mut impl Rng, layout: &Layout) -> Transform {
    // Room to turn and fire without the bullet hitting a wall straight away.
    let clear = RADIUS + 1.2;
    let p = layout
        .random_point(rng, clear, |p| p.length() >= MIN_START_DISTANCE)
        // A cramped arena: anywhere clear will do.
        .or_else(|| layout.random_point(rng, clear, |_| true))
        .unwrap_or(Vec2::new(0.0, MIN_START_DISTANCE));
    Transform::from_xyz(p.x, 0.9, p.y).with_rotation(Quat::from_rotation_y(rng.random_range(0.0..TAU)))
}

fn take_bearings(mut bearings: ResMut<Bearings>, eyes: Query<&GlobalTransform, With<MainCamera>>) {
    *bearings = Bearings { eyes: eyes.single().ok().copied() };
}

/// Moves the shooter by the current `ControlScheme` (or the one a dart scrambled you into).
/// Stuns, hops and knock backs override it.
#[allow(clippy::type_complexity)]
fn drive(
    time: Res<Time>,
    scheme: Res<ControlScheme>,
    bearings: Res<Bearings>,
    mut q: Query<
        (
            &ActionState<ShooterAction>,
            &Position,
            &mut Rotation,
            &mut LinearVelocity,
            &mut Stagger,
            &mut Stunned,
            &mut Sidestep,
            &mut Scrambled,
        ),
        With<Shooter>,
    >,
) {
    let dt = time.delta_secs();
    let chosen = *scheme;
    for (actions, at, mut rot, mut vel, mut stagger, mut stunned, mut side, mut scrambled) in &mut q {
        let scheme = scrambled.scheme(chosen);
        scrambled.left = (scrambled.left - dt).max(0.0);
        let input = actions.clamped_axis_pair(&ShooterAction::Drive);
        side.cooldown -= dt;
        if let Some((_, age, _)) = &mut side.first {
            *age += dt;
        }
        // A fresh push straight left or right (not a diagonal, which is driving while turning).
        let pushed = if scheme.turns() && input.x.abs() > 0.5 && input.y.abs() < 0.5 { input.x.signum() } else { 0.0 };
        let tap = (pushed != 0.0 && side.held != pushed).then_some(pushed);
        side.held = pushed;
        if stunned.left > 0.0 || stagger.left > 0.0 {
            side.first = None;
        } else if let Some(dir) = tap {
            match side.first {
                Some((first, age, facing)) if first == dir && age <= DOUBLE_TAP_WINDOW && side.cooldown <= 0.0 => {
                    // Undo the little turn the two taps made, then hop square to the side.
                    rot.0 = facing;
                    side.first = None;
                    side.left = HOP_SECS;
                    side.cooldown = HOP_COOLDOWN;
                    side.velocity = rot.0 * Vec3::X * dir * HOP_SPEED;
                }
                _ => side.first = Some((dir, 0.0, rot.0)),
            }
        }

        if stunned.left > 0.0 {
            stunned.left -= dt;
            let pull = stunned.pull_to.map_or(Vec3::ZERO, |p| (p - at.0).with_y(0.0).normalize_or_zero() * PULL_SPEED);
            vel.0 = Vec3::new(pull.x, vel.0.y, pull.z);
            stagger.left = 0.0;
            side.left = 0.0;
            continue;
        }
        // A knock back (his laser, a wall) cuts a hop short.
        if stagger.left > 0.0 {
            side.left = 0.0;
        }
        if side.left > 0.0 {
            side.left -= dt;
            vel.0 = Vec3::new(side.velocity.x, vel.0.y, side.velocity.z);
            continue;
        }
        if stagger.left > 0.0 {
            stagger.left -= dt;
            let push = stagger.push * (stagger.left / STAGGER_SECS).max(0.0);
            vel.0 = Vec3::new(push.x, vel.0.y, push.z);
            if scheme.turns() {
                rot.0 = Quat::from_rotation_y(-input.x * TURN_SPEED * dt) * rot.0;
            }
            continue;
        }

        let planar = match scheme.basis(bearings.eyes.as_ref()) {
            Some((f, r)) => {
                let want = (f * input.y + r * input.x).clamp_length_max(1.0);
                face(&mut rot, want, want.length(), dt);
                want * MOVE_SPEED
            }
            None => {
                rot.0 = Quat::from_rotation_y(-input.x * TURN_SPEED * dt) * rot.0;
                rot.0 * Vec3::NEG_Z * input.y * MOVE_SPEED
            }
        };
        vel.0 = Vec3::new(planar.x, vel.0.y, planar.z);
    }
}

/// Footstep timing and what's already been heard. Not simulation state: re-running the
/// movement never touches it, so nothing sounds twice.
#[derive(Component, Default)]
pub struct Stride {
    next: f32,
    hopping: bool,
    bumps: u32,
}

/// Steps while walking, a scuff on landing a hop, a thud on each bump.
fn make_noise(
    time: Res<Time>,
    mut steps: MessageWriter<Footstep>,
    mut bumps: MessageWriter<Bump>,
    mut q: Query<(&Position, &LinearVelocity, &Stagger, &Stunned, &Sidestep, &mut Stride)>,
) {
    for (at, vel, stagger, stunned, side, mut stride) in &mut q {
        if stagger.bumps > stride.bumps {
            // The knock back points away from what you walked into.
            bumps.write(Bump { at: at.0 - stagger.push.normalize_or_zero() * RADIUS });
        }
        stride.bumps = stagger.bumps;
        let hopping = side.left > 0.0;
        if stride.hopping && !hopping && stagger.left <= 0.0 && stunned.left <= 0.0 {
            steps.write(Footstep { at: at.0 });
        }
        stride.hopping = hopping;
        if hopping || stagger.left > 0.0 || stunned.left > 0.0 || vel.0.with_y(0.0).length() <= 0.5 {
            stride.next = 0.0;
            continue;
        }
        stride.next -= time.delta_secs();
        if stride.next <= 0.0 {
            stride.next = STEP_INTERVAL;
            steps.write(Footstep { at: at.0 });
        }
    }
}

/// Swing round to face along `dir` (on the floor plane), quicker the harder you push.
fn face(rot: &mut Rotation, dir: Vec3, push: f32, dt: f32) {
    if push < 0.1 {
        return;
    }
    let Ok(dir) = Dir3::new(dir.with_y(0.0)) else { return };
    let goal = Quat::from_rotation_y(f32::atan2(-dir.x, -dir.z));
    let angle = rot.0.angle_between(goal);
    if angle > 1e-4 {
        rot.0 = rot.0.slerp(goal, (FACE_TURN_SPEED * push.min(1.0) * dt / angle).min(1.0));
    }
}

/// Walking into something solid: thud and get knocked back, so you notice you're stuck.
fn bump(
    spatial: SpatialQuery,
    mut q: Query<(&Position, &mut LinearVelocity, &mut Stagger, &Stunned, &mut Sidestep), With<Shooter>>,
) {
    for (at, mut vel, mut stagger, stunned, mut side) in &mut q {
        if stagger.left > 0.0 || stunned.left > 0.0 {
            continue;
        }
        let Ok(dir) = Dir3::new(Vec3::new(vel.0.x, 0.0, vel.0.z)) else { continue };
        let Some(hit) = spatial.cast_ray(
            at.0,
            dir,
            RADIUS + BUMP_REACH,
            true,
            &SpatialQueryFilter::from_mask(Layer::World),
        ) else {
            continue;
        };
        let normal = Vec3::new(hit.normal.x, 0.0, hit.normal.z).normalize_or(-dir.as_vec3());
        side.left = 0.0;
        *stagger = Stagger { bumps: stagger.bumps + 1, ..stagger.clone().knock(normal * KNOCKBACK_SPEED) };
        vel.0 = Vec3::new(stagger.push.x, vel.0.y, stagger.push.z);
    }
}

fn whistle(
    time: Res<Time>,
    mut cooldown: Local<f32>,
    mut whistles: MessageWriter<Whistle>,
    q: Query<(&ActionState<ShooterAction>, &Transform), With<Shooter>>,
) {
    *cooldown -= time.delta_secs();
    for (actions, t) in &q {
        if actions.just_pressed(&ShooterAction::Whistle) && *cooldown <= 0.0 {
            *cooldown = WHISTLE_COOLDOWN;
            whistles.write(Whistle { at: t.translation });
        }
    }
}
