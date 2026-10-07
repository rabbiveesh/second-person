//! Audio (bevy_kira_audio) with our own small spatial mix. The listener is the target's head,
//! so everything is heard from *his* position: your footsteps and gunshots tell you where you
//! are relative to the view. Sounds are procedurally generated (see `examples/gen_sfx.rs`).
//!
//! kira's built-in spatial plugin pans hard to one ear and fades linearly in dB, which is
//! useless on one earbud and makes everything equally loud. Instead each sound plays as a dry
//! and a low-passed twin, and [`mix`] sets their volumes and a capped pan every frame:
//! - distance: inverse-distance falloff (gentler for footsteps), faded to silence at the sound's range;
//! - direction: panning capped at [`MAX_PAN`], so both ears always hear it;
//! - behind the listener, or with cover in between: crossfade to the muffled twin and drop a bit.

use bevy::{platform::collections::HashMap, prelude::*};
use bevy_kira_audio::prelude::*;
use rand::Rng;

use crate::{
    arena::{self, Floor},
    combat::{BulletImpact, Gunshot, ShooterHit, TargetHit, WarningShot},
    round::{GameState, RoundEntity},
    shooter::{Bump, Footstep},
    target::MainCamera,
};

// Tuning knobs.
/// Hardest pan, either way. 1.0 would be one ear only.
pub const MAX_PAN: f32 = 0.3;
/// Full volume inside this distance; beyond it, `6 * rolloff` dB quieter per doubling.
const REF_DIST: f32 = 2.0;
/// How much (0..1) a sound directly behind is muffled, and how much quieter it gets.
const BEHIND_MUFFLE: f32 = 0.85;
const BEHIND_DB: f32 = -4.0;
/// Same for a sound with cover between it and the listener.
const OCCLUDED_MUFFLE: f32 = 0.9;
const OCCLUDED_DB: f32 = -6.0;
const FOOTSTEP_DB: f32 = -4.0;
/// Footsteps are your only cue to where you are, so they carry across the whole arena
/// (its diagonal is ~85m) and fade gently: -3 dB per doubling. Distance reads mostly from
/// the muffling and the shrinking volume, not from the step vanishing.
const FOOTSTEP_RANGE: f32 = 90.0;
const FOOTSTEP_ROLLOFF: f32 = 0.5;
/// Playback-rate jitter per footstep, on top of picking a random variant.
const FOOTSTEP_PITCH_JITTER: f64 = 0.06;
const STEP_VARIANTS: usize = 3;
/// Walking into a wall. Carries like footsteps, since it's also about where *you* are.
const BUMP_DB: f32 = -2.0;

/// A sound and its low-passed twin.
struct Sound {
    dry: Handle<AudioSource>,
    muffled: Handle<AudioSource>,
}

impl Sound {
    fn load(assets: &AssetServer, name: &str) -> Self {
        Self {
            dry: assets.load(format!("sfx/{name}.wav")),
            muffled: assets.load(format!("sfx/{name}_muffled.wav")),
        }
    }
}

#[derive(Resource)]
pub struct Sfx {
    shot: Sound,
    impact: Sound,
    target_hit: Sound,
    return_fire: Sound,
    steps: HashMap<Floor, Vec<Sound>>,
    bump: Sound,
}

/// A playing sound placed in the world. Its two instances are re-mixed every frame.
#[derive(Component)]
struct Emitter {
    dry: Handle<AudioInstance>,
    muffled: Handle<AudioInstance>,
    db: f32,
    range: f32,
    rolloff: f32,
}

/// One-shot emitter entity, despawned once its sound has surely finished.
#[derive(Component)]
struct OneShot(Timer);

pub fn plugin(app: &mut App) {
    app.add_plugins(AudioPlugin)
        .add_systems(Startup, load)
        .add_systems(Update, (play_events, expire).run_if(in_state(GameState::Playing)))
        .add_systems(PostUpdate, remix.after(TransformSystems::Propagate));
}

