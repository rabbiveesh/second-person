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
    round::{Attract, GameState, RoundEntity, SpawnRound},
    target::{MainCamera, Target},
};

pub const SHOOTER_MAX_HP: f32 = 100.0;
/// Minimum start distance from the target (who starts at the origin).
pub const MIN_START_DISTANCE: f32 = 12.0;
const RADIUS: f32 = 0.35;
const MOVE_SPEED: f32 = 5.0;
const TURN_SPEED: f32 = 2.4;

/// How the arrows (or the stick) map to movement. C cycles through them; it's a spike to feel
/// out which ones are playable, so they're all in the build.
#[derive(Resource, Reflect, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[reflect(Resource)]
pub enum ControlScheme {
    /// Up/down drive along your own facing, left/right turn.
    #[default]
    Tank,
    /// Up is north (the top of the radar), whatever anyone is facing. You turn to face where you go.
    Compass,
    /// Relative to what's on screen: up goes away from his eyes, right goes screen-right. When he
    /// turns his head, your directions turn with it.
    Eyes,
    /// `Eyes` turned round, as if you were the one facing the camera: up comes towards him, right is
    /// screen-left.
    Mirror,
    /// Up walks straight at him, left/right circle him (wherever he is, seen or not).
    Magnetic,
    /// You never stop walking: left/right steer, up hurries, down stands still.
    Snake,
    /// `Eyes` on a frozen pond: the arrows push, and you slide.
    Ice,
}

impl ControlScheme {
    pub const ALL: [Self; 7] =
        [Self::Tank, Self::Compass, Self::Eyes, Self::Mirror, Self::Magnetic, Self::Snake, Self::Ice];

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// The key hint for the arrows.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Tank => "Up/Down move   Left/Right turn (double-tap: hop)",
            Self::Compass => "Arrows walk north/south/east/west (radar up is north)",
            Self::Eyes => "Arrows walk as seen on screen",
            Self::Mirror => "Arrows walk mirrored: Up comes at him",
            Self::Magnetic => "Up walks at him   Left/Right circle him",
            Self::Snake => "Always walking   Left/Right steer   Up hurry   Down stop",
            Self::Ice => "Arrows push (screen-relative) and you slide",
        }
    }

    /// What your shots do relative to tank controls: the schemes that do more of the work for
    /// you (walking you at him, or a frame that never moves) pay for it in damage.
    pub fn damage_dealt(self) -> f32 {
        match self {
            Self::Tank | Self::Snake | Self::Mirror | Self::Ice => 1.0,
            Self::Eyes => 0.9,
            Self::Compass => 0.85,
            Self::Magnetic => 0.7,
        }
    }

    /// The tank-style schemes keep turning on left/right (and the double-tap hop).
    fn turns(self) -> bool {
        matches!(self, Self::Tank | Self::Snake)
    }

    /// For the direct schemes: the world directions (forward, right) that Up and Right push
    /// along, on the floor plane. `None` for the tank-style ones.
    pub fn basis(self, me: Vec3, him: Option<Vec3>, eyes: Option<&GlobalTransform>) -> Option<(Vec3, Vec3)> {
        let compass = (Vec3::NEG_Z, Vec3::X);
        // Right is forward turned a quarter clockwise, seen from above.
        let with_right = |f: Vec3| (f, Vec3::new(-f.z, 0.0, f.x));
        let screen = || {
            eyes.and_then(|e| Dir3::new(e.forward().with_y(0.0)).ok())
                .map_or(compass, |f| with_right(f.as_vec3()))
        };
        match self {
            Self::Tank | Self::Snake => None,
            Self::Compass => Some(compass),
            Self::Eyes | Self::Ice => Some(screen()),
            Self::Mirror => {
                let (f, r) = screen();
                Some((-f, -r))
            }
            Self::Magnetic => Some(
                him.and_then(|h| Dir3::new((h - me).with_y(0.0)).ok())
                    .map_or(compass, |f| with_right(f.as_vec3())),
            ),
        }
    }
}

/// How fast the direct schemes swing you round to face where you're walking (rad/s).
const FACE_TURN_SPEED: f32 = 10.0;
/// Snake: walking pace with nothing held, and how much Up adds.
const SNAKE_SPEED: f32 = 3.2;
const SNAKE_HURRY: f32 = 1.8;
/// Ice: push (m/s²) and how much of your speed the ice takes per second.
const ICE_PUSH: f32 = 7.0;
const ICE_DRAG: f32 = 0.35;
const ICE_MAX_SPEED: f32 = 7.5;

