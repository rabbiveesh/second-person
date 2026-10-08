//! Headless gameplay tests: the real `gameplay` plugins on `MinimalPlugins`, with a
//! fixed 60 Hz clock so runs are deterministic(ish — the AI's scan uses rand).

use std::{f32::consts::TAU, time::Duration};

use avian3d::prelude::*;
use bevy::{
    input::InputPlugin, prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy,
};
use bevy::ecs::system::RunSystemOnce;
use leafwing_input_manager::prelude::*;
use second_person::{
    arena::{ArenaMode, Layout},
    combat::{Bullet, Gunshot, ShooterHit, TargetHit},
    round::GameState,
    arena::{Floor, Floors},
    shooter::{Bump, SHOOTER_MAX_HP, Shooter},
    target::{Activity, Alert, Suspicion, TARGET_MAX_HP, Target, VIEW_RANGE},
};

const DT: f32 = 1.0 / 60.0;

fn app() -> App {
    app_cfg(ArenaMode::Classic, |_| {})
}

/// `app()` plus extra plugins, which must go in before `finish`.
fn app_with(extra: impl FnOnce(&mut App)) -> App {
    app_cfg(ArenaMode::Classic, extra)
}

fn app_in(arena: ArenaMode) -> App {
    app_cfg(arena, |_| {})
}