fn load(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(Sfx {
        shot: Sound::load(&assets, "shot"),
        impact: Sound::load(&assets, "impact"),
        target_hit: Sound::load(&assets, "target_hit"),
        return_fire: Sound::load(&assets, "return_fire"),
        steps: Floor::ALL
            .into_iter()
            .map(|f| {
                let variants = (0..STEP_VARIANTS).map(|i| Sound::load(&assets, &format!("step_{}{i}", f.name())));
                (f, variants.collect())
            })
            .collect(),
        bump: Sound::load(&assets, "bump"),
    });
}

/// Volumes (dB) for the dry and muffled instances, and the pan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mix {
    pub dry_db: f32,
    pub muffled_db: f32,
    pub pan: f32,
}

const SILENCE_DB: f32 = -60.0;

fn to_db(amp: f32) -> f32 {
    if amp <= 1e-3 { SILENCE_DB } else { (20.0 * amp.log10()).max(SILENCE_DB) }
}

/// Mix for a sound at `src`, heard by a listener at `ear` (only its yaw matters).
pub fn mix(ear: &GlobalTransform, src: Vec3, db: f32, range: f32, rolloff: f32) -> Mix {
    let (here, there) = (ear.translation().xz(), src.xz());
    let to = there - here;
    let dist = to.length();
    let dir = to.normalize_or_zero();
    let right = ear.right().xz().normalize_or_zero();
    let fwd = ear.forward().xz().normalize_or_zero();

    // Inverse distance, then a smooth fade over the last 30% of the range so nothing pops.
    let fade = 1.0 - ((dist / range - 0.7) / 0.3).clamp(0.0, 1.0);
    let mut amp = (REF_DIST / dist.max(REF_DIST)).powf(rolloff) * fade * fade * (3.0 - 2.0 * fade);

    let behind = (-dir.dot(fwd)).max(0.0);
    let occluded = dist > 0.5 && arena::los_blocked(here, there);
    let mut muffle = behind * BEHIND_MUFFLE;
    let mut extra_db = behind * BEHIND_DB;
    if occluded {
        muffle = muffle.max(OCCLUDED_MUFFLE);
        extra_db += OCCLUDED_DB;
    }
    amp *= 10f32.powf((db + extra_db) / 20.0);

    // Equal-power crossfade, so muffling changes the tone and not the loudness.
    let theta = muffle * std::f32::consts::FRAC_PI_2;
    Mix {
        dry_db: to_db(amp * theta.cos()),
        muffled_db: to_db(amp * theta.sin()),
        pan: dir.dot(right) * MAX_PAN,
    }
}

#[allow(clippy::too_many_arguments)]
fn play_events(
    mut commands: Commands,
    audio: Res<Audio>,
    sfx: Res<Sfx>,
    ear: Query<&GlobalTransform, With<MainCamera>>,
    mut shots: MessageReader<Gunshot>,
    mut impacts: MessageReader<BulletImpact>,
    mut target_hits: MessageReader<TargetHit>,
    mut shooter_hits: MessageReader<ShooterHit>,
    mut warnings: MessageReader<WarningShot>,
    mut steps: MessageReader<Footstep>,
    mut bumps: MessageReader<Bump>,
) {
    let Ok(ear) = ear.single() else { return };
    let mut rng = rand::rng();
    let mut play = |sound: &Sound, at: Vec3, db: f32, range: f32, rolloff: f32, rate: f64| {
        let m = mix(ear, at, db, range, rolloff);
        let start = |source: &Handle<AudioSource>, vol: f32| {
            audio.play(source.clone()).with_volume(vol).with_panning(m.pan).with_playback_rate(rate).handle()
        };
        let emitter = Emitter {
            dry: start(&sound.dry, m.dry_db),
            muffled: start(&sound.muffled, m.muffled_db),
            db,
            range,
            rolloff,
        };
        commands.spawn((
            Name::new("Sound"),
            RoundEntity,
            Transform::from_translation(at),
            emitter,
            OneShot(Timer::from_seconds(2.0, TimerMode::Once)),
        ));
    };
    for s in shots.read() {
        play(&sfx.shot, s.muzzle, 0.0, 80.0, 1.0, 1.0);
    }
    for i in impacts.read() {
        play(&sfx.impact, i.at, -4.0, 30.0, 1.0, 1.0);
    }
    for h in target_hits.read() {
        play(&sfx.target_hit, h.at, 0.0, 10.0, 1.0, 1.0);
    }
    for w in warnings.read() {
        play(&sfx.return_fire, w.from, -2.0, 30.0, 1.0, 1.0);
    }
    for h in shooter_hits.read() {
        play(&sfx.return_fire, h.from, -2.0, 30.0, 1.0, 1.0);
    }
    for s in steps.read() {
        let variants = &sfx.steps[&arena::floor_at(s.at.xz())];
        let step = &variants[rng.random_range(0..variants.len())];
        let rate = 1.0 + rng.random_range(-FOOTSTEP_PITCH_JITTER..FOOTSTEP_PITCH_JITTER);
        play(step, s.at, FOOTSTEP_DB, FOOTSTEP_RANGE, FOOTSTEP_ROLLOFF, rate);
    }
    for b in bumps.read() {
        play(&sfx.bump, b.at, BUMP_DB, FOOTSTEP_RANGE, FOOTSTEP_ROLLOFF, 1.0);
    }
}

