//! Radar: a top-down orthographic camera in a corner viewport. It sees the arena plus a
//! radar-only render layer holding contacts.
//!
//! `RadarMode` is the difficulty knob:
//! - `Full`: live blips, shooter heading, target view cone.
//! - `Sonar` (default): positions only, revealed by a periodic ping, then fading. Gunshots
//!   also ping the shooter's position.
//! - `Off`: no radar.

use bevy::{
    camera::{ScalingMode, Viewport, visibility::RenderLayers},
    prelude::*,
    window::PrimaryWindow,
};
use bevy_kira_audio::prelude::*;

use crate::{
    arena::ARENA_HALF,
    audio::Sfx,
    combat::Gunshot,
    round::{GameState, RoundEntity},
};

pub const RADAR_LAYER: usize = 1;
/// For things both cameras should see (the arena). Actors stay on layer 0 only, so the
/// radar can never see them directly — only via blips/contacts.
pub const WORLD_AND_RADAR: &[usize] = &[0, RADAR_LAYER];

/// Fraction of the window's shorter side that the radar occupies.
const RADAR_FRACTION: f32 = 0.34;
const RADAR_MARGIN: f32 = 16.0;
const SONAR_PERIOD: f32 = 2.0;
const CONTACT_FADE: f32 = 1.8;
const BLIP_HEIGHT: f32 = 4.0;

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RadarMode {
    Full,
    #[default]
    Sonar,
    Off,
}

impl RadarMode {
    pub fn next(self) -> Self {
        match self {
            Self::Full => Self::Sonar,
            Self::Sonar => Self::Off,
            Self::Off => Self::Full,
        }
    }
}

/// Live radar decoration (blips, heading, view cone): only shown in `Full` mode.
#[derive(Component)]
pub struct LiveBlip;

/// Something sonar reveals, and the colour of its contact.
#[derive(Component)]
pub struct RadarContact(pub Color);

#[derive(Component)]
struct RadarCamera;

/// A fading sonar contact or sweep ring.
#[derive(Component)]
struct Fading {
    timer: Timer,
    material: Handle<StandardMaterial>,
    base_alpha: f32,
    grow_to: Option<f32>,
}

#[derive(Resource)]
struct RadarAssets {
    contact: Handle<Mesh>,
    ring: Handle<Mesh>,
}

/// Where the radar is on screen, in logical pixels, so the HUD can frame it.
#[derive(Resource, Default)]
pub struct RadarRect(pub Rect);

pub fn plugin(app: &mut App) {
    app.init_resource::<RadarRect>()
        .init_resource::<RadarMode>()
        .add_systems(Startup, spawn_radar)
        .add_systems(Update, (apply_mode, fade))
        .add_systems(Update, (sonar_sweep, gunshot_pings).run_if(in_state(GameState::Playing)))
        .add_systems(PostUpdate, fit_viewport);
}

fn spawn_radar(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(RadarAssets {
        contact: meshes.add(Sphere::new(0.9)),
        ring: meshes.add(Annulus::new(0.95, 1.0)),
    });
    commands.spawn((
        Name::new("Radar camera"),
        RadarCamera,
        Camera3d::default(),
        Camera {
            order: 1,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.02, 0.07, 0.04)),
            ..default()
        },
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: ARENA_HALF * 2.0 + 2.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        // North (-Z) is up on the radar.
        Transform::from_xyz(0.0, 60.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
        RenderLayers::layer(RADAR_LAYER),
    ));
}

