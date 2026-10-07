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
    arena::{Floor, Floors},
    shooter::{Bump, SHOOTER_MAX_HP, Shooter},
    target::{Activity, Alert, Suspicion, TARGET_MAX_HP, Target},
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
    .add_plugins(second_person::gameplay);
    count::<Gunshot>(&mut app);
    count::<TargetHit>(&mut app);
    count::<ShooterHit>(&mut app);
    count::<Bump>(&mut app);
    count::<second_person::combat::WarningShot>(&mut app);
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
                "t={:.1} hp={} {:?} pos={:.1?} lvl={:.2} eng={} sees={} last={:?}",
                i as f32 * DT,
                w.iter_entities().find_map(|e| e.get::<Shooter>().map(|s| s.hp)).unwrap_or(-1.0),
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

#[test]
fn walking_into_a_wall_thuds_and_knocks_you_back() {
    let mut app = app();
    let shooter = single::<Shooter>(&mut app);
    let start = Vec3::new(20.0, 0.9, 25.0);
    place(&mut app, shooter, start, yaw_towards(start, start + Vec3::Z));
    KeyCode::ArrowUp.press(app.world_mut());
    let z = |app: &App| app.world().get::<Transform>(shooter).unwrap().translation.z;
    for _ in 0..120 {
        app.update();
        if counted::<Bump>(&app) > 0 {
            break;
        }
    }
    assert_eq!(counted::<Bump>(&app), 1, "no bump");
    let at_wall = z(&app);
    assert!(at_wall < 29.5, "went through the wall: {at_wall}");
    KeyCode::ArrowUp.release(app.world_mut());
    step(&mut app, 0.5);
    assert!(z(&app) < at_wall - 0.3, "not knocked back: {} vs {at_wall}", z(&app));
}

#[test]
fn floor_zones() {
    let floors = Floors::classic();
    assert_eq!(floors.at(Vec2::ZERO), Floor::Wood);
    assert_eq!(floors.at(Vec2::new(20.0, -20.0)), Floor::Gravel);
    assert_eq!(floors.at(Vec2::new(-20.0, -20.0)), Floor::Metal);
    assert_eq!(floors.at(Vec2::new(0.0, 20.0)), Floor::Grass);
}

#[test]
fn engaged_target_moves_between_covers() {
    let mut app = app();
    let shooter_pos = Vec3::new(0.0, 0.9, -12.0);
    stage(&mut app, shooter_pos, shooter_pos);
    let shooter = single::<Shooter>(&mut app);
    app.world_mut().get_mut::<Shooter>(shooter).unwrap().hp = 1e6;
    let target = single::<Target>(&mut app);
    {
        let mut s = app.world_mut().get_mut::<Suspicion>(target).unwrap();
        s.bump(1.0);
        s.last_known = Some(shooter_pos);
    }
    let mut spots: Vec<Vec2> = Vec::new();
    for _ in 0..(40.0 / DT) as usize {
        app.update();
        if let Some(c) = app.world().get::<second_person::target::CoverPlan>(target)
            && !spots.iter().any(|s| s.distance(c.0.spot) < 1.0)
        {
            spots.push(c.0.spot);
        }
    }
    assert!(spots.len() >= 2, "only ever used {spots:?}");
}

#[test]
fn he_hears_footsteps_up_close_on_loud_floors() {
    use second_person::shooter::Footstep;
    let mut app = app();
    // Facing away from where the steps happen, so it's hearing, not sight.
    stage(&mut app, Vec3::new(0.0, 0.9, -25.0), Vec3::new(0.0, 0.9, -10.0));
    let target = single::<Target>(&mut app);
    let level = |app: &App| app.world().get::<Suspicion>(target).unwrap().level;

    // Grass, 9m behind him: out of earshot.
    app.world_mut().write_message(Footstep { at: Vec3::new(0.0, 0.9, 9.0) });
    app.update();
    assert_eq!(level(&app), 0.0);

    // Wood plaza, 3m behind: heard, and he knows where.
    app.world_mut().write_message(Footstep { at: Vec3::new(0.0, 0.9, 3.0) });
    app.update();
    assert!(level(&app) > 0.0);
    assert!(app.world().get::<Alert>(target).is_some());
}

