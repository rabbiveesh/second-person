//! Radar: a top-down orthographic camera in a corner viewport. It sees the arena plus a
//! radar-only render layer holding contacts.
//!
//! `RadarMode` is the difficulty knob:
//! - `Full`: live blips, shooter heading, target view cone.
//! - `Sonar` (default): positions only, revealed by a periodic ping, then fading. Gunshots
//!   also ping the shooter's position.
//! - `Off`: no radar.

use bevy::{
    camera::{ScalingMode, visibility::RenderLayers},
    prelude::*,
};

use crate::{
    arena::Layout,
    combat::Gunshot,
    round::{GameState, RoundEntity},
};

pub const RADAR_LAYER: usize = 1;
/// For things both cameras should see (the arena). Actors stay on layer 0 only, so the
/// radar can never see them directly — only via blips/contacts.
pub const WORLD_AND_RADAR: &[usize] = &[0, RADAR_LAYER];

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

/// Laid out by `layout::ScreenLayout`.
#[derive(Component)]
pub struct RadarCamera;

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

pub fn plugin(app: &mut App) {
    app.init_resource::<RadarMode>()
        .add_systems(Startup, spawn_radar)
        .add_systems(Update, (apply_mode, fade, frame_layout.run_if(resource_changed::<Layout>)))
        .add_systems(Update, (sonar_sweep, gunshot_pings).run_if(in_state(GameState::Playing)));
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
            // Fitted to the arena by `frame_layout`.
            scaling_mode: ScalingMode::FixedVertical { viewport_height: 62.0 },
            ..OrthographicProjection::default_3d()
        }),
        // North (-Z) is up on the radar.
        Transform::from_xyz(0.0, 60.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
        RenderLayers::layer(RADAR_LAYER),
        // Lit by its own flat ambient instead of a light on the radar layer: shadows would
        // leak the actors onto the map, and WebGL2 allows only one directional light in
        // total (a second one silently drops the sun from the main view).
        AmbientLight {
            brightness: 3_500.0,
            ..default()
        },
    ));
}

/// Centre the radar on the arena and zoom so the whole floor fits.
fn frame_layout(layout: Res<Layout>, mut camera: Single<(&mut Projection, &mut Transform), With<RadarCamera>>) {
    let bounds = layout.bounds();
    let (projection, transform) = &mut *camera;
    if let Projection::Orthographic(ortho) = &mut **projection {
        let side = bounds.size().max_element() + 2.0;
        ortho.scaling_mode = ScalingMode::FixedVertical { viewport_height: side };
    }
    let c = bounds.center();
    **transform = Transform::from_xyz(c.x, 60.0, c.y).looking_at(Vec3::new(c.x, 0.0, c.y), Vec3::NEG_Z);
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
    layout: Res<Layout>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    contacts: Query<(&GlobalTransform, &RadarContact)>,
) {
    let timer = timer.get_or_insert_with(|| Timer::from_seconds(SONAR_PERIOD, TimerMode::Repeating));
    if *mode != RadarMode::Sonar || !timer.tick(time.delta()).just_finished() {
        return;
    }
    for (t, contact) in &contacts {
        spawn_contact(&mut commands, &assets, &mut materials, t.translation(), contact.0);
    }
    // Sweep ring from the centre of the arena out to its corners.
    let bounds = layout.bounds();
    let material = fading_material(&mut materials, Color::srgb(0.3, 1.0, 0.5), 0.6);
    commands.spawn((
        Name::new("Sonar sweep"),
        RoundEntity,
        Mesh3d(assets.ring.clone()),
        MeshMaterial3d(material.clone()),
        Transform::from_xyz(bounds.center().x, BLIP_HEIGHT, bounds.center().y)
            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        RenderLayers::layer(RADAR_LAYER),
        Fading {
            timer: Timer::from_seconds(0.9, TimerMode::Once),
            material,
            base_alpha: 0.6,
            grow_to: Some(bounds.half_size().length() * 1.03),
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
