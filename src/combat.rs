//! Bullets, hits, hearing, and the target shooting back.

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;
use rand::Rng;

use crate::{
    Layer,
    arena,
    round::{GameState, RoundEntity},
    nav::MoveTo,
    shooter::{Bump, Footstep, Whistle, Shooter, ShooterAction, Stagger, Stunned},
    target::{Activity, Alert, MainCamera, Relocate, Suspicion, Target},
};

const BULLET_SPEED: f32 = 45.0;
const FIRE_COOLDOWN: f32 = 0.3;
const HEARING_RANGE: f32 = 18.0;
const NEAR_MISS_RANGE: f32 = 7.0;
const RETURN_FIRE_INTERVAL: f32 = 0.8;
const RETURN_FIRE_DAMAGE: f32 = 8.0;
/// His laser shoves you back, and you can't walk until it wears off.
const RETURN_FIRE_KNOCKBACK: f32 = 5.0;
/// Suspicion per footstep at point-blank range (fading to 0 at the floor's hearing range).
/// Steps come ~2.4 times a second, so walking right behind him on metal fills it in ~3s.
const FOOTSTEP_SUSPICION: f32 = 0.15;
const BUMP_HEARING_RANGE: f32 = 14.0;
const BUMP_SUSPICION: f32 = 0.2;
/// The whistle is your "where am I?" button, and the price is that he hears it from far off.
const WHISTLE_HEARING_RANGE: f32 = 28.0;
const WHISTLE_SUSPICION: f32 = 0.4;
/// Suspicious but not yet engaged: he fires warning shots that land this far off where he
/// thinks you are, every so often.
const WARNING_SUSPICION: f32 = 0.35;
const WARNING_MISS: std::ops::Range<f32> = 1.5..3.5;
const WARNING_INTERVAL: std::ops::Range<f32> = 1.2..2.8;
/// Grappling hook: used while fighting, on a shooter he can see within this range.
const GRAPPLE_RANGE: std::ops::Range<f32> = 6.0..20.0;
const GRAPPLE_COOLDOWN: f32 = 14.0;
/// Stop reeling at this distance (or after `GRAPPLE_MAX_PULL` seconds, if you snag on something).
const GRAPPLE_CLOSE: f32 = 2.5;
const GRAPPLE_MAX_PULL: f32 = 2.0;
const HAMMER_SHOTS: u32 = 3;
const HAMMER_INTERVAL: f32 = 0.18;
const HAMMER_DAMAGE: f32 = 7.0;
const GRAPPLE_STUN: f32 = 1.6;

#[derive(Component)]
pub struct Bullet {
    /// Where it was fired from: the target looks back here if it lands nearby.
    origin: Vec3,
}

/// The shooter fired. Drives muzzle flash, gunshot sound and radar ping.
#[derive(Message, Clone, Copy)]
pub struct Gunshot {
    pub muzzle: Vec3,
    pub dir: Vec3,
}

/// A bullet hit world geometry.
#[derive(Message, Clone, Copy)]
pub struct BulletImpact {
    pub at: Vec3,
}

/// The target got shot.
#[derive(Message, Clone, Copy)]
pub struct TargetHit {
    pub at: Vec3,
}

/// The target fired a deliberate miss near where he thinks you are: a warning, no damage.
#[derive(Message, Clone, Copy)]
pub struct WarningShot {
    pub from: Vec3,
    pub to: Vec3,
}

/// He fired his grappling hook at you (from his hand, to you).
#[derive(Message, Clone, Copy)]
pub struct GrappleFired {
    pub from: Vec3,
    pub to: Vec3,
}

/// His grappling hook move, on the target while it runs: reel you in, hammer you with
/// shots, leave you stunned, then run for other cover.
#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
pub enum Grapple {
    Pulling { time: f32 },
    Hammering { shots_left: u32, next: f32 },
}

/// The target shot the shooter (hitscan from `from` to `to`).
#[derive(Message, Clone, Copy)]
pub struct ShooterHit {
    pub from: Vec3,
    pub to: Vec3,
}

