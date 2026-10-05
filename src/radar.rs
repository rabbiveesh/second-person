//! Radar: a top-down orthographic camera in a corner viewport. It sees the world
//! plus a radar-only render layer holding the blips and the target's view cone.

use bevy::{
    camera::{ScalingMode, Viewport, visibility::RenderLayers},
    prelude::*,
    window::PrimaryWindow,
};

use crate::arena::ARENA_HALF;

pub const RADAR_LAYER: usize = 1;
pub const WORLD_AND_RADAR: &[usize] = &[0, RADAR_LAYER];

/// Fraction of the window's shorter side that the radar occupies.
const RADAR_FRACTION: f32 = 0.34;
const RADAR_MARGIN: f32 = 16.0;

#[derive(Component)]
struct RadarCamera;

/// Where the radar is on screen, in logical pixels, so the HUD can frame it.
#[derive(Resource, Default)]
pub struct RadarRect(pub Rect);

pub fn plugin(app: &mut App) {
    app.init_resource::<RadarRect>()
        .add_systems(Startup, spawn_radar)
        .add_systems(PostUpdate, fit_viewport);
}

fn spawn_radar(mut commands: Commands) {
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
        RenderLayers::from_layers(WORLD_AND_RADAR),
    ));
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
