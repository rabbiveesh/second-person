//! Spatial audio (bevy_kira_audio). The listener is the target's head, so everything is
//! heard from *his* position: your footsteps and gunshots tell you where you are relative
//! to the view. Sounds are procedurally generated (see `examples/gen_sfx.rs`).

use bevy::prelude::*;
use bevy_kira_audio::{SpatialRadius, prelude::*};

use crate::{
    combat::{BulletImpact, Gunshot, ShooterHit, TargetHit},
    round::{GameState, RoundEntity},
    shooter::Footstep,
    target::MainCamera,
};

#[derive(Resource)]
pub struct Sfx {
    shot: Handle<AudioSource>,
    impact: Handle<AudioSource>,
    target_hit: Handle<AudioSource>,
    return_fire: Handle<AudioSource>,
    step: Handle<AudioSource>,
}

/// One-shot emitter entity, despawned once its sound has surely finished.
#[derive(Component)]
struct OneShot(Timer);

pub fn plugin(app: &mut App) {
    app.add_plugins((AudioPlugin, SpatialAudioPlugin))
        .add_systems(Startup, load)
        .add_systems(Update, attach_listener)
        .add_systems(Update, (play_events, expire).run_if(in_state(GameState::Playing)));
}

fn load(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(Sfx {
        shot: assets.load("sfx/shot.wav"),
        impact: assets.load("sfx/impact.wav"),
        target_hit: assets.load("sfx/target_hit.wav"),
        return_fire: assets.load("sfx/return_fire.wav"),
        step: assets.load("sfx/step.wav"),
    });
}

/// The target's eyes are also his ears.
fn attach_listener(mut commands: Commands, cams: Query<Entity, Added<MainCamera>>) {
    for cam in &cams {
        commands.entity(cam).insert(SpatialAudioReceiver);
    }
}

#[allow(clippy::too_many_arguments)]
fn play_events(
    mut commands: Commands,
    audio: Res<Audio>,
    sfx: Res<Sfx>,
    mut shots: MessageReader<Gunshot>,
    mut impacts: MessageReader<BulletImpact>,
    mut target_hits: MessageReader<TargetHit>,
    mut shooter_hits: MessageReader<ShooterHit>,
    mut steps: MessageReader<Footstep>,
) {
    let mut play = |sound: &Handle<AudioSource>, at: Vec3, radius: f32| {
        let instance = audio.play(sound.clone()).handle();
        commands.spawn((
            Name::new("Sound"),
            RoundEntity,
            Transform::from_translation(at),
            SpatialAudioEmitter {
                instances: vec![instance],
            },
            SpatialRadius { radius },
            OneShot(Timer::from_seconds(2.0, TimerMode::Once)),
        ));
    };
    for s in shots.read() {
        play(&sfx.shot, s.muzzle, 80.0);
    }
    for i in impacts.read() {
        play(&sfx.impact, i.at, 30.0);
    }
    for h in target_hits.read() {
        play(&sfx.target_hit, h.at, 10.0);
    }
    for h in shooter_hits.read() {
        play(&sfx.return_fire, h.from, 30.0);
    }
    for s in steps.read() {
        play(&sfx.step, s.at, 24.0);
    }
}

fn expire(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut OneShot)>) {
    for (e, mut t) in &mut q {
        if t.0.tick(time.delta()).is_finished() {
            commands.entity(e).despawn();
        }
    }
}