#[derive(Actionlike, PartialEq, Eq, Clone, Copy, Hash, Debug, Reflect)]
pub enum ShooterAction {
    /// x: turn (right positive), y: throttle (forward positive). Analog, so a touch stick or
    /// gamepad gives partial speed; arrow keys drive it as a d-pad.
    #[actionlike(DualAxis)]
    Drive,
    Fire,
    Whistle,
}

#[derive(Component, Reflect)]
#[reflect(Component)]
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
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Stagger {
    pub left: f32,
    pub push: Vec3,
}

/// Caught by his grappling hook: no walking, turning or firing until `left` runs out. While
/// `pull_to` is set you're being reeled toward that point.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Stunned {
    pub left: f32,
    pub pull_to: Option<Vec3>,
}

const PULL_SPEED: f32 = 14.0;

impl Stagger {
    /// Knocked back at `push` (m/s), easing off over the stagger.
    pub fn knock(push: Vec3) -> Self {
        Self { left: STAGGER_SECS, push }
    }
}

/// Double-tap left or right (arrows, or a double flick of the touch stick) to hop sideways
/// without turning.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Sidestep {
    /// The first tap of a possible double: its side (-1 left, +1 right), when, and the facing
    /// before it, since a tap on a turn key also turns you a little.
    first: Option<(f32, f32, Quat)>,
    /// Whether the stick is pushed left/right right now (to catch each fresh push).
    held: f32,
    /// Seconds of hop left, and its velocity.
    pub left: f32,
    pub velocity: Vec3,
    cooldown: f32,
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

pub fn plugin(app: &mut App) {
    app.add_plugins(InputManagerPlugin::<ShooterAction>::default())
        .add_message::<Footstep>()
        .add_message::<Bump>()
        .add_message::<Whistle>()
        .init_resource::<ControlScheme>()
        .register_type::<ControlScheme>()
        .add_systems(OnEnter(GameState::Playing), spawn_shooter.in_set(SpawnRound).run_if(|attract: Res<Attract>| !attract.0))
        // Movement runs on the fixed tick, so it's the same at any frame rate (and can be re-run).
        .add_systems(FixedUpdate, (drive, bump).chain().run_if(in_state(GameState::Playing)))
        .add_systems(Update, whistle.run_if(in_state(GameState::Playing)));
}

fn spawn_shooter(
    mut commands: Commands,
    layout: Res<Layout>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let blip = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 1.0, 0.3),
        unlit: true,
        ..default()
    });

    commands.spawn((
        Name::new("Shooter"),
        RoundEntity,
        Shooter { hp: SHOOTER_MAX_HP },
        (Stagger::default(), Stunned::default(), Sidestep::default()),
        InputMap::default()
            .with_dual_axis(ShooterAction::Drive, VirtualDPad::arrow_keys())
            .with(ShooterAction::Fire, KeyCode::Space)
            .with(ShooterAction::Whistle, KeyCode::KeyW),
        RigidBody::Dynamic,
        Collider::capsule(RADIUS, 1.0),
        LockedAxes::ROTATION_LOCKED,
        Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
        CollisionLayers::new(Layer::Shooter, [Layer::World, Layer::Target]),
        random_start(&mut rand::rng(), &layout),
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
    ));
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

