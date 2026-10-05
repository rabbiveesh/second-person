//! Headless gameplay tests: the real `gameplay` plugins on `MinimalPlugins`, with a
//! fixed 60 Hz clock so runs are deterministic(ish — the AI's scan uses rand).

use std::time::Duration;

use avian3d::prelude::*;
use bevy::{
    input::InputPlugin, prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy,
};
use bevy::ecs::system::RunSystemOnce;
use leafwing_input_manager::prelude::*;
use second_person::{
    combat::{Bullet, Gunshot, ShooterHit, TargetHit},
    round::GameState,
    shooter::{SHOOTER_MAX_HP, Shooter},
    target::{Activity, Alert, MainCamera, Suspicion, TARGET_MAX_HP, Target},
};

const DT: f32 = 1.0 / 60.0;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        InputPlugin,
        AssetPlugin::default(),
        TransformPlugin,
        bevy::mesh::MeshPlugin,
    ))
    .init_asset::<StandardMaterial>()
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(DT)))
    .add_plugins((second_person::gameplay, second_person::juice::plugin));
    count::<Gunshot>(&mut app);
    count::<TargetHit>(&mut app);
    count::<ShooterHit>(&mut app);
    // `App::run` would call these; plugins like avian init resources in `finish`.
    app.finish();
    app.cleanup();
    app.update(); // Startup + OnEnter(Playing)
    app.update();
    app
}

/// Counts messages of type `M` into `Count<M>`.
#[derive(Resource)]
struct Count<M>(usize, std::marker::PhantomData<M>);

fn count<M: Message>(app: &mut App) {
    app.insert_resource(Count::<M>(0, default()))
        .add_systems(Last, |mut r: MessageReader<M>, mut c: ResMut<Count<M>>| c.0 += r.read().count());
}

fn counted<M: Message>(app: &App) -> usize {
    app.world().resource::<Count<M>>().0
}

fn step(app: &mut App, secs: f32) {
    for _ in 0..(secs / DT).ceil() as usize {
        app.update();
    }
}

fn single<C: Component>(app: &mut App) -> Entity {
    app.world_mut().query_filtered::<Entity, With<C>>().single(app.world()).unwrap()
}

/// Teleport a rigid body (both the Transform and avian's Position/Rotation).
fn place(app: &mut App, e: Entity, pos: Vec3, yaw: f32) {
    let rot = Quat::from_rotation_y(yaw);
    let mut entity = app.world_mut().entity_mut(e);
    entity.insert((
        Transform::from_translation(pos).with_rotation(rot),
        Position(pos),
        Rotation(rot),
        LinearVelocity::ZERO,
    ));
}

/// Yaw that makes `from` face `to` (Bevy forward is -Z).
fn yaw_towards(from: Vec3, to: Vec3) -> f32 {
    let d = to - from;
    f32::atan2(-d.x, -d.z)
}

/// Put the target at the origin facing `point`, and the shooter at `shooter_pos` facing the target.
fn stage(app: &mut App, shooter_pos: Vec3, target_faces: Vec3) {
    let target = single::<Target>(app);
    let shooter = single::<Shooter>(app);
    let t = Vec3::new(0.0, 0.9, 0.0);
    place(app, target, t, yaw_towards(t, target_faces));
    place(app, shooter, shooter_pos, yaw_towards(shooter_pos, t));
    app.update();
}

fn press(app: &mut App, key: KeyCode) {
    key.press(app.world_mut());
    app.update();
    key.release(app.world_mut());
    app.update();
}

#[test]
fn round_starts_with_one_of_each() {
    let mut app = app();
    assert_eq!(app.world_mut().query::<&Shooter>().iter(app.world()).count(), 1);
    assert_eq!(app.world_mut().query::<&Target>().iter(app.world()).count(), 1);
    assert_eq!(*app.world().resource::<State<GameState>>().get(), GameState::Playing);
}

#[test]
fn firing_spawns_a_bullet_and_a_gunshot() {
    let mut app = app();
    press(&mut app, KeyCode::Space);
    assert_eq!(counted::<Gunshot>(&app), 1);
    assert_eq!(app.world_mut().query::<&Bullet>().iter(app.world()).count(), 1);
}

