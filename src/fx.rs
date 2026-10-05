//! Visual effects (bevy_firework particles + short-lived lights) driven by combat messages.
//! The muzzle flash is a navigation aid: it lights up the shooter's surroundings even
//! when he's off-screen.

use bevy::prelude::*;
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
    combat::{BulletImpact, Gunshot, ShooterHit, TargetHit},
    round::RoundEntity,
};

/// Despawn after this long (flash lights, tracers).
#[derive(Component)]
struct Ttl(Timer);

#[derive(Component)]
struct Tracer {
    from: Vec3,
    to: Vec3,
}

pub fn plugin(app: &mut App) {
    app.add_plugins(ParticleSystemPlugin::default())
        .add_systems(Update, (muzzle_flash, impact_sparks, target_hit, return_fire, expire, draw_tracers));
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

fn return_fire(mut commands: Commands, mut hits: MessageReader<ShooterHit>) {
    for hit in hits.read() {
        commands.spawn((
            Name::new("Tracer"),
            RoundEntity,
            Tracer { from: hit.from, to: hit.to },
            Ttl(Timer::from_seconds(0.12, TimerMode::Once)),
        ));
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