/// Moves the shooter by the current `ControlScheme`. Stuns, hops and knock backs override it.
#[allow(clippy::too_many_arguments)]
fn drive(
    time: Res<Time>,
    scheme: Res<ControlScheme>,
    him: Query<&GlobalTransform, With<Target>>,
    eyes: Query<&GlobalTransform, With<MainCamera>>,
    mut step: Local<f32>,
    mut steps: MessageWriter<Footstep>,
    mut q: Query<
        (
            &ActionState<ShooterAction>,
            &Transform,
            &mut Rotation,
            &mut LinearVelocity,
            &mut Stagger,
            &mut Stunned,
            &mut Sidestep,
        ),
        With<Shooter>,
    >,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let scheme = *scheme;
    let him = him.single().ok().map(|t| t.translation());
    let eyes = eyes.single().ok();
    for (actions, t, mut rot, mut vel, mut stagger, mut stunned, mut side) in &mut q {
        let input = actions.clamped_axis_pair(&ShooterAction::Drive);
        side.cooldown -= dt;
        // A fresh push straight left or right (not a diagonal, which is driving while turning).
        let pushed = if scheme.turns() && input.x.abs() > 0.5 && input.y.abs() < 0.5 { input.x.signum() } else { 0.0 };
        let tap = (pushed != 0.0 && side.held != pushed).then_some(pushed);
        side.held = pushed;
        if stunned.left > 0.0 || stagger.left > 0.0 {
            side.first = None;
        } else if let Some(dir) = tap {
            match side.first {
                Some((first, at, facing)) if first == dir && now - at <= DOUBLE_TAP_WINDOW && side.cooldown <= 0.0 => {
                    // Undo the little turn the two taps made, then hop square to the side.
                    rot.0 = facing;
                    side.first = None;
                    side.left = HOP_SECS;
                    side.cooldown = HOP_COOLDOWN;
                    side.velocity = rot.0 * Vec3::X * dir * HOP_SPEED;
                }
                _ => side.first = Some((dir, now, rot.0)),
            }
        }

        if stunned.left > 0.0 {
            stunned.left -= time.delta_secs();
            let pull = stunned.pull_to.map_or(Vec3::ZERO, |p| (p - t.translation).with_y(0.0).normalize_or_zero() * PULL_SPEED);
            vel.0 = Vec3::new(pull.x, vel.0.y, pull.z);
            stagger.left = 0.0;
            side.left = 0.0;
            *step = 0.0;
            continue;
        }
        // A knock back (his laser, a wall) cuts a hop short.
        if stagger.left > 0.0 {
            side.left = 0.0;
        }
        if side.left > 0.0 {
            side.left -= dt;
            vel.0 = Vec3::new(side.velocity.x, vel.0.y, side.velocity.z);
            if side.left <= 0.0 {
                // Landing scuffs like a step, so he can hear it.
                steps.write(Footstep { at: t.translation });
            }
            *step = 0.0;
            continue;
        }
        if stagger.left > 0.0 {
            stagger.left -= time.delta_secs();
            let push = stagger.push * (stagger.left / STAGGER_SECS).max(0.0);
            vel.0 = Vec3::new(push.x, vel.0.y, push.z);
            if scheme.turns() {
                rot.0 = Quat::from_rotation_y(-input.x * TURN_SPEED * dt) * rot.0;
            }
            *step = 0.0;
            continue;
        }

        let planar = match (scheme, scheme.basis(t.translation, him, eyes)) {
            (ControlScheme::Ice, Some((f, r))) => {
                let push = (f * input.y + r * input.x).clamp_length_max(1.0) * ICE_PUSH * dt;
                let v = (vel.0.with_y(0.0) * (1.0 - ICE_DRAG * dt) + push).clamp_length_max(ICE_MAX_SPEED);
                face(&mut rot, v, v.length() / ICE_MAX_SPEED, dt);
                v
            }
            (_, Some((f, r))) => {
                let want = (f * input.y + r * input.x).clamp_length_max(1.0);
                face(&mut rot, want, want.length(), dt);
                want * MOVE_SPEED
            }
            (ControlScheme::Snake, None) => {
                rot.0 = Quat::from_rotation_y(-input.x * TURN_SPEED * dt) * rot.0;
                let speed = if input.y < -0.5 { 0.0 } else { SNAKE_SPEED + SNAKE_HURRY * input.y.max(0.0) };
                rot.0 * Vec3::NEG_Z * speed
            }
            _ => {
                rot.0 = Quat::from_rotation_y(-input.x * TURN_SPEED * dt) * rot.0;
                rot.0 * Vec3::NEG_Z * input.y * MOVE_SPEED
            }
        };
        vel.0 = Vec3::new(planar.x, vel.0.y, planar.z);

        if planar.length() > 0.5 {
            *step -= time.delta_secs();
            if *step <= 0.0 {
                *step = STEP_INTERVAL;
                steps.write(Footstep { at: t.translation });
            }
        } else {
            *step = 0.0;
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
    mut bumps: MessageWriter<Bump>,
    mut q: Query<(&Transform, &mut LinearVelocity, &mut Stagger, &Stunned, &mut Sidestep), With<Shooter>>,
) {
    for (t, mut vel, mut stagger, stunned, mut side) in &mut q {
        if stagger.left > 0.0 || stunned.left > 0.0 {
            continue;
        }
        let Ok(dir) = Dir3::new(Vec3::new(vel.0.x, 0.0, vel.0.z)) else { continue };
        let Some(hit) = spatial.cast_ray(
            t.translation,
            dir,
            RADIUS + BUMP_REACH,
            true,
            &SpatialQueryFilter::from_mask(Layer::World),
        ) else {
            continue;
        };
        let normal = Vec3::new(hit.normal.x, 0.0, hit.normal.z).normalize_or(-dir.as_vec3());
        bumps.write(Bump { at: t.translation + dir * hit.distance });
        side.left = 0.0;
        *stagger = Stagger::knock(normal * KNOCKBACK_SPEED);
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