#[derive(Resource)]
struct Assets3d {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}


pub fn plugin(app: &mut App) {
    app.add_message::<Gunshot>()
        .add_message::<BulletImpact>()
        .add_message::<TargetHit>()
        .add_message::<ShooterHit>()
        .add_message::<WarningShot>()
        .add_message::<GrappleFired>()
        .add_systems(Startup, load_assets)
        .add_systems(
            Update,
            (fire, bullet_hits, hear_movement, warning_fire, grapple, return_fire, check_outcome)
                .chain()
                .run_if(in_state(GameState::Playing)),
        );
}

fn load_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(Assets3d {
        mesh: meshes.add(Sphere::new(0.12)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.9, 0.3),
            emissive: LinearRgba::rgb(8.0, 6.0, 1.0),
            ..default()
        }),
    });
}

fn fire(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<Assets3d>,
    mut cooldown: Local<Option<Timer>>,
    shooter: Single<(&ActionState<ShooterAction>, &Transform, &Stunned), With<Shooter>>,
    target: Single<(Entity, &Transform, &mut Suspicion), With<Target>>,
    mut gunshots: MessageWriter<Gunshot>,
) {
    let cooldown = cooldown.get_or_insert_with(|| {
        let mut t = Timer::from_seconds(FIRE_COOLDOWN, TimerMode::Once);
        t.finish();
        t
    });
    cooldown.tick(time.delta());

    let (actions, t, stunned) = *shooter;
    if !actions.just_pressed(&ShooterAction::Fire) || !cooldown.is_finished() || stunned.left > 0.0 {
        return;
    }
    cooldown.reset();

    let forward = t.forward();
    let muzzle = t.translation + t.rotation * Vec3::new(0.3, 0.15, -0.9);
    commands.spawn((
        Name::new("Bullet"),
        RoundEntity,
        Bullet { origin: t.translation },
        RigidBody::Dynamic,
        Collider::sphere(0.12),
        GravityScale(0.0),
        LinearVelocity(forward * BULLET_SPEED),
        SweptCcd::default(),
        CollisionEventsEnabled,
        CollisionLayers::new(Layer::Bullet, [Layer::World, Layer::Target]),
        Mesh3d(assets.mesh.clone()),
        MeshMaterial3d(assets.material.clone()),
        Transform::from_translation(muzzle),
    ));
    gunshots.write(Gunshot { muzzle, dir: *forward });

    // Gunshots are loud.
    let (target_e, target_t, mut suspicion) = target.into_inner();
    if target_t.translation.distance(t.translation) < HEARING_RANGE {
        suspicion.bump(0.25);
        suspicion.last_known = Some(t.translation);
        commands.entity(target_e).insert(Alert::new(t.translation, 3.0));
    }
}

fn bullet_hits(
    mut commands: Commands,
    mut collisions: MessageReader<CollisionStart>,
    bullets: Query<(&Bullet, &Transform)>,
    mut targets: Query<(Entity, &mut Target, &Transform, &mut Suspicion)>,
    mut hits: MessageWriter<TargetHit>,
    mut impacts: MessageWriter<BulletImpact>,
) {
    let mut spent = Vec::new();
    for ev in collisions.read() {
        let (bullet_e, other) = if bullets.contains(ev.collider1) {
            (ev.collider1, ev.collider2)
        } else if bullets.contains(ev.collider2) {
            (ev.collider2, ev.collider1)
        } else {
            continue;
        };
        if spent.contains(&bullet_e) {
            continue;
        }
        spent.push(bullet_e);
        let (bullet, bullet_t) = bullets.get(bullet_e).unwrap();
        commands.entity(bullet_e).despawn();

        if let Ok((_, mut target, target_t, mut suspicion)) = targets.get_mut(other) {
            target.hp = target.hp.saturating_sub(1);
            suspicion.bump(0.6);
            suspicion.last_known = Some(bullet.origin);
            commands.entity(other).insert(Alert::new(bullet.origin, 3.0));
            hits.write(TargetHit { at: target_t.translation });
        } else {
            impacts.write(BulletImpact { at: bullet_t.translation });
            // Near miss: he hears the impact and looks toward where it came from.
            for (target_e, _, target_t, mut suspicion) in &mut targets {
                if target_t.translation.distance(bullet_t.translation) < NEAR_MISS_RANGE {
                    suspicion.bump(0.3);
                    suspicion.last_known = Some(bullet.origin);
                    commands.entity(target_e).insert(Alert::new(bullet.origin, 2.5));
                }
            }
        }
    }
}