#[test]
fn shooting_the_target_hurts_and_alerts_him() {
    let mut app = app();
    // Shooter right behind him: point blank, and out of his view.
    stage(&mut app, Vec3::new(0.0, 0.9, 6.0), Vec3::new(0.0, 0.9, -10.0));
    press(&mut app, KeyCode::Space);
    step(&mut app, 0.5);

    assert_eq!(counted::<TargetHit>(&app), 1);
    let target = single::<Target>(&mut app);
    assert_eq!(app.world().get::<Target>(target).unwrap().hp, TARGET_MAX_HP - 1);
    assert!(app.world().get::<Suspicion>(target).unwrap().level > 0.5);
}

/// The eyes' local offset angle from rest (the AI only turns the body and head).
fn flinch_angle(app: &mut App) -> f32 {
    let cam = single::<MainCamera>(app);
    app.world().get::<Transform>(cam).unwrap().rotation.angle_between(Quat::IDENTITY)
}

#[test]
fn getting_shot_jolts_his_view_then_it_settles() {
    let mut app = app();
    stage(&mut app, Vec3::new(0.0, 0.9, 6.0), Vec3::new(0.0, 0.9, -10.0));
    press(&mut app, KeyCode::Space);
    let mut peak: f32 = 0.0;
    for _ in 0..30 {
        app.update();
        peak = peak.max(flinch_angle(&mut app));
    }
    assert_eq!(counted::<TargetHit>(&app), 1);
    assert!(peak > 0.1, "view should visibly snap, peaked at {peak}");
    step(&mut app, 1.5);
    assert!(flinch_angle(&mut app) < 0.01, "view should settle back");
}

#[test]
fn killing_him_drops_his_view_to_the_floor() {
    let mut app = app();
    stage(&mut app, Vec3::new(0.0, 0.9, 6.0), Vec3::new(0.0, 0.9, -10.0));
    let target = single::<Target>(&mut app);
    app.world_mut().get_mut::<Target>(target).unwrap().hp = 1;
    press(&mut app, KeyCode::Space);
    step(&mut app, 0.5);
    assert_eq!(*app.world().resource::<State<GameState>>().get(), GameState::Won);
    step(&mut app, 1.5);
    let cam = single::<MainCamera>(&mut app);
    let eyes = app.world().get::<GlobalTransform>(cam).unwrap().compute_transform();
    assert!(eyes.translation.y < 0.5, "eyes at {} should be near the floor", eyes.translation.y);
    // Lying on his side: the view's up is mostly sideways.
    assert!(eyes.up().y.abs() < 0.4, "view up {:?} should be tipped over", eyes.up());
}

#[test]
fn the_shooter_falls_over_when_killed() {
    let mut app = app();
    stage(&mut app, Vec3::new(0.0, 0.9, 6.0), Vec3::new(0.0, 0.9, -10.0));
    let shooter = single::<Shooter>(&mut app);
    let floor = app.world().get::<Transform>(shooter).unwrap().translation.y - 0.85;
    app.world_mut().get_mut::<Shooter>(shooter).unwrap().hp = 0.0;
    app.update();
    assert_eq!(*app.world().resource::<State<GameState>>().get(), GameState::Lost);
    step(&mut app, 1.5);
    let t = app.world().get::<Transform>(shooter).unwrap();
    assert!(t.up().y.abs() < 0.1, "shooter should be lying down, up = {:?}", t.up());
    assert!((t.translation.y - floor - 0.35).abs() < 0.1, "should rest on the floor, at {}", t.translation.y);
}

#[test]
fn he_sees_the_shooter_in_his_view() {
    let mut app = app();
    let shooter_pos = Vec3::new(0.0, 0.9, -10.0);
    stage(&mut app, shooter_pos, shooter_pos);
    app.update();
    let target = single::<Target>(&mut app);
    let s = app.world().get::<Suspicion>(target).unwrap();
    assert!(s.sees_shooter, "shooter straight ahead in the open should be seen");
    assert!(s.level > 0.0);
}

