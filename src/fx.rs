//! Visual effects (bevy_firework particles + short-lived lights) driven by combat messages.
//! The muzzle flash is a navigation aid: it lights up the shooter's surroundings even
//! when he's off-screen.

use bevy::{light::NotShadowCaster, prelude::*};
use bevy_firework::{
    bevy_utilitarian::prelude::{RandF32, RandVec3},
    core::{
        BlendMode, EmissionPacing, EmissionSettings, ParticleSettings, ParticleSpawner,
        ParticleSpawnerFinished,
    },
    curve::{FireworkCurve, FireworkGradient},
    emission_shape::EmissionShape,
    plugin::ParticleSystemPlugin,
};

use crate::{
    combat::{BulletImpact, Grapple, GrappleFired, Gunshot, ShooterHit, TargetHit, WarningShot},
    shooter::Shooter,
    target::MainCamera,
    round::RoundEntity,
};

/// Despawn after this long (flash lights, tracers).
#[derive(Component)]
struct Ttl(Timer);

/// Thins out over its `Ttl` (the lingering tracer of the killing shot).
#[derive(Component)]
struct Thin;

#[derive(Component)]
struct Tracer {
    from: Vec3,
    to: Vec3,
}

pub fn plugin(app: &mut App) {
    app.add_plugins(ParticleSystemPlugin::default())
        .add_systems(Update, (muzzle_flash, impact_sparks, target_hit, return_fire, thin, expire, draw_tracers, spawn_hook, fly_hook));
}

fn muzzle_flash(mut commands: Commands, mut shots: MessageReader<Gunshot>) {
    for shot in shots.read() {
        commands.spawn((
            Name::new("Muzzle light"),
            RoundEntity,
            PointLight {
                color: Color::srgb(1.0, 0.75, 0.4),
                intensity: 2_000_000.0,
                range: 14.0,
                ..default()
            },
            Transform::from_translation(shot.muzzle),
            Ttl(Timer::from_seconds(0.08, TimerMode::Once)),
        ));
        burst(
            &mut commands,
            "Muzzle flash",
            shot.muzzle,
            shot.dir,
            0.35,
            16,
            (6.0, 14.0),
            LinearRgba::rgb(12.0, 7.0, 2.0),
            0.18,
        );
    }
}

fn impact_sparks(mut commands: Commands, mut impacts: MessageReader<BulletImpact>) {
    for impact in impacts.read() {
        burst(
            &mut commands,
            "Impact sparks",
            impact.at,
            Vec3::Y,
            1.2,
            12,
            (2.0, 6.0),
            LinearRgba::rgb(6.0, 5.0, 3.0),
            0.35,
        );
    }
}

fn target_hit(mut commands: Commands, mut hits: MessageReader<TargetHit>) {
    for hit in hits.read() {
        burst(
            &mut commands,
            "Blood",
            hit.at,
            Vec3::Y,
            1.5,
            20,
            (1.0, 4.0),
            LinearRgba::rgb(2.0, 0.0, 0.0),
            0.5,
        );
    }
}

fn return_fire(
    mut commands: Commands,
    mut hits: MessageReader<ShooterHit>,
    mut warnings: MessageReader<WarningShot>,
    shooter: Option<Single<&Shooter>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let dead = shooter.is_some_and(|s| s.hp <= 0.0);
    let hits = hits.read().map(|h| (h.from, h.to, true));
    let shots = hits.chain(warnings.read().map(|w| (w.from, w.to, false)));
    for (from, to, hit) in shots {
        commands.spawn((
            Name::new("Tracer"),
            RoundEntity,
            Tracer { from, to },
            Ttl(Timer::from_seconds(0.12, TimerMode::Once)),
        ));
        if !(hit && dead) {
            continue;
        }
        // The killing shot: a thick beam that hangs in the air, and a big burst where it lands.
        // It starts a little way out so it doesn't fill his view up close.
        let dir = (to - from).normalize_or(Vec3::Y);
        let from = from + dir * 1.5;
        let span = to - from;
        commands.spawn((
            Name::new("Killing tracer"),
            RoundEntity,
            Mesh3d(meshes.add(Cylinder::new(0.035, span.length()))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.35, 0.2),
                emissive: LinearRgba::rgb(10.0, 2.0, 1.0),
                unlit: true,
                ..default()
            })),
            NotShadowCaster,
            Transform::from_translation(from + span / 2.0)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, span.normalize_or(Vec3::Y))),
            Thin,
            Ttl(Timer::from_seconds(1.5, TimerMode::Once)),
        ));
        commands.spawn((
            Name::new("Killing light"),
            RoundEntity,
            PointLight {
                color: Color::srgb(1.0, 0.4, 0.3),
                intensity: 3_000_000.0,
                range: 10.0,
                ..default()
            },
            Transform::from_translation(to),
            Ttl(Timer::from_seconds(0.15, TimerMode::Once)),
        ));
        burst(
            &mut commands,
            "Shooter down",
            to,
            span.normalize_or(Vec3::Y),
            1.4,
            48,
            (3.0, 9.0),
            LinearRgba::rgb(8.0, 1.0, 0.4),
            0.7,
        );
    }
}

