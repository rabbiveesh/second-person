//! His scramble dart. While fighting and seeing you at mid range he winds up (a glow and a
//! whine you can read), then throws a slow dart that scrambles your controls for a few seconds
//! (`shooter::Scrambled`). It flies slowly enough to sidestep and cover stops it. Every knob comes
//! from the Gunfight band (`difficulty::ScrambleLevers`), the wind-up also from the assist dial.
//!
//! The dart is moved by hand, not by the physics engine: it's a slow, fat projectile, and a swept
//! segment test against the shooter and the layout is exact and deterministic.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    arena::Layout,
    combat::{Grapple, check_outcome},
    difficulty::Tuning,
    round::{GameState, RoundEntity},
    shooter::{ControlScheme, Scrambled, Shooter, Stagger, Stunned},
    target::{Activity, MainCamera, Roll, Suspicion, Target},
};

/// He throws it from this far...
pub const DART_RANGE: std::ops::Range<f32> = 6.0..22.0;
/// ...and it's gone after this long.
const DART_LIFETIME: f32 = 3.0;
/// Hit radius: the dart's own plus the shooter's capsule.
const DART_HIT_RADIUS: f32 = 0.35 + 0.25;
/// The dart appears this far in front of him.
const DART_LAUNCH_AHEAD: f32 = 0.8;
/// No dart for this long after a round starts, so the opening isn't decided by one.
const FIRST_DART_AFTER: f32 = 4.0;

/// Winding up a dart, on the target: `left` seconds until it flies.
#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
pub struct Charging {
    pub left: f32,
}

/// A dart in flight.
#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
pub struct Dart {
    pub velocity: Vec3,
    pub left: f32,
}

/// He started winding up (for the telegraph: glow, whine, HUD).
#[derive(Message, Clone, Copy, Debug)]
pub struct DartCharging {
    pub windup: f32,
}

/// The dart left his hand.
#[derive(Message, Clone, Copy, Debug)]
pub struct DartThrown {
    pub from: Vec3,
    pub velocity: Vec3,
}

/// The dart hit you: your controls are `into` for `secs`.
#[derive(Message, Clone, Copy, Debug)]
pub struct ScrambledHit {
    pub at: Vec3,
    pub into: ControlScheme,
    pub secs: f32,
}

/// The dart missed (hit a wall or ran out).
#[derive(Message, Clone, Copy, Debug)]
pub struct DartMissed {
    pub at: Vec3,
}

#[derive(Resource)]
struct DartLook {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

/// Whether he carries the dart at all (on by default; off for A/B runs of the simulator).
#[derive(Resource, Reflect, Clone, Copy, Debug)]
#[reflect(Resource)]
pub struct Armed(pub bool);

impl Default for Armed {
    fn default() -> Self {
        Self(true)
    }
}

/// Seconds until he may wind up again; reset each round.
#[derive(Resource, Default)]
struct Cooldown(f32);

pub fn plugin(app: &mut App) {
    app.add_message::<DartCharging>()
        .add_message::<DartThrown>()
        .add_message::<ScrambledHit>()
        .add_message::<DartMissed>()
        .init_resource::<Cooldown>()
        .init_resource::<Armed>()
        .add_systems(Startup, load)
        .add_systems(OnEnter(GameState::Playing), |mut c: ResMut<Cooldown>| c.0 = FIRST_DART_AFTER)
        .add_systems(Update, (charge, fly).chain().before(check_outcome).run_if(in_state(GameState::Playing)));
}

fn load(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(DartLook {
        mesh: meshes.add(Sphere::new(0.35)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.85, 0.55, 1.0),
            emissive: LinearRgba::rgb(10.0, 4.0, 20.0),
            ..default()
        }),
    });
}

/// Where to throw so a dart at `speed` meets you, leading your velocity by `lead` (0..1).
pub fn aim(from: Vec3, at: Vec3, velocity: Vec3, speed: f32, lead: f32) -> Vec3 {
    let mut point = at;
    // Two refinements are plenty at these speeds.
    for _ in 0..2 {
        let t = from.xz().distance(point.xz()) / speed.max(0.1);
        point = at + velocity.with_y(0.0) * t * lead;
    }
    point
}