#[test]
fn cover_blocks_line_of_sight() {
    let mut app = app();
    // Crate at (-6, _, -8); shooter directly behind it as seen from the origin.
    let shooter_pos = Vec3::new(-9.0, 0.9, -12.0);
    stage(&mut app, shooter_pos, shooter_pos);
    app.update();
    let target = single::<Target>(&mut app);
    assert!(!app.world().get::<Suspicion>(target).unwrap().sees_shooter);
}

#[test]
fn nearby_gunshot_makes_him_investigate() {
    let mut app = app();
    // Behind him (out of view) but within hearing range, firing away from him.
    let shooter = single::<Shooter>(&mut app);
    let target = single::<Target>(&mut app);
    stage(&mut app, Vec3::new(0.0, 0.9, 10.0), Vec3::new(0.0, 0.9, -10.0));
    place(&mut app, shooter, Vec3::new(0.0, 0.9, 10.0), 0.0 + std::f32::consts::PI);
    press(&mut app, KeyCode::Space);
    step(&mut app, 0.2);
    assert!(app.world().get::<Alert>(target).is_some());
    assert_eq!(app.world().get::<Activity>(target), Some(&Activity::Investigating));
}

#[test]
fn engaged_target_runs_for_cover() {
    let mut app = app();
    let shooter_pos = Vec3::new(0.0, 0.9, -12.0);
    stage(&mut app, shooter_pos, shooter_pos);
    let target = single::<Target>(&mut app);
    {
        let mut s = app.world_mut().get_mut::<Suspicion>(target).unwrap();
        s.bump(1.0);
        s.last_known = Some(shooter_pos);
    }
    let mut took_cover = false;
    let mut hidden = false;
    for _ in 0..(6.0 / DT) as usize {
        app.update();
        took_cover |= app.world().get::<Activity>(target) == Some(&Activity::TakingCover);
        let p = app.world().get::<Transform>(target).unwrap().translation;
        hidden |= second_person::arena::los_blocked(shooter_pos.xz(), p.xz());
    }
    assert!(took_cover, "never ran for cover");
    assert!(hidden, "never got out of the shooter's line of sight");
}

#[test]
fn engaged_target_eventually_kills_an_exposed_shooter_then_r_restarts() {
    let mut app = app();
    let shooter_pos = Vec3::new(0.0, 0.9, -8.0);
    stage(&mut app, shooter_pos, shooter_pos);
    let target = single::<Target>(&mut app);
    app.world_mut().get_mut::<Suspicion>(target).unwrap().bump(1.0);

    // He hides and peeks, so this takes a while; a shooter standing in the open still loses.
    for i in 0..(90.0 / DT) as usize {
        app.update();
        if std::env::var("TRACE").is_ok() && i % 30 == 0 {
            let w = app.world();
            let s = w.get::<Suspicion>(target).unwrap();
            eprintln!(
                "t={:.1} {:?} pos={:.1?} lvl={:.2} eng={} sees={} last={:?}",
                i as f32 * DT,
                w.get::<Activity>(target),
                w.get::<Transform>(target).unwrap().translation.xz(),
                s.level, s.engaged, s.sees_shooter, s.last_known.map(|p| p.xz())
            );
            eprintln!(
                "      moveto={:?} route={:?} cover={:?}",
                w.get::<second_person::nav::MoveTo>(target).map(|m| m.dest.xz()),
                w.get::<second_person::nav::Route>(target).map(|r| r.0.iter().map(|p| p.xz()).collect::<Vec<_>>()),
                w.get::<second_person::target::CoverPlan>(target).map(|c| c.0),
            );
        }
        if *app.world().resource::<State<GameState>>().get() != GameState::Playing {
            break;
        }
    }
    assert!(counted::<ShooterHit>(&app) > 0);
    assert_eq!(*app.world().resource::<State<GameState>>().get(), GameState::Lost);

    press(&mut app, KeyCode::KeyR);
    app.update();
    assert_eq!(*app.world().resource::<State<GameState>>().get(), GameState::Playing);
    let shooters: Vec<f32> = app.world_mut().query::<&Shooter>().iter(app.world()).map(|s| s.hp).collect();
    assert_eq!(shooters, vec![SHOOTER_MAX_HP]);
}