/// Keep the mix right as the listener turns or walks while a sound is still playing.
fn remix(
    ear: Query<&GlobalTransform, With<MainCamera>>,
    emitters: Query<(&Transform, &Emitter)>,
    mut instances: ResMut<Assets<AudioInstance>>,
) {
    let Ok(ear) = ear.single() else { return };
    for (t, e) in &emitters {
        let m = mix(ear, t.translation, e.db, e.range, e.rolloff);
        for (handle, db) in [(&e.dry, m.dry_db), (&e.muffled, m.muffled_db)] {
            if let Some(mut i) = instances.get_mut(handle) {
                i.set_decibels(db, AudioTween::default());
                i.set_panning(m.pan, AudioTween::default());
            }
        }
    }
}

fn expire(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut OneShot)>) {
    for (e, mut t) in &mut q {
        if t.0.tick(time.delta()).is_finished() {
            commands.entity(e).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Listener at the origin facing -Z (Bevy's forward), so +X is his right.
    fn ear() -> GlobalTransform {
        GlobalTransform::from_xyz(0.0, 1.6, 0.0)
    }

    #[test]
    fn pan_is_capped_so_both_ears_hear_it() {
        let m = mix(&ear(), Vec3::new(5.0, 0.0, 0.0), 0.0, 50.0, 1.0);
        assert!((m.pan - MAX_PAN).abs() < 1e-4, "{m:?}");
        let m = mix(&ear(), Vec3::new(-5.0, 0.0, 0.0), 0.0, 50.0, 1.0);
        assert!((m.pan + MAX_PAN).abs() < 1e-4, "{m:?}");
    }

    #[test]
    fn farther_is_quieter_and_out_of_range_is_silent() {
        let near = mix(&ear(), Vec3::new(0.0, 0.0, -4.0), 0.0, 30.0, 1.0);
        let far = mix(&ear(), Vec3::new(0.0, 0.0, -16.0), 0.0, 30.0, 1.0);
        assert!(near.dry_db > far.dry_db + 10.0, "{near:?} {far:?}");
        let gone = mix(&ear(), Vec3::new(0.0, 0.0, -31.0), 0.0, 30.0, 1.0);
        assert_eq!(gone.dry_db, SILENCE_DB);
    }

    #[test]
    fn behind_is_muffled() {
        let front = mix(&ear(), Vec3::new(0.0, 0.0, -4.0), 0.0, 30.0, 1.0);
        let back = mix(&ear(), Vec3::new(0.0, 0.0, 4.0), 0.0, 30.0, 1.0);
        assert!(front.dry_db > front.muffled_db + 20.0, "{front:?}");
        assert!(back.muffled_db > back.dry_db + 10.0, "{back:?}");
    }
}