fn thin(mut q: Query<(&mut Transform, &Ttl), With<Thin>>) {
    for (mut t, ttl) in &mut q {
        let left = 1.0 - ttl.0.fraction();
        t.scale = Vec3::new(left, 1.0, left);
    }
}

/// The grappling hook in flight / latched on: a shaft with three barbed prongs, flying from
/// his hand to you and riding along while you're reeled in, with its rope back to his hand.
#[derive(Component)]
struct Hook {
    flight: f32,
}

/// The rope: a unit-length cylinder stretched between his hand and the hook each frame.
#[derive(Component)]
struct HookRope;

const HOOK_FLIGHT_SECS: f32 = 0.15;
/// Big enough to read at range; it's a game, not a hardware store.
const HOOK_SCALE: f32 = 2.2;

fn spawn_hook(
    mut commands: Commands,
    mut fired: MessageReader<GrappleFired>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    old: Query<Entity, Or<(With<Hook>, With<HookRope>)>>,
) {
    let Some(shot) = fired.read().last() else { return };
    old.iter().for_each(|e| commands.entity(e).despawn());
    commands.spawn((
        Name::new("Grappling rope"),
        RoundEntity,
        HookRope,
        Mesh3d(meshes.add(Cylinder::new(0.025, 1.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.55, 0.45, 0.3))),
        Transform::from_translation(shot.from).with_scale(Vec3::ZERO),
    ));
    let metal = materials.add(StandardMaterial {
        base_color: Color::srgb(0.6, 0.62, 0.66),
        metallic: 0.8,
        perceptual_roughness: 0.35,
        ..default()
    });
    // Modelled pointing down -Z (its flight direction): shaft, a ring at the back for the
    // rope, and three prongs at the tip curling back like an anchor.
    let shaft = meshes.add(Cylinder::new(0.035, 0.5));
    let ring = meshes.add(Torus::new(0.03, 0.06));
    let prong = meshes.add(Cuboid::new(0.035, 0.035, 0.28));
    let barb = meshes.add(Cuboid::new(0.03, 0.03, 0.1));
    let mut hook = commands.spawn((
        Name::new("Grappling hook"),
        RoundEntity,
        Hook { flight: 0.0 },
        Transform::from_translation(shot.from).looking_at(shot.to, Vec3::Y).with_scale(Vec3::splat(HOOK_SCALE)),
        Visibility::default(),
    ));
    hook.with_children(|h| {
        h.spawn((
            Mesh3d(shaft),
            MeshMaterial3d(metal.clone()),
            Transform::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        ));
        h.spawn((
            Mesh3d(ring),
            MeshMaterial3d(metal.clone()),
            Transform::from_xyz(0.0, 0.0, 0.28).with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
        ));
        for i in 0..3 {
            // Each prong leaves the tip angled outward and back; a barb points inward at its end.
            let around = Quat::from_rotation_z(i as f32 * std::f32::consts::TAU / 3.0);
            let out = Quat::from_rotation_x(-0.75);
            h.spawn((
                Mesh3d(prong.clone()),
                MeshMaterial3d(metal.clone()),
                Transform::from_rotation(around * out).with_translation(around * Vec3::new(0.0, 0.08, -0.17)),
            ));
            h.spawn((
                Mesh3d(barb.clone()),
                MeshMaterial3d(metal.clone()),
                Transform::from_rotation(around * Quat::from_rotation_x(0.9))
                    .with_translation(around * Vec3::new(0.0, 0.17, -0.04)),
            ));
        }
    });
}