#[allow(clippy::too_many_arguments)]
fn charge(
    mut commands: Commands,
    time: Res<Time>,
    mut cooldown: ResMut<Cooldown>,
    tuning: Res<Tuning>,
    layout: Res<Layout>,
    armed: Res<Armed>,
    look: Res<DartLook>,
    eyes: Single<&GlobalTransform, With<MainCamera>>,
    target: Single<
        (Entity, &Transform, &Suspicion, Option<&Activity>, Option<&mut Charging>, Has<Grapple>, Has<Roll>),
        With<Target>,
    >,
    shooter: Single<(&Transform, &LinearVelocity, &Stunned, &Stagger, &Scrambled), With<Shooter>>,
    darts: Query<(), With<Dart>>,
    mut charging_msgs: MessageWriter<DartCharging>,
    mut thrown: MessageWriter<DartThrown>,
) {
    let dt = time.delta_secs();
    cooldown.0 -= dt;
    let Some(levers) = tuning.him().scramble.filter(|_| armed.0) else {
        return;
    };
    let (e, t, suspicion, activity, charging, grappling, rolling) = target.into_inner();
    let (you, vel, stunned, stagger, scrambled) = *shooter;
    let dist = t.translation.xz().distance(you.translation.xz());

    let Some(mut charging) = charging else {
        // Never on top of another of his moves, or of one already holding you.
        let busy = grappling || rolling || stunned.left > 0.0 || stagger.left > 0.0 || scrambled.active();
        if cooldown.0 <= 0.0
            && !busy
            && darts.is_empty()
            && activity == Some(&Activity::Engaging)
            && suspicion.sees_shooter
            && DART_RANGE.contains(&dist)
        {
            let windup = levers.windup + tuning.help().scramble_windup;
            commands.entity(e).insert(Charging { left: windup });
            charging_msgs.write(DartCharging { windup });
        }
        return;
    };
    // A grapple or a roll mid wind-up spoils the throw.
    if grappling || rolling {
        commands.entity(e).remove::<Charging>();
        cooldown.0 = levers.cooldown * 0.5;
        return;
    }
    charging.left -= dt;
    if charging.left > 0.0 {
        return;
    }
    commands.entity(e).remove::<Charging>();
    cooldown.0 = levers.cooldown;
    // Out of sight at the last moment he throws at where he last saw you.
    let at = if suspicion.sees_shooter { you.translation } else { suspicion.last_known.unwrap_or(you.translation) };
    let lead = if suspicion.sees_shooter { levers.lead } else { 0.0 };
    // From his eyes, not his hand: he can see you, so the dart can't clip the corner he peeks round.
    let from = eyes.translation();
    let mut point = aim(from, at, vel.0, levers.speed, lead);
    // Never lead you into a wall: throw straight at you instead.
    if layout.los_blocked(from.xz(), point.xz()) {
        point = at;
    }
    let velocity = (point - from).with_y(0.0).normalize_or_zero() * levers.speed;
    commands.spawn((
        Name::new("Scramble dart"),
        RoundEntity,
        Dart { velocity, left: DART_LIFETIME },
        Mesh3d(look.mesh.clone()),
        MeshMaterial3d(look.material.clone()),
        // A step out from his eyes, so it isn't a blob filling his view on the first frame.
        Transform::from_translation((from + velocity.normalize_or_zero() * DART_LAUNCH_AHEAD).with_y(you.translation.y)),
    ));
    thrown.write(DartThrown { from, velocity });
}

#[allow(clippy::too_many_arguments)]
fn fly(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<Tuning>,
    layout: Res<Layout>,
    mut darts: Query<(Entity, &mut Dart, &mut Transform), Without<Shooter>>,
    shooter: Single<(&Transform, &mut Scrambled, &Stunned), With<Shooter>>,
    mut hits: MessageWriter<ScrambledHit>,
    mut misses: MessageWriter<DartMissed>,
) {
    let dt = time.delta_secs();
    let (you, mut scrambled, stunned) = shooter.into_inner();
    for (e, mut dart, mut t) in &mut darts {
        let from = t.translation;
        let to = from + dart.velocity * dt;
        dart.left -= dt;
        // Closest approach to you along this frame's path.
        let seg = (to - from).xz();
        let rel = you.translation.xz() - from.xz();
        let s = (rel.dot(seg) / seg.length_squared().max(1e-6)).clamp(0.0, 1.0);
        let closest = from.xz() + seg * s;
        if closest.distance(you.translation.xz()) <= DART_HIT_RADIUS && !layout.los_blocked(from.xz(), closest) {
            commands.entity(e).despawn();
            // Grappled mid-flight: the hook wins, the dart's wasted.
            if stunned.left > 0.0 {
                misses.write(DartMissed { at: you.translation });
                continue;
            }
            let Some(levers) = tuning.him().scramble else {
                continue;
            };
            let into = ControlScheme::Mirror;
            *scrambled = Scrambled { left: levers.secs, into };
            hits.write(ScrambledHit { at: you.translation, into, secs: levers.secs });
            continue;
        }
        if layout.los_blocked(from.xz(), to.xz()) || dart.left <= 0.0 {
            commands.entity(e).despawn();
            misses.write(DartMissed { at: to });
            continue;
        }
        t.translation = to;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_aims_where_you_will_be() {
        let from = Vec3::ZERO;
        let at = Vec3::new(0.0, 0.0, -10.0);
        let v = Vec3::new(5.0, 0.0, 0.0);
        assert_eq!(aim(from, at, v, 10.0, 0.0), at);
        let led = aim(from, at, v, 10.0, 1.0);
        // About a second of flight, so about 5m ahead of you.
        assert!(led.x > 4.0 && led.x < 6.0, "{led}");
    }
}