/// He hears your footsteps when you're close (farther on loud floors), walking into walls,
/// and whistling (from far off).
/// Each sound nudges suspicion and tells him where you are, so creeping up is a risk.
fn hear_movement(
    mut commands: Commands,
    floors: Res<arena::Floors>,
    mut steps: MessageReader<Footstep>,
    mut bumps: MessageReader<Bump>,
    mut whistles: MessageReader<Whistle>,
    target: Single<(Entity, &Transform, &mut Suspicion), With<Target>>,
) {
    let (target_e, target_t, mut suspicion) = target.into_inner();
    let ear = target_t.translation;
    let heard = steps
        .read()
        .map(|s| (s.at, floors.at(s.at.xz()).hearing_range(), FOOTSTEP_SUSPICION))
        .chain(bumps.read().map(|b| (b.at, BUMP_HEARING_RANGE, BUMP_SUSPICION)))
        .chain(whistles.read().map(|w| (w.at, WHISTLE_HEARING_RANGE, WHISTLE_SUSPICION)));
    for (at, range, amount) in heard {
        let closeness = 1.0 - ear.distance(at) / range;
        if closeness <= 0.0 {
            continue;
        }
        suspicion.bump(amount * closeness);
        suspicion.last_known = Some(at);
        commands.entity(target_e).insert(Alert::new(at, 2.0));
    }
}

/// Suspicious but not sure yet: he fires deliberate misses toward where he last saw or heard
/// you, once he's facing that way. Tells you he's onto you (and where he is).
fn warning_fire(
    time: Res<Time>,
    spatial: SpatialQuery,
    mut cooldown: Local<f32>,
    eyes: Single<&GlobalTransform, With<MainCamera>>,
    target: Single<&Suspicion, With<Target>>,
    mut warnings: MessageWriter<WarningShot>,
    mut impacts: MessageWriter<BulletImpact>,
) {
    *cooldown -= time.delta_secs();
    let suspicion = *target;
    let Some(guess) = suspicion.last_known else { return };
    if suspicion.engaged || suspicion.level < WARNING_SUSPICION || *cooldown > 0.0 {
        return;
    }
    let from = eyes.translation() + eyes.down() * 0.3 + eyes.right() * 0.2;
    let to_guess = guess - from;
    if eyes.forward().angle_between(to_guess) > 0.5 {
        return; // still turning to look
    }
    let mut rng = rand::rng();
    *cooldown = rng.random_range(WARNING_INTERVAL);
    let side = to_guess.with_y(0.0).normalize_or_zero().cross(Vec3::Y);
    let sign = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
    let aim = guess + side * sign * rng.random_range(WARNING_MISS) + Vec3::Y * rng.random_range(-0.6..0.4);
    let Ok(dir) = Dir3::new(aim - from) else { return };
    // Stops at whatever it hits first (cover, a wall), else flies on past.
    let to = match spatial.cast_ray(from, dir, 60.0, true, &SpatialQueryFilter::from_mask(Layer::World)) {
        Some(hit) => {
            let at = from + dir * hit.distance;
            impacts.write(BulletImpact { at });
            at
        }
        None => from + dir * 60.0,
    };
    warnings.write(WarningShot { from, to });
}