/// Fly the hook out, then keep it latched on your front while you're reeled in; drop it after.
#[allow(clippy::type_complexity)]
fn fly_hook(
    mut commands: Commands,
    time: Res<Time>,
    grapple: Option<Single<&Grapple>>,
    eyes: Option<Single<&GlobalTransform, With<MainCamera>>>,
    shooter: Option<Single<&Transform, (With<Shooter>, Without<Hook>, Without<HookRope>)>>,
    mut hooks: Query<(Entity, &mut Hook, &mut Transform), Without<HookRope>>,
    mut ropes: Query<(Entity, &mut Transform), With<HookRope>>,
) {
    let pulling = grapple.is_some_and(|g| matches!(**g, Grapple::Pulling { .. }));
    let (true, Some(eyes), Some(shooter)) = (pulling, eyes, shooter) else {
        hooks.iter().map(|h| h.0).chain(ropes.iter().map(|r| r.0)).for_each(|e| commands.entity(e).despawn());
        return;
    };
    let hand = eyes.translation() + eyes.down() * 0.3 + eyes.right() * 0.2;
    let to_you = shooter.translation - hand;
    // Latch point: your body's surface facing him, so the hook isn't buried inside you.
    let latch = shooter.translation - to_you.normalize_or_zero() * 0.5;
    for (_, mut hook, mut t) in &mut hooks {
        hook.flight += time.delta_secs();
        let pos = hand.lerp(latch, (hook.flight / HOOK_FLIGHT_SECS).min(1.0));
        *t = Transform::from_translation(pos)
            .looking_to(to_you, Vec3::Y)
            .with_scale(Vec3::splat(HOOK_SCALE));
        // The rope ties onto the ring at the back of the hook.
        let tail = pos - to_you.normalize_or_zero() * 0.28 * HOOK_SCALE;
        for (_, mut rope) in &mut ropes {
            let span = tail - hand;
            *rope = Transform::from_translation(hand + span / 2.0)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, span.normalize_or(Vec3::Y)))
                .with_scale(Vec3::new(1.0, span.length(), 1.0));
        }
    }
}

fn draw_tracers(tracers: Query<&Tracer>, mut gizmos: Gizmos) {
    for t in &tracers {
        gizmos.line(t.from, t.to, Color::srgb(1.0, 0.3, 0.2));
    }
}

fn expire(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Ttl)>) {
    for (e, mut ttl) in &mut q {
        if ttl.0.tick(time.delta()).is_finished() {
            commands.entity(e).despawn();
        }
    }
}

/// One-shot particle burst, cleaned up when the spawner finishes.
#[allow(clippy::too_many_arguments)]
fn burst(
    commands: &mut Commands,
    name: &'static str,
    at: Vec3,
    dir: Vec3,
    spread: f32,
    count: usize,
    speed: (f32, f32),
    glow: LinearRgba,
    lifetime: f32,
) {
    commands
        .spawn((
            Name::new(name),
            RoundEntity,
            Transform::from_translation(at),
            ParticleSpawner {
                particle_settings: vec![ParticleSettings {
                    lifetime: RandF32 {
                        min: lifetime * 0.6,
                        max: lifetime,
                    },
                    initial_scale: RandF32 { min: 0.04, max: 0.09 },
                    scale_curve: FireworkCurve::even_samples(vec![1.0, 0.2]),
                    base_color: FireworkGradient::even_samples(vec![glow, glow.with_alpha(0.0)]),
                    emissive_color: FireworkGradient::even_samples(vec![glow, glow.with_alpha(0.0)]),
                    blend_mode: BlendMode::Add,
                    pbr: false,
                    linear_drag: 2.0,
                    acceleration: Vec3::new(0.0, -6.0, 0.0),
                    fade_scene: 0.0,
                    ..default()
                }],
                emission_settings: vec![EmissionSettings {
                    emission_pacing: EmissionPacing::OneShot(count),
                    emission_shape: EmissionShape::Point,
                    initial_velocity: RandVec3 {
                        direction: dir,
                        magnitude: RandF32 {
                            min: speed.0,
                            max: speed.1,
                        },
                        spread,
                    },
                    ..default()
                }],
                ..default()
            },
        ))
        .observe(|finished: On<ParticleSpawnerFinished>, mut commands: Commands| {
            commands.entity(finished.event_target()).despawn();
        });
}
