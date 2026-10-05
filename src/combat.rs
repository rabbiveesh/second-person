//! Bullets, hits, hearing, and the target shooting back.

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;

use crate::{
    Layer,
    round::{GameState, RoundEntity},
    shooter::{Shooter, ShooterAction},
    target::{Activity, Alert, MainCamera, Suspicion, Target},
};

const BULLET_SPEED: f32 = 45.0;
const FIRE_COOLDOWN: f32 = 0.3;
const HEARING_RANGE: f32 = 18.0;
const NEAR_MISS_RANGE: f32 = 7.0;
const RETURN_FIRE_INTERVAL: f32 = 0.8;
const RETURN_FIRE_DAMAGE: f32 = 8.0;

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
    /// Which way the bullet was travelling (unit, horizontal-ish).
    pub dir: Vec3,
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
        .add_systems(Startup, load_assets)
        .add_systems(
            Update,
            (fire, bullet_hits, return_fire, check_outcome)
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
    shooter: Single<(&ActionState<ShooterAction>, &Transform), With<Shooter>>,
    target: Single<(Entity, &Transform, &mut Suspicion), With<Target>>,
    mut gunshots: MessageWriter<Gunshot>,
) {
    let cooldown = cooldown.get_or_insert_with(|| {
        let mut t = Timer::from_seconds(FIRE_COOLDOWN, TimerMode::Once);
        t.finish();
        t
    });
    cooldown.tick(time.delta());

    let (actions, t) = *shooter;
    if !actions.just_pressed(&ShooterAction::Fire) || !cooldown.is_finished() {
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
            let dir = (target_t.translation - bullet.origin).normalize_or_zero();
            hits.write(TargetHit { at: target_t.translation, dir });
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

/// While engaging and able to see the shooter, the target fires back (hitscan).
fn return_fire(
    time: Res<Time>,
    mut timer: Local<Option<Timer>>,
    eyes: Single<&GlobalTransform, With<MainCamera>>,
    target: Single<(&Suspicion, Option<&Activity>), With<Target>>,
    mut shooter: Single<(&mut Shooter, &Transform)>,
    mut hits: MessageWriter<ShooterHit>,
) {
    let timer = timer.get_or_insert_with(|| Timer::from_seconds(RETURN_FIRE_INTERVAL, TimerMode::Repeating));
    let (suspicion, activity) = *target;
    if activity != Some(&Activity::Engaging) || !suspicion.sees_shooter {
        timer.reset();
        return;
    }
    if timer.tick(time.delta()).just_finished() {
        let (ref mut s, t) = *shooter;
        s.hp = (s.hp - RETURN_FIRE_DAMAGE).max(0.0);
        // Start the tracer just below the eyes so it's visible from his own view.
        let from = eyes.translation() + eyes.down() * 0.3 + eyes.right() * 0.2;
        hits.write(ShooterHit { from, to: t.translation });
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