fn apply_mode(
    mode: Res<RadarMode>,
    mut camera: Single<&mut Camera, With<RadarCamera>>,
    mut blips: Query<&mut Visibility, With<LiveBlip>>,
) {
    camera.is_active = *mode != RadarMode::Off;
    let vis = if *mode == RadarMode::Full {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut blips {
        v.set_if_neq(vis);
    }
}

fn sonar_sweep(
    mut commands: Commands,
    time: Res<Time>,
    mode: Res<RadarMode>,
    mut timer: Local<Option<Timer>>,
    assets: Res<RadarAssets>,
    sfx: Res<Sfx>,
    audio: Res<Audio>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    contacts: Query<(&GlobalTransform, &RadarContact)>,
) {
    let timer = timer.get_or_insert_with(|| Timer::from_seconds(SONAR_PERIOD, TimerMode::Repeating));
    if *mode != RadarMode::Sonar || !timer.tick(time.delta()).just_finished() {
        return;
    }
    audio.play(sfx.ping.clone()).with_volume(-14.0);
    for (t, contact) in &contacts {
        spawn_contact(&mut commands, &assets, &mut materials, t.translation(), contact.0);
    }
    // Sweep ring from the centre of the arena.
    let material = fading_material(&mut materials, Color::srgb(0.3, 1.0, 0.5), 0.6);
    commands.spawn((
        Name::new("Sonar sweep"),
        RoundEntity,
        Mesh3d(assets.ring.clone()),
        MeshMaterial3d(material.clone()),
        Transform::from_xyz(0.0, BLIP_HEIGHT, 0.0)
            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        RenderLayers::layer(RADAR_LAYER),
        Fading {
            timer: Timer::from_seconds(0.9, TimerMode::Once),
            material,
            base_alpha: 0.6,
            grow_to: Some(ARENA_HALF * 1.45),
        },
    ));
}

/// Gunshots give away the shooter's position on the radar (in any mode that has one).
fn gunshot_pings(
    mut commands: Commands,
    mode: Res<RadarMode>,
    assets: Res<RadarAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut shots: MessageReader<Gunshot>,
) {
    for shot in shots.read() {
        if *mode == RadarMode::Sonar {
            spawn_contact(&mut commands, &assets, &mut materials, shot.muzzle, Color::srgb(1.0, 0.8, 0.3));
        }
    }
}

fn spawn_contact(
    commands: &mut Commands,
    assets: &RadarAssets,
    materials: &mut Assets<StandardMaterial>,
    at: Vec3,
    color: Color,
) {
    let material = fading_material(materials, color, 1.0);
    commands.spawn((
        Name::new("Sonar contact"),
        RoundEntity,
        Mesh3d(assets.contact.clone()),
        MeshMaterial3d(material.clone()),
        Transform::from_translation(at.with_y(BLIP_HEIGHT)),
        RenderLayers::layer(RADAR_LAYER),
        Fading {
            timer: Timer::from_seconds(CONTACT_FADE, TimerMode::Once),
            material,
            base_alpha: 1.0,
            grow_to: None,
        },
    ));
}

fn fading_material(materials: &mut Assets<StandardMaterial>, color: Color, alpha: f32) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: color.with_alpha(alpha),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        cull_mode: None,
        ..default()
    })
}

fn fade(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(Entity, &mut Fading, &mut Transform)>,
) {
    for (e, mut f, mut t) in &mut q {
        f.timer.tick(time.delta());
        let k = f.timer.fraction();
        if let Some(mut m) = materials.get_mut(&f.material) {
            m.base_color.set_alpha(f.base_alpha * (1.0 - k));
        }
        if let Some(r) = f.grow_to {
            t.scale = Vec3::splat(r * k.max(0.01));
        }
        if f.timer.is_finished() {
            commands.entity(e).despawn();
        }
    }
}

/// Keep the radar square in the bottom-right corner, whatever the window size.
fn fit_viewport(
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<&mut Camera, With<RadarCamera>>,
    mut rect: ResMut<RadarRect>,
) {
    let scale = window.scale_factor();
    let (w, h) = (window.width(), window.height());
    let side = w.min(h) * RADAR_FRACTION;
    if side < 1.0 {
        return;
    }
    let min = Vec2::new(w - side - RADAR_MARGIN, h - side - RADAR_MARGIN);
    rect.0 = Rect::from_corners(min, min + side);

    let viewport = Viewport {
        physical_position: (min * scale).as_uvec2(),
        physical_size: UVec2::splat((side * scale) as u32),
        ..default()
    };
    let unchanged = camera.viewport.as_ref().is_some_and(|v| {
        v.physical_position == viewport.physical_position && v.physical_size == viewport.physical_size
    });
    if !unchanged {
        camera.viewport = Some(viewport);
    }
}
