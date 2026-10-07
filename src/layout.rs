//! Screen layout: where the 3D view, the radar and the touch deck go, and the field of view
//! that fits them. Recomputed from the window every frame, so rotating a phone or resizing a
//! browser re-lays everything out on the next frame.
//!
//! - **Landscape** (and anything near square): the view fills the window and the radar sits in
//!   the bottom-right corner over it.
//! - **Portrait** (a phone held upright): a full-height view would be a tall, narrow slit, so the
//!   bottom of the screen becomes a *deck* below the view. The radar and the whistle button live
//!   there, and the left thumb drives from there, so nothing covers his eyes.
//!
//! Field of view is Hor+ with a floor: the vertical FOV stays at `BASE_VFOV` while the view is
//! wide enough to show at least `MIN_HFOV` across, and opens up (to `MAX_VFOV`) when it isn't.
//! Bevy's `PerspectiveProjection::fov` is vertical, so a fixed value alone would crop the sides
//! harder the narrower the screen gets.

use bevy::{
    camera::{CameraUpdateSystems, Viewport},
    prelude::*,
    window::PrimaryWindow,
};

use crate::{radar::RadarCamera, target::MainCamera, touch::WHISTLE_SIZE};

/// Vertical FOV on a landscape screen.
pub const BASE_VFOV: f32 = 70f32.to_radians();
/// The least horizontal FOV any screen gets, as long as `MAX_VFOV` allows.
pub const MIN_HFOV: f32 = 80f32.to_radians();
/// Beyond this the edges of the view stretch too much to be worth the extra width.
pub const MAX_VFOV: f32 = 100f32.to_radians();

/// Height over width at which the deck opens below the view.
const PORTRAIT_ASPECT: f32 = 1.3;
const MARGIN: f32 = 16.0;
/// Landscape radar: this fraction of the window's shorter side.
const RADAR_FRACTION: f32 = 0.34;
/// Portrait radar: half the width, up to this.
const DECK_RADAR_MAX: f32 = 260.0;

/// Where things are on screen, in logical pixels (origin top-left).
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct ScreenLayout {
    /// The 3D view through his eyes.
    pub view: Rect,
    /// The control deck below the view, in portrait. Empty in landscape.
    pub deck: Rect,
    pub radar: Rect,
}

impl ScreenLayout {
    pub fn fit(size: Vec2) -> Self {
        let (w, h) = (size.x, size.y);
        if h < w * PORTRAIT_ASPECT {
            let side = w.min(h) * RADAR_FRACTION;
            let min = Vec2::new(w - side - MARGIN, h - side - MARGIN);
            return Self {
                view: Rect::from_corners(Vec2::ZERO, size),
                deck: Rect::default(),
                radar: Rect::from_corners(min, min + side),
            };
        }
        // The radar takes the right half of the deck (taps there fire, and it only shows), with
        // the whistle stacked above it. The left half is free for the drive stick.
        let side = (w / 2.0 - 1.5 * MARGIN).min(DECK_RADAR_MAX);
        let deck_top = h - (side + WHISTLE_SIZE + 3.0 * MARGIN);
        let min = Vec2::new(w - side - MARGIN, h - side - MARGIN);
        Self {
            view: Rect::new(0.0, 0.0, w, deck_top),
            deck: Rect::new(0.0, deck_top, w, h),
            radar: Rect::from_corners(min, min + side),
        }
    }

    pub fn portrait(&self) -> bool {
        self.deck.height() > 0.0
    }
}

/// Vertical FOV for a view `aspect` (width / height) wide: Hor+ above `MIN_HFOV`.
pub fn vfov_for(aspect: f32) -> f32 {
    let for_min_hfov = 2.0 * ((MIN_HFOV / 2.0).tan() / aspect).atan();
    for_min_hfov.clamp(BASE_VFOV, MAX_VFOV)
}

pub fn plugin(app: &mut App) {
    app.init_resource::<ScreenLayout>()
        .add_systems(PostUpdate, (fit_layout, (fit_eyes, fit_radar)).chain().before(CameraUpdateSystems));
}

fn fit_layout(window: Single<&Window, With<PrimaryWindow>>, mut layout: ResMut<ScreenLayout>) {
    if window.width() < 1.0 || window.height() < 1.0 {
        return;
    }
    layout.set_if_neq(ScreenLayout::fit(window.size()));
}