#[test]
fn cover_spots_hide_from_the_threat() {
    use second_person::arena::{find_cover, is_clear, los_blocked};
    for threat in [Vec2::new(0.0, -12.0), Vec2::new(15.0, 15.0), Vec2::new(-20.0, 0.0)] {
        let cover = find_cover(Vec2::ZERO, threat).expect("some cover exists");
        assert!(los_blocked(threat, cover.spot), "{cover:?} visible from {threat}");
        assert!(is_clear(cover.spot, 0.5));
    }
}

#[test]
fn navmesh_routes_around_cover() {
    use second_person::nav::Nav;
    let mut app = app();
    // Straight line from (-10,-6) to (-10,2) goes through the pillar at (-10,-2).
    let (a, b) = (Vec3::new(-10.0, 0.9, -6.0), Vec3::new(-10.0, 0.9, 2.0));
    let path = app
        .world_mut()
        .run_system_once(move |nav: Nav| nav.path(a, b))
        .unwrap()
        .expect("navmesh built and path found");
    let mut prev = a.xz();
    for w in path {
        for i in 0..=20 {
            let p = prev.lerp(w.xz(), i as f32 / 20.0);
            assert!(second_person::arena::is_clear(p, 0.3), "path passes through cover at {p}");
        }
        prev = w.xz();
    }
}

#[test]
fn random_starts_are_clear_of_cover_and_the_target() {
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    for _ in 0..500 {
        let t = second_person::shooter::random_start(&mut rng);
        let p = t.translation.xz();
        assert!(p.length() >= second_person::shooter::MIN_START_DISTANCE, "{p}");
        assert!(second_person::arena::is_clear(p, 0.35), "{p} overlaps cover");
    }
}

#[test]
fn radar_cannot_see_the_shooter_directly() {
    use bevy::camera::visibility::RenderLayers;
    use second_person::radar::{LiveBlip, RADAR_LAYER};
    let mut app = app();
    let radar = RenderLayers::layer(RADAR_LAYER);
    let shooter = single::<Shooter>(&mut app);
    // The body and every non-blip child mesh must be invisible to the radar camera.
    let mut meshes = vec![shooter];
    meshes.extend(app.world().get::<Children>(shooter).unwrap().iter());
    for e in meshes {
        if app.world().get::<LiveBlip>(e).is_some() || app.world().get::<Mesh3d>(e).is_none() {
            continue;
        }
        let layers = app.world().get::<RenderLayers>(e).cloned().unwrap_or_default();
        assert!(!layers.intersects(&radar), "{:?} is visible on radar", app.world().get::<Name>(e));
    }
}

#[test]
fn no_shadow_casting_light_reaches_the_radar() {
    use bevy::camera::visibility::RenderLayers;
    use second_person::radar::RADAR_LAYER;
    // Shadows from layer-0 actors would show up on the radar's ground and give them away.
    let mut app = app();
    let radar = RenderLayers::layer(RADAR_LAYER);
    let mut q = app.world_mut().query::<(&DirectionalLight, Option<&RenderLayers>)>();
    for (light, layers) in q.iter(app.world()) {
        if light.shadow_maps_enabled {
            assert!(!layers.cloned().unwrap_or_default().intersects(&radar));
        }
    }
}

/// Gameplay + radar, startup only: radar's Update systems need audio, which tests don't have.
fn radar_startup_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, InputPlugin, AssetPlugin::default(), TransformPlugin))
        .add_plugins((bevy::mesh::MeshPlugin, bevy::window::WindowPlugin::default()))
        .init_asset::<StandardMaterial>()
        .add_plugins((second_person::gameplay, second_person::radar::plugin));
    app.finish();
    app.cleanup();
    app.world_mut().run_schedule(Startup);
    app
}

#[test]
fn at_most_one_directional_light_with_presentation() {
    // WebGL2 caps directional lights at 1 *globally*; a second one (e.g. a radar-only light)
    // silently drops the sun and leaves the main view black on the web.
    let mut app = radar_startup_app();
    let n = app.world_mut().query::<&DirectionalLight>().iter(app.world()).count();
    assert_eq!(n, 1, "WebGL2 supports one directional light");
}