fn app_cfg(arena: ArenaMode, extra: impl FnOnce(&mut App)) -> App {
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
    .add_plugins(second_person::gameplay)
    .insert_resource(arena);
    extra(&mut app);
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
    // Not the random start: that can put you nose-up against a crate, which eats the bullet.
    stage(&mut app, Vec3::new(0.0, 0.9, -12.0), Vec3::NEG_Z);
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
        hidden |= Layout::classic().los_blocked(shooter_pos.xz(), p.xz());
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
            let (hp, you) = w
                .iter_entities()
                .find_map(|e| Some((e.get::<Shooter>()?.hp, e.get::<Transform>()?.translation.xz())))
                .unwrap();
            eprintln!(
                "t={:.1} hp={} you={:.1?} {:?} pos={:.1?} lvl={:.2} eng={} sees={} last={:?}",
                i as f32 * DT,
                hp,
                you,
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
    let layout = Layout::classic();
    for threat in [Vec2::new(0.0, -12.0), Vec2::new(15.0, 15.0), Vec2::new(-20.0, 0.0)] {
        let cover = layout.find_cover(Vec2::ZERO, threat).expect("some cover exists");
        assert!(layout.los_blocked(threat, cover.spot), "{cover:?} visible from {threat}");
        assert!(layout.is_clear(cover.spot, 0.5));
    }
}

/// He never quite lands on his peek spot and you rarely stay exactly where he saw you, so a
/// peek whose sightline grazes a corner sees nothing but crate and the fight stalls.
#[test]
fn peeks_see_the_threat_with_some_slack() {
    let layout = Layout::classic();
    let ring: Vec<Vec2> = (0..8).map(|i| Vec2::from_angle(i as f32 * TAU / 8.0) * 0.45).chain([Vec2::ZERO]).collect();
    let mut checked = 0;
    for x in (-26..=26).step_by(4) {
        for z in (-26..=26).step_by(4) {
            let threat = Vec2::new(x as f32, z as f32);
            if !layout.is_clear(threat, 1.0) {
                continue;
            }
            for from in [Vec2::ZERO, Vec2::new(15.0, -15.0), Vec2::new(-15.0, 15.0)] {
                let Some(cover) = layout.find_cover(from, threat) else { continue };
                for peek in [cover.peek, cover.alt_peek] {
                    assert!(peek.distance(threat) < VIEW_RANGE - 1.0, "{cover:?}: peek out of eyesight of {threat}");
                    for (&him, &you) in ring.iter().flat_map(|a| ring.iter().map(move |b| (a, b))) {
                        assert!(
                            !layout.los_blocked(peek + him, threat + you),
                            "{cover:?}: peek {} can't see {}",
                            peek + him,
                            threat + you
                        );
                    }
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 100, "only {checked} peeks checked");
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
            assert!(Layout::classic().is_clear(p, 0.3), "path passes through cover at {p}");
        }
        prev = w.xz();
    }
}

#[test]
fn random_starts_are_clear_of_cover_and_the_target() {
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    let layouts = std::iter::once(Layout::classic()).chain((0..40).map(Layout::random));
    for layout in layouts {
        for _ in 0..50 {
            let t = second_person::shooter::random_start(&mut rng, &layout);
            let p = t.translation.xz();
            assert!(p.length() >= second_person::shooter::MIN_START_DISTANCE, "{p} in {}", layout.name);
            assert!(layout.is_clear(p, 0.35), "{p} overlaps cover in {}", layout.name);
        }
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

/// The first seed whose generated layout has each shape.
fn seed_per_shape() -> Vec<u64> {
    let mut seen = std::collections::HashSet::new();
    (0..500)
        .filter(|&seed| seen.insert(Layout::random(seed).name.split(' ').next().unwrap().to_owned()))
        .collect()
}

#[test]
fn generated_layouts_are_sound() {
    use rand::SeedableRng;
    assert_eq!(seed_per_shape().len(), 6, "every shape shows up");
    for seed in 0..300 {
        let layout = Layout::random(seed);
        let name = &layout.name;
        assert_eq!(Layout::random(seed).outline, layout.outline, "{name} is not reproducible");
        assert!(layout.area() > 400.0, "{name} too small: {}", layout.area());
        assert!(layout.is_clear(Vec2::ZERO, 3.0), "{name}: the target's spawn is blocked");
        assert!(layout.blocks.len() >= 3, "{name}: only {} blocks", layout.blocks.len());
        for (i, a) in layout.blocks.iter().enumerate() {
            assert!(layout.contains(a.center), "{name}: block outside the walls");
            for b in &layout.blocks[i + 1..] {
                let gap = ((a.center - b.center).abs() - (a.half + b.half)).max_element();
                assert!(gap > 2.0, "{name}: blocks {a:?} and {b:?} leave no room to pass");
            }
        }
        // Somewhere to hide from a shooter standing in the open.
        let threat = layout
            .random_point(&mut rand::rngs::StdRng::seed_from_u64(seed), 1.0, |p| p.length() > 10.0)
            .unwrap();
        assert!(layout.find_cover(Vec2::ZERO, threat).is_some(), "{name}: no cover from {threat}");
    }
}

#[test]
fn walls_block_line_of_sight_round_corners() {
    // Notch: the bitten-out side blocks the view across it.
    let layout = Layout {
        name: "test".into(),
        outline: vec![
            Vec2::new(-20.0, -20.0),
            Vec2::new(20.0, -20.0),
            Vec2::new(20.0, 20.0),
            Vec2::new(5.0, 20.0),
            Vec2::new(5.0, 5.0),
            Vec2::new(-5.0, 5.0),
            Vec2::new(-5.0, 20.0),
            Vec2::new(-20.0, 20.0),
        ],
        blocks: vec![],
    };
    assert!(layout.los_blocked(Vec2::new(-12.0, 15.0), Vec2::new(12.0, 15.0)));
    assert!(!layout.los_blocked(Vec2::new(-12.0, 0.0), Vec2::new(12.0, 0.0)));
    assert!(!layout.contains(Vec2::new(0.0, 10.0)));
    assert!(!layout.is_clear(Vec2::new(0.0, 4.5), 1.0), "too close to the notch wall");
}

#[test]
fn every_generated_shape_is_walkable_end_to_end() {
    use rand::SeedableRng;
    use second_person::nav::Nav;
    for seed in seed_per_shape() {
        let mut app = app_in(ArenaMode::Seed(seed));
        // The navmesh picks up the new arena's obstacles a few frames in.
        step(&mut app, 0.2);
        let layout = app.world().resource::<Layout>().clone();
        assert_eq!(layout.name, Layout::random(seed).name);
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        for _ in 0..25 {
            let to = layout.random_point(&mut rng, 1.0, |_| true).unwrap();
            let (a, b) = (Vec3::new(0.0, 0.9, 0.0), Vec3::new(to.x, 0.9, to.y));
            let path = app
                .world_mut()
                .run_system_once(move |nav: Nav| nav.path(a, b))
                .unwrap()
                .unwrap_or_else(|| panic!("{}: no path to {to}", layout.name));
            let mut prev = a.xz();
            for w in path {
                for i in 0..=20 {
                    let p = prev.lerp(w.xz(), i as f32 / 20.0);
                    assert!(layout.is_clear(p, 0.3), "{}: path to {to} clips {p}", layout.name);
                }
                prev = w.xz();
            }
        }
    }
}

#[test]
fn l_switches_arena_and_rebuilds_it() {
    let mut app = app();
    let walls = |app: &mut App| {
        app.world_mut()
            .query::<&Name>()
            .iter(app.world())
            .filter(|n| n.as_str() == "Wall")
            .count()
    };
    assert_eq!(walls(&mut app), 4);
    assert_eq!(app.world().resource::<Layout>().name, "Classic");

    press(&mut app, KeyCode::KeyL);
    assert_eq!(*app.world().resource::<ArenaMode>(), ArenaMode::Random);
    let layout = app.world().resource::<Layout>().clone();
    assert_ne!(layout.name, "Classic");
    assert_eq!(walls(&mut app), layout.outline.len(), "old walls despawned, new ones spawned");
    assert_eq!(app.world_mut().query::<&Shooter>().iter(app.world()).count(), 1);
    assert_eq!(app.world_mut().query::<&Target>().iter(app.world()).count(), 1);

    press(&mut app, KeyCode::KeyL);
    assert_eq!(app.world().resource::<Layout>().name, "Classic");
    assert_eq!(walls(&mut app), 4);
}
#[test]
fn arrow_keys_drive_the_shooter_like_a_tank() {
    let mut app = app();
    let shooter = single::<Shooter>(&mut app);
    let start = Vec3::new(0.0, 0.9, 15.0);
    place(&mut app, shooter, start, 0.0); // facing -Z, towards the origin
    app.update();

    KeyCode::ArrowUp.press(app.world_mut());
    step(&mut app, 0.5);
    KeyCode::ArrowUp.release(app.world_mut());
    let pos = app.world().get::<Transform>(shooter).unwrap().translation;
    assert!(pos.z < start.z - 1.0, "Up drives forward: {pos}");
    assert!((pos.x - start.x).abs() < 0.1, "and straight: {pos}");

    KeyCode::ArrowLeft.press(app.world_mut());
    step(&mut app, 0.5);
    KeyCode::ArrowLeft.release(app.world_mut());
    let (yaw, _, _) = app.world().get::<Transform>(shooter).unwrap().rotation.to_euler(EulerRot::YXZ);
    assert!(yaw > 0.5, "Left turns counter-clockwise (left): yaw {yaw}");
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
    // Close enough that the cover he picks (off to the side) keeps you within grapple range.
    let shooter_pos = Vec3::new(0.0, 0.9, -10.0);
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
    press(&mut app, KeyCode::KeyW);
    let s = app.world().get::<Suspicion>(target).unwrap();
    assert!(s.level > 0.0, "didn't hear it");
    assert!(app.world().get::<Alert>(target).is_some());
}

#[test]
fn touch_stick_snaps_to_arrow_key_directions() {
    use second_person::touch::snap_8way;
    assert_eq!(snap_8way(Vec2::new(0.1, 0.1)), Vec2::ZERO, "dead zone");
    assert_eq!(snap_8way(Vec2::new(0.7, 0.7)), Vec2::ONE, "diagonal = up and right held");
    assert_eq!(snap_8way(Vec2::new(0.4, 0.9)), Vec2::Y, "mostly up = full forward, no turn");
    assert_eq!(snap_8way(Vec2::new(-0.9, 0.2)), Vec2::NEG_X, "mostly left = turn only");
    assert_eq!(snap_8way(Vec2::new(-0.5, -0.6)), Vec2::NEG_ONE);
}

#[test]
fn tapping_the_whistle_button_does_not_fire() {
    use second_person::touch::tap_fires;
    let button = Rect::new(800.0, 300.0, 884.0, 384.0);
    assert!(tap_fires(Vec2::new(700.0, 200.0), 1000.0, button), "right half fires");
    assert!(!tap_fires(Vec2::new(300.0, 200.0), 1000.0, button), "left half is the stick");
    assert!(!tap_fires(Vec2::new(840.0, 340.0), 1000.0, button), "whistle button");
}

#[test]
fn start_screen_holds_the_game_until_a_press_then_controls_follow_the_last_input() {
    use bevy::input::touch::{TouchInput, TouchPhase};
    use leafwing_input_manager::prelude::ActionState;
    use second_person::{shooter::ShooterAction, start::{self, Started}, touch::TouchControls};
    let mut app = app_with(|app| {
        app.add_plugins(start::plugin);
    });
    step(&mut app, 0.5);
    assert!(!app.world().resource::<Started>().0);
    assert!(app.world().resource::<Time<Virtual>>().is_paused(), "frozen behind the overlay");

    // The key that dismisses the overlay doesn't fire.
    KeyCode::Space.press(app.world_mut());
    step(&mut app, 0.1);
    KeyCode::Space.release(app.world_mut());
    step(&mut app, 0.1);
    assert!(app.world().resource::<Started>().0);
    assert!(!app.world().resource::<Time<Virtual>>().is_paused());
    assert_eq!(counted::<Gunshot>(&app), 0, "dismissing press fired");
    assert!(!app.world().resource::<TouchControls>().0, "a key means keyboard");

    // Now Space fires as usual.
    press(&mut app, KeyCode::Space);
    assert_eq!(counted::<Gunshot>(&app), 1);
    let shooter = single::<Shooter>(&mut app);
    assert!(!app.world().get::<ActionState<ShooterAction>>(shooter).unwrap().disabled());

    // A touch switches to touch controls, a key back to keyboard.
    app.world_mut().write_message(TouchInput {
        phase: TouchPhase::Started,
        position: Vec2::new(10.0, 10.0),
        window: Entity::PLACEHOLDER,
        force: None,
        id: 1,
    });
    app.update();
    assert!(app.world().resource::<TouchControls>().0);
    press(&mut app, KeyCode::ArrowUp);
    assert!(!app.world().resource::<TouchControls>().0);
}

/// Shooter 12m straight behind him (out of his view), turned `off_deg` away, fires once.
fn shot_off_by(off_deg: f32) -> App {
    let mut app = app();
    let shooter_pos = Vec3::new(0.0, 0.9, 12.0);
    stage(&mut app, shooter_pos, Vec3::new(0.0, 0.9, -10.0));
    let shooter = single::<Shooter>(&mut app);
    let yaw = yaw_towards(shooter_pos, Vec3::new(0.0, 0.9, 0.0)) + off_deg.to_radians();
    place(&mut app, shooter, shooter_pos, yaw);
    app.update();
    press(&mut app, KeyCode::Space);
    step(&mut app, 0.6);
    app
}

#[test]
fn bullet_magnetism_bends_a_near_miss_into_him() {
    assert_eq!(counted::<TargetHit>(&shot_off_by(8.0)), 1, "8° off should be pulled in");
    assert_eq!(counted::<TargetHit>(&shot_off_by(-8.0)), 1);
    assert_eq!(counted::<TargetHit>(&shot_off_by(25.0)), 0, "25° off is a miss");
}

#[test]
fn bullet_magnetism_does_not_bend_around_cover() {
    use second_person::combat::magnetised;
    // Crate at (-6, _, -8) between this spot and the origin.
    let muzzle = Vec3::new(-9.0, 1.0, -12.0);
    let to_him = (Vec3::new(0.0, 0.9, 0.0) - muzzle).normalize();
    let forward = Quat::from_rotation_y(8f32.to_radians()) * to_him;
    assert_eq!(magnetised(&Layout::classic(), muzzle, forward, Vec3::new(0.0, 0.9, 0.0)), forward);
    // Same angle in the open does bend.
    let open = Vec3::new(0.0, 1.0, 12.0);
    let fwd = Quat::from_rotation_y(8f32.to_radians()) * (Vec3::new(0.0, 0.9, 0.0) - open).normalize();
    assert_ne!(magnetised(&Layout::classic(), open, fwd, Vec3::new(0.0, 0.9, 0.0)), fwd);
}

/// Him at `him`, the shooter at `shooter_pos`, both on the ground plane (XZ). Engages him, then
/// runs frames while he's running for cover; `each` sees the app every frame of the run.
fn run_for_cover(him: Vec2, shooter_pos: Vec2, mut each: impl FnMut(&mut App, Entity, Entity)) {
    let (him, shooter_pos) = (Vec3::new(him.x, 0.9, him.y), Vec3::new(shooter_pos.x, 0.9, shooter_pos.y));
    let mut app = app();
    let target = single::<Target>(&mut app);
    let shooter = single::<Shooter>(&mut app);
    place(&mut app, target, him, yaw_towards(him, shooter_pos));
    place(&mut app, shooter, shooter_pos, yaw_towards(shooter_pos, him));
    app.update();
    app.world_mut().get_mut::<Shooter>(shooter).unwrap().hp = 1e6;
    {
        let mut s = app.world_mut().get_mut::<Suspicion>(target).unwrap();
        s.bump(1.0);
        s.last_known = Some(shooter_pos);
    }
    let running = |app: &App| app.world().get::<Activity>(target) == Some(&Activity::TakingCover);
    for _ in 0..(1.0 / DT) as usize {
        app.update();
        if running(&app) {
            break;
        }
    }
    assert!(running(&app), "never ran for cover from {shooter_pos}");
    for _ in 0..(8.0 / DT) as usize {
        if !running(&app) {
            break;
        }
        each(&mut app, target, shooter);
        app.update();
    }
}

/// (him, shooter): spots where the nearest cover is straight along the line of fire, away from
/// or towards the shooter, so a plain nearest-cover dash would be a shooting-gallery run.
const COVER_RUNS: [(Vec2, Vec2); 4] = [
    (Vec2::new(24.0, 0.0), Vec2::new(24.0, -12.0)),
    (Vec2::new(20.0, -8.0), Vec2::new(20.0, 4.0)),
    (Vec2::new(24.0, 4.0), Vec2::new(24.0, 16.0)),
    (Vec2::new(20.0, 8.0), Vec2::new(11.5, -0.5)),
];

#[test]
fn running_for_cover_moves_across_your_line_of_fire() {
    let (mut tangential, mut total) = (0.0, 0.0);
    for (him, shooter_pos) in COVER_RUNS {
        run_for_cover(him, shooter_pos, |app, target, _| {
            let w = app.world();
            let v = w.get::<LinearVelocity>(target).unwrap().0.xz();
            let line = (w.get::<Transform>(target).unwrap().translation.xz() - shooter_pos).normalize();
            tangential += v.perp_dot(line).abs();
            total += v.length();
        });
    }
    let share = tangential / total.max(1e-6);
    assert!(total > 0.0, "never moved");
    assert!(share > 0.4, "mostly along the line of fire: tangential share {share:.2}");
}

#[test]
fn magnetised_fire_down_the_line_misses_most_shots_on_a_cover_run() {
    let (mut shots, mut hits) = (0, 0);
    for (him, shooter_pos) in COVER_RUNS {
        let shooter_at = Vec3::new(shooter_pos.x, 0.9, shooter_pos.y);
        let mut since = 1.0;
        let (mut shots_seen, mut hits_seen) = (0, 0);
        run_for_cover(him, shooter_pos, |app, target, shooter| {
            // He can take it: we're counting hits, not ending the round.
            app.world_mut().get_mut::<Target>(target).unwrap().hp = 1000;
            // Keep the gun on him (magnetism does the rest) and fire as fast as allowed.
            let at = app.world().get::<Transform>(target).unwrap().translation;
            place(app, shooter, shooter_at, yaw_towards(shooter_at, at));
            since += DT;
            if since >= 0.3 {
                since = 0.0;
                KeyCode::Space.press(app.world_mut());
            } else {
                KeyCode::Space.release(app.world_mut());
            }
            let (s, h) = (counted::<Gunshot>(app), counted::<TargetHit>(app));
            shots += s - shots_seen;
            hits += h - hits_seen;
            (shots_seen, hits_seen) = (s, h);
        });
    }
    assert!(shots >= 8, "too few shots: {shots}");
    assert!(hits * 2 < shots, "{hits} of {shots} shots hit");
}