/// The eyes respawn with him every round, so this keeps any new camera in line too.
fn fit_eyes(
    layout: Res<ScreenLayout>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut eyes: Query<(&mut Camera, &mut Projection), With<MainCamera>>,
) {
    let view = layout.view;
    if view.width() < 1.0 || view.height() < 1.0 {
        return;
    }
    let viewport = layout.portrait().then(|| viewport(view, window.scale_factor()));
    let fov = vfov_for(view.width() / view.height());
    for (mut camera, mut projection) in &mut eyes {
        set_viewport(&mut camera, viewport.clone());
        if let Projection::Perspective(p) = projection.as_ref()
            && p.fov != fov
            && let Projection::Perspective(p) = projection.as_mut()
        {
            p.fov = fov;
        }
    }
}

fn fit_radar(
    layout: Res<ScreenLayout>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<&mut Camera, With<RadarCamera>>,
) {
    if layout.radar.width() >= 1.0 {
        set_viewport(&mut camera, Some(viewport(layout.radar, window.scale_factor())));
    }
}

fn viewport(rect: Rect, scale: f32) -> Viewport {
    Viewport {
        physical_position: (rect.min * scale).as_uvec2(),
        physical_size: (rect.size() * scale).as_uvec2().max(UVec2::ONE),
        ..default()
    }
}

/// Only touch the camera when the viewport actually moves, so change detection stays quiet.
fn set_viewport(camera: &mut Mut<Camera>, viewport: Option<Viewport>) {
    let same = match (&camera.viewport, &viewport) {
        (None, None) => true,
        (Some(a), Some(b)) => a.physical_position == b.physical_position && a.physical_size == b.physical_size,
        _ => false,
    };
    if !same {
        camera.viewport = viewport;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hfov(vfov: f32, aspect: f32) -> f32 {
        2.0 * ((vfov / 2.0).tan() * aspect).atan()
    }

    #[test]
    fn landscape_keeps_the_base_fov() {
        for aspect in [16.0 / 9.0, 844.0 / 390.0, 4.0 / 3.0] {
            assert_eq!(vfov_for(aspect), BASE_VFOV, "aspect {aspect}");
        }
    }

    #[test]
    fn narrow_views_open_up_to_the_minimum_width() {
        let layout = ScreenLayout::fit(Vec2::new(390.0, 844.0));
        let aspect = layout.view.width() / layout.view.height();
        let across = hfov(vfov_for(aspect), aspect).to_degrees();
        assert!((across - MIN_HFOV.to_degrees()).abs() < 0.5, "a portrait phone sees {across}° across");
        // A full-height view at the base FOV would show barely a third of that.
        assert!(hfov(BASE_VFOV, 390.0 / 844.0).to_degrees() < 40.0);
    }

    #[test]
    fn extreme_slivers_stop_at_the_cap() {
        assert_eq!(vfov_for(0.2), MAX_VFOV);
    }

    #[test]
    fn landscape_has_no_deck_and_the_radar_overlays_the_view() {
        let layout = ScreenLayout::fit(Vec2::new(844.0, 390.0));
        assert!(!layout.portrait());
        assert_eq!(layout.view, Rect::new(0.0, 0.0, 844.0, 390.0));
        assert!(layout.view.contains(layout.radar.center()));
    }

    #[test]
    fn portrait_puts_radar_and_whistle_in_a_deck_below_the_view() {
        let size = Vec2::new(390.0, 844.0);
        let layout = ScreenLayout::fit(size);
        assert!(layout.portrait());
        assert_eq!(layout.view.max.y, layout.deck.min.y);
        assert_eq!(layout.deck.max, size);
        // Radar fully in the deck, on the right (fire) half, with room for the whistle above.
        assert!(layout.radar.min.y - layout.deck.min.y >= WHISTLE_SIZE + MARGIN);
        assert!(layout.radar.min.x > size.x / 2.0);
        assert!(layout.radar.max.x <= size.x && layout.radar.max.y <= size.y);
        // The view stays wider than a 3:4 portrait slit.
        assert!(layout.view.width() / layout.view.height() > 0.65);
    }
}