#[test]
fn his_laser_knocks_you_back() {
    use second_person::{combat::Grapple, shooter::{Stagger, Stunned}};
    let mut app = app();
    let shooter_pos = Vec3::new(0.0, 0.9, -8.0);
    stage(&mut app, shooter_pos, shooter_pos);
    let target = single::<Target>(&mut app);
    app.world_mut().get_mut::<Suspicion>(target).unwrap().bump(1.0);
    let shooter = single::<Shooter>(&mut app);
    app.world_mut().get_mut::<Shooter>(shooter).unwrap().hp = 1e6;
    // Wait for a plain return-fire hit (not part of a grapple burst).
    let mut seen = counted::<ShooterHit>(&app);
    for _ in 0..(60.0 / DT) as usize {
        app.update();
        let hits = counted::<ShooterHit>(&app);
        let w = app.world();
        if hits > seen && w.get::<Grapple>(target).is_none() && w.get::<Stunned>(shooter).unwrap().left <= 0.0 {
            let stagger = w.get::<Stagger>(shooter).unwrap();
            assert!(stagger.left > 0.0 && stagger.push.length() > 1.0);
            return;
        }
        seen = hits;
    }
    panic!("no plain return fire");
}

#[test]
fn suspicious_target_fires_warning_shots_that_miss() {
    use second_person::combat::WarningShot;
    let mut app = app();
    let guess = Vec3::new(0.0, 0.9, -10.0);
    // You're actually far behind him; he thinks you're in front.
    stage(&mut app, Vec3::new(0.0, 0.9, 25.0), guess);
    let target = single::<Target>(&mut app);
    {
        let mut s = app.world_mut().get_mut::<Suspicion>(target).unwrap();
        s.level = 0.8;
        s.last_known = Some(guess);
    }
    step(&mut app, 2.0);
    assert!(counted::<WarningShot>(&app) >= 1, "no warning shot");
    assert_eq!(counted::<ShooterHit>(&app), 0);
    assert!(!app.world().get::<Suspicion>(target).unwrap().engaged);
}

#[test]
fn grapple_pulls_you_in_hammers_stuns_and_he_runs() {
    use second_person::{combat::Grapple, shooter::Stunned};
    let mut app = app();
    let shooter_pos = Vec3::new(0.0, 0.9, -12.0);
    stage(&mut app, shooter_pos, shooter_pos);
    let target = single::<Target>(&mut app);
    let shooter = single::<Shooter>(&mut app);
    app.world_mut().get_mut::<Shooter>(shooter).unwrap().hp = 1e6;
    {
        let mut s = app.world_mut().get_mut::<Suspicion>(target).unwrap();
        s.bump(1.0);
        s.last_known = Some(shooter_pos);
    }
    let dist = |app: &App| {
        let w = app.world();
        w.get::<Transform>(target).unwrap().translation.xz().distance(w.get::<Transform>(shooter).unwrap().translation.xz())
    };
    // Wait for the hook.
    let mut hooked_at = None;
    for _ in 0..(30.0 / DT) as usize {
        app.update();
        if app.world().get::<Grapple>(target).is_some() {
            hooked_at = Some(dist(&app));
            break;
        }
    }
    let hooked_at = hooked_at.expect("never grappled");
    let hits_before = counted::<ShooterHit>(&app);
    let mut closest = hooked_at;
    let mut stunned = false;
    for _ in 0..(4.0 / DT) as usize {
        app.update();
        closest = closest.min(dist(&app));
        stunned |= app.world().get::<Stunned>(shooter).unwrap().left > 0.5;
        if app.world().get::<Grapple>(target).is_none() {
            break;
        }
    }
    assert!(app.world().get::<Grapple>(target).is_none(), "grapple never finished");
    assert!(closest < 3.5, "not reeled in: {hooked_at} -> {closest}");
    assert!(counted::<ShooterHit>(&app) >= hits_before + 3, "no burst");
    assert!(stunned, "never stunned");
    // Then he runs for it.
    step(&mut app, 1.5);
    assert!(dist(&app) > closest + 2.0, "didn't run: {}", dist(&app));
}

#[test]
fn whistling_is_heard_from_far_off() {
    let mut app = app();
    // 20m behind him: far beyond footstep range, within whistle range.
    stage(&mut app, Vec3::new(0.0, 0.9, 20.0), Vec3::new(0.0, 0.9, -10.0));
    let target = single::<Target>(&mut app);
    press(&mut app, KeyCode::KeyC);
    let s = app.world().get::<Suspicion>(target).unwrap();
    assert!(s.level > 0.0, "didn't hear it");
    assert!(app.world().get::<Alert>(target).is_some());
}