/// The grappling hook. While fighting, if he can see you at mid range and it's off cooldown,
/// he hooks you and reels you in, fires a quick burst point-blank, leaves you stunned, and
/// runs for a different cover.
#[allow(clippy::too_many_arguments)]
fn grapple(
    mut commands: Commands,
    time: Res<Time>,
    mut cooldown: Local<f32>,
    eyes: Single<&GlobalTransform, With<MainCamera>>,
    target: Single<(Entity, &Transform, &Suspicion, Option<&Activity>, Option<&mut Grapple>), With<Target>>,
    shooter: Single<(&mut Shooter, &Transform, &mut Stunned)>,
    mut fired: MessageWriter<GrappleFired>,
    mut hits: MessageWriter<ShooterHit>,
) {
    let dt = time.delta_secs();
    *cooldown -= dt;
    let (target_e, target_t, suspicion, activity, grapple) = target.into_inner();
    let (mut s, shooter_t, mut stunned) = shooter.into_inner();
    let hand = eyes.translation() + eyes.down() * 0.3 + eyes.right() * 0.2;
    let dist = target_t.translation.xz().distance(shooter_t.translation.xz());

    let Some(mut grapple) = grapple else {
        if *cooldown <= 0.0
            && activity == Some(&Activity::Engaging)
            && suspicion.sees_shooter
            && GRAPPLE_RANGE.contains(&dist)
        {
            *cooldown = GRAPPLE_COOLDOWN;
            commands.entity(target_e).insert(Grapple::Pulling { time: 0.0 }).try_remove::<MoveTo>();
            fired.write(GrappleFired { from: hand, to: shooter_t.translation });
        }
        return;
    };
    match &mut *grapple {
        Grapple::Pulling { time } => {
            *time += dt;
            *stunned = Stunned { left: 0.2, pull_to: Some(target_t.translation) };
            if dist < GRAPPLE_CLOSE || *time > GRAPPLE_MAX_PULL {
                *grapple = Grapple::Hammering { shots_left: HAMMER_SHOTS, next: 0.0 };
            }
        }
        Grapple::Hammering { shots_left, next } => {
            *stunned = Stunned { left: GRAPPLE_STUN, pull_to: None };
            *next -= dt;
            if *next > 0.0 {
                return;
            }
            s.hp = (s.hp - HAMMER_DAMAGE).max(0.0);
            hits.write(ShooterHit { from: hand, to: shooter_t.translation });
            *shots_left -= 1;
            *next = HAMMER_INTERVAL;
            if *shots_left == 0 {
                commands
                    .entity(target_e)
                    .remove::<Grapple>()
                    .insert(Relocate(target_t.translation.xz()));
            }
        }
    }
}

/// While engaging and able to see the shooter, the target fires back (hitscan).
fn return_fire(
    time: Res<Time>,
    mut timer: Local<Option<Timer>>,
    eyes: Single<&GlobalTransform, With<MainCamera>>,
    target: Single<(&Suspicion, Option<&Activity>, Has<Grapple>), With<Target>>,
    mut shooter: Single<(&mut Shooter, &Transform, &mut Stagger)>,
    mut hits: MessageWriter<ShooterHit>,
) {
    let timer = timer.get_or_insert_with(|| Timer::from_seconds(RETURN_FIRE_INTERVAL, TimerMode::Repeating));
    let (suspicion, activity, grappling) = *target;
    if activity != Some(&Activity::Engaging) || !suspicion.sees_shooter || grappling {
        timer.reset();
        return;
    }
    if timer.tick(time.delta()).just_finished() {
        let (ref mut s, t, ref mut stagger) = *shooter;
        s.hp = (s.hp - RETURN_FIRE_DAMAGE).max(0.0);
        // Start the tracer just below the eyes so it's visible from his own view.
        let from = eyes.translation() + eyes.down() * 0.3 + eyes.right() * 0.2;
        hits.write(ShooterHit { from, to: t.translation });
        let away = (t.translation - from).with_y(0.0).normalize_or_zero();
        **stagger = Stagger::knock(away * RETURN_FIRE_KNOCKBACK);
    }
}

fn check_outcome(
    target: Single<&Target>,
    shooter: Single<&Shooter>,
    mut next: ResMut<NextState<GameState>>,
) {
    if target.hp == 0 {
        next.set(GameState::Won);
    } else if shooter.hp <= 0.0 {
        next.set(GameState::Lost);
    }
}
