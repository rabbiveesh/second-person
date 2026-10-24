//! A player's side of the duel. Your body is a `Shooter` you drive (predicted, so it answers at
//! once); the opponent's body is dressed as him, `Target` and eyes included, so the single-player
//! view, HUD, radar, fx and audio all work as they are. The server's word arrives as the game's
//! own messages (`Gunshot`, `TargetHit`, `ShooterHit`, `BulletImpact`) and round states.

use std::net::SocketAddr;

use avian3d::prelude::*;
use bevy::{camera::visibility::RenderLayers, prelude::*};
use leafwing_input_manager::prelude::*;
use lightyear::{
    netcode::{NetcodeClient, client_plugin::NetcodeConfig},
    prelude::{client::*, input::client::InputSystems, *},
    websocket::prelude::client::{ClientConfig, WebSocketClientIo},
};
use rand::Rng;

use super::{
    ArenaSeed, Eyelids, FollowServerSet, Health, KEY, NetEvent, PlayerId, conditioner, muzzle, protocol_id,
};
use crate::{
    arena::{ArenaMode, Layout},
    combat::{BULLET_SPEED, BulletImpact, BulletLook, FIRE_COOLDOWN, Gunshot, ShooterHit, TargetHit, magnetised},
    difficulty::Tuning,
    hud::Notes,
    radar::{RADAR_LAYER, RadarCamera},
    round::GameState,
    shooter::{self, SHOOTER_MAX_HP, Shooter, ShooterAction, Sidestep, Stagger, Stunned},
    target::{self, MainCamera, Suspicion, Target},
};

/// How a client plays: who it is, the server's WebSocket URL, and whether a bot drives.
#[derive(Clone, Debug)]
pub struct Options {
    pub id: u64,
    pub url: String,
    pub lag: u64,
    pub bot: bool,
}

/// A render layer nothing is on: what you see with the other player's eyes shut.
const BLIND_LAYER: usize = 7;
const COSMETIC_LIFE: f32 = 1.5;
/// Seconds without a connection before saying so.
const NO_SERVER_AFTER: f32 = 5.0;

/// A bullet drawn here; the server's own decides what it hits.
#[derive(Component)]
struct Cosmetic {
    vel: Vec3,
    life: f32,
}

#[derive(Resource)]
struct Bot;

pub fn plugin(app: &mut App, opts: Options) {
    // Not added by ClientPlugins: without it, rollback never runs.
    app.insert_resource(PredictionManager::default());
    if opts.bot {
        app.insert_resource(Bot)
            .add_systems(FixedPreUpdate, bot.in_set(InputSystems::WriteClientInputs));
    }
    let Options { id, url, lag, .. } = opts;
    app.add_systems(Startup, move |mut commands: Commands, protocol: Res<bevy_replicon::shared::protocol::ProtocolHash>| {
        // The WebSocket URL says where the server is; the server doesn't check the token's address
        // (behind a proxy it can't know its public one), so any address will do here.
        let nowhere = SocketAddr::from(([127, 0, 0, 1], super::PORT));
        let auth = Authentication::Manual { server_addr: nowhere, client_id: id, private_key: KEY, protocol_id: protocol_id(&protocol) };
        #[cfg(not(target_family = "wasm"))]
        let config = ClientConfig::builder().with_native_certs();
        #[cfg(target_family = "wasm")]
        let config = ClientConfig::default();
        let client = commands
            .spawn((
                Name::new("Client"),
                Client,
                ReplicationReceiver,
                Link::default().with_conditioner(conditioner(lag)),
                PeerAddr(nowhere),
                NetcodeClient::new(auth, NetcodeConfig { client_timeout_secs: 3, token_expire_secs: -1, ..default() })
                    .expect("netcode client"),
                WebSocketClientIo::from_url(config, url.clone()),
            ))
            .id();
        commands.trigger(Connect { entity: client });
        info!("player {id} connecting to {url}");
    })
    .add_systems(PreUpdate, follow_arena.in_set(FollowServerSet).after(ReplicationSystems::Receive))
    .add_systems(
        Update,
        (dress_me, dress_them, follow_health, receive, fly_cosmetics, blindness),
    )
    .add_systems(
        FixedUpdate,
        fire_locally
            .after(shooter::Movement)
            .run_if(not(is_in_rollback))
            .run_if(in_state(GameState::Playing)),
    )
    .configure_sets(FixedUpdate, shooter::Noise.run_if(not(is_in_rollback)));
}

/// Your body: the shooter you drive, predicted.
#[allow(clippy::type_complexity)]
fn dress_me(
    mut commands: Commands,
    q: Query<(Entity, &Position, &Rotation), (With<PlayerId>, With<Predicted>, Without<Shooter>)>,
    bot: Option<Res<Bot>>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    for (e, pos, rot) in &q {
        let input = if bot.is_some() {
            // The bot writes the actions itself; an empty map still marks the body as ours.
            InputMap::default()
        } else {
            shooter::controls().with(ShooterAction::CloseEyes, KeyCode::ShiftLeft).with(ShooterAction::CloseEyes, KeyCode::ShiftRight)
        };
        let mut me = commands.entity(e);
        me.insert((
            Name::new("You"),
            Shooter { hp: SHOOTER_MAX_HP },
            shooter::physics(),
            input,
            Transform::from_translation(pos.0).with_rotation(rot.0),
            Visibility::default(),
        ))
        .insert_if_new((Stagger::default(), Stunned::default(), Sidestep::default()));
        if let (Some(meshes), Some(materials)) = (meshes.as_mut(), materials.as_mut()) {
            me.insert(shooter::looks(meshes, materials));
        }
        info!("in the arena");
    }
}

/// The other player, dressed as him: you see through their eyes.
#[allow(clippy::type_complexity)]
fn dress_them(
    mut commands: Commands,
    q: Query<(Entity, &Position, &Rotation), (With<PlayerId>, With<Interpolated>, Without<Target>)>,
    tuning: Res<Tuning>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    for (e, pos, rot) in &q {
        let mut them = commands.entity(e);
        them.insert((
            Name::new("Them"),
            Target { hp: SHOOTER_MAX_HP, max_hp: SHOOTER_MAX_HP, braced_until: 0.0 },
            Suspicion::default(),
            Transform::from_translation(pos.0).with_rotation(rot.0),
        ));
        if let (Some(meshes), Some(materials)) = (meshes.as_mut(), materials.as_mut()) {
            them.insert(target::looks(meshes, materials, tuning.him().view_half_angle));
        }
        info!("opponent arrived");
    }
}

fn follow_health(
    mut me: Query<(&Health, &mut Shooter), Changed<Health>>,
    mut them: Query<(&Health, &mut Target), Changed<Health>>,
) {
    for (health, mut shooter) in &mut me {
        shooter.hp = health.0;
    }
    for (health, mut target) in &mut them {
        target.hp = health.0;
    }
}

/// Rebuild the arena the server announced: the game's own arena plugin builds it on entering
/// `Playing`, from the mode.
fn follow_arena(
    seed: Query<&ArenaSeed, Changed<ArenaSeed>>,
    mut mode: ResMut<ArenaMode>,
    mut next: ResMut<NextState<GameState>>,
    mut current: Local<Option<u64>>,
) {
    let Ok(&ArenaSeed(seed)) = seed.single() else { return };
    // Replication can touch the component without changing it.
    if *current == Some(seed) {
        return;
    }
    *current = Some(seed);
    info!("next arena: seed {seed}");
    *mode = ArenaMode::Seed(seed);
    next.set(GameState::Playing);
}

/// The server's events, as the game's own messages.
#[allow(clippy::too_many_arguments)]
fn receive(
    mut commands: Commands,
    mut rx: Query<&mut MessageReceiver<NetEvent>>,
    me: Query<(&PlayerId, &Transform), With<Shooter>>,
    look: Option<Res<BulletLook>>,
    mut next: ResMut<NextState<GameState>>,
    (mut gunshots, mut impacts, mut hurt, mut hits): (
        MessageWriter<Gunshot>,
        MessageWriter<BulletImpact>,
        MessageWriter<ShooterHit>,
        MessageWriter<TargetHit>,
    ),
) {
    let mine = me.single().ok();
    for mut r in &mut rx {
        for ev in r.receive() {
            match ev {
                NetEvent::Shot { muzzle, dir, .. } => {
                    gunshots.write(Gunshot { muzzle, dir });
                    cosmetic(&mut commands, look.as_deref(), muzzle, dir);
                }
                NetEvent::Impact { at } => {
                    impacts.write(BulletImpact { at });
                }
                NetEvent::Hit { victim, from, at, damage, kind } => match mine {
                    Some((id, t)) if id.0 == victim => {
                        hurt.write(ShooterHit { from, to: t.translation });
                    }
                    _ => {
                        let dir = (at - from).normalize_or_zero();
                        hits.write(TargetHit { at, dir, damage, kind, execution: false });
                    }
                },
                NetEvent::Kill { winner } => {
                    let won = mine.is_some_and(|(id, _)| id.0 == winner);
                    info!("{}", if won { "round won" } else { "round lost" });
                    next.set(if won { GameState::Won } else { GameState::Lost });
                }
            }
        }
    }
}

fn cosmetic(commands: &mut Commands, look: Option<&BulletLook>, from: Vec3, dir: Vec3) {
    let mut bullet = commands.spawn((
        Name::new("Bullet"),
        Cosmetic { vel: dir * BULLET_SPEED, life: COSMETIC_LIFE },
        Transform::from_translation(from),
    ));
    if let Some(look) = look {
        bullet.insert((Mesh3d(look.mesh.clone()), MeshMaterial3d(look.material.clone())));
    }
}

/// Your own shots show the instant you fire, aimed the way the server will aim them: with the
/// assist, at the opponent where you see them.
#[allow(clippy::too_many_arguments)]
fn fire_locally(
    mut commands: Commands,
    time: Res<Time>,
    mut cooldown: Local<f32>,
    layout: Res<Layout>,
    tuning: Res<Tuning>,
    look: Option<Res<BulletLook>>,
    me: Query<(&ActionState<ShooterAction>, &Position, &Rotation, &Stunned, Option<&Eyelids>, &Shooter)>,
    them: Query<&Position, (With<Target>, Without<Shooter>)>,
    mut gunshots: MessageWriter<Gunshot>,
) {
    *cooldown -= time.delta_secs();
    let Ok((actions, pos, rot, stunned, lids, shooter)) = me.single() else { return };
    let ready = *cooldown <= 0.0 && stunned.left <= 0.0 && !lids.is_some_and(|l| l.closed) && shooter.hp > 0.0;
    if !actions.just_pressed(&ShooterAction::Fire) || !ready {
        return;
    }
    *cooldown = FIRE_COOLDOWN;
    let (from, forward) = muzzle(pos.0, rot.0);
    let dir = them.single().map_or(forward, |t| magnetised(&layout, &tuning.help(), from, forward, t.0));
    gunshots.write(Gunshot { muzzle: from, dir });
    cosmetic(&mut commands, look.as_deref(), from, dir);
}

fn fly_cosmetics(
    mut commands: Commands,
    time: Res<Time>,
    layout: Res<Layout>,
    mut q: Query<(Entity, &mut Cosmetic, &mut Transform)>,
    bodies: Query<&Position, With<PlayerId>>,
) {
    let dt = time.delta_secs();
    for (e, mut b, mut t) in &mut q {
        b.life -= dt;
        t.translation += b.vel * dt;
        let at = t.translation;
        // Gone at a wall or a body (the server reports what it really hit).
        let into_body = bodies.iter().any(|p| p.0.xz().distance(at.xz()) < 0.4 && b.life < COSMETIC_LIFE - 0.05);
        if b.life <= 0.0 || !layout.is_clear(at.xz(), 0.05) || into_body {
            commands.entity(e).despawn();
        }
    }
}

/// Their eyes shut: your view (and radar) go black. Yours shut: they're blind, and you can't shoot.
#[allow(clippy::type_complexity)]
fn blindness(
    mut commands: Commands,
    them: Query<&Eyelids, With<Target>>,
    me: Query<&Eyelids, With<Shooter>>,
    mut eyes: Query<(Entity, &mut Camera), (With<MainCamera>, Without<RadarCamera>)>,
    mut radar: Query<(Entity, &mut Camera), With<RadarCamera>>,
    mut radar_clear: Local<Option<ClearColorConfig>>,
    mut notes: ResMut<Notes>,
    (link, time): (Query<Has<Connected>, With<Client>>, Res<Time<Real>>),
) {
    let blind = them.single().is_ok_and(|l| l.closed);
    for (e, mut camera) in &mut eyes {
        commands.entity(e).insert(RenderLayers::layer(if blind { BLIND_LAYER } else { 0 }));
        camera.clear_color = if blind { ClearColorConfig::Custom(Color::BLACK) } else { ClearColorConfig::Default };
    }
    for (e, mut camera) in &mut radar {
        let normal = *radar_clear.get_or_insert(camera.clear_color);
        commands.entity(e).insert(RenderLayers::layer(if blind { BLIND_LAYER } else { RADAR_LAYER }));
        camera.clear_color = if blind { ClearColorConfig::Custom(Color::BLACK) } else { normal };
    }
    let mut lines = vec![];
    // A server built from another version of the game drops our connect requests (see
    // `protocol_id`), which looks just like an unreachable one.
    if time.elapsed_secs() > NO_SERVER_AFTER && link.single().is_ok_and(|connected| !connected) {
        lines.push("Can't join: the duel server is down, or on a different version of the game".to_string());
    }
    if blind {
        lines.push("THEY'VE SHUT THEIR EYES: you're blind".to_string());
    }
    match me.single() {
        Ok(l) if l.closed => lines.push("Your eyes are shut: they're blind, you can't shoot".into()),
        Ok(l) if l.cooldown > 0 => lines.push("Your eyes are resting".into()),
        _ => {}
    }
    if notes.0 != lines {
        notes.0 = lines;
    }
}

/// What the bot remembers between ticks.
#[derive(Default)]
struct BotMind {
    /// Where it last saw them: it loses track while their eyes are shut.
    last_seen: Option<Vec3>,
    shut_for: u32,
    aim_error: f32,
    retarget: u32,
    /// Out of sight: a spot that sees both it and them, and how long it gives itself to get there.
    waypoint: Option<(Vec2, u32)>,
    /// Backing off a wall: ticks left, and which way it turns.
    escape: Option<(u32, f32)>,
}

/// A bot for self-play and tests: it sees what any client sees (it has to, to draw the other
/// player's eyes), and plays roughly fair. When they shut their eyes it keeps acting on where it
/// last saw them, and it shuts its own when they line up a shot.
#[allow(clippy::type_complexity)]
fn bot(
    layout: Res<Layout>,
    mut me: Query<(&mut ActionState<ShooterAction>, &Position, &Rotation, Option<&Eyelids>), With<Shooter>>,
    them: Query<(&Position, &Rotation, Option<&Eyelids>), With<Target>>,
    mut mind: Local<BotMind>,
) {
    let Ok((mut actions, pos, rot, lids)) = me.single_mut() else { return };
    let Ok((their_pos, their_rot, their_lids)) = them.single() else { return };
    let mind = &mut *mind;
    let mut rng = rand::rng();
    if !their_lids.is_some_and(|l| l.closed) || mind.last_seen.is_none() {
        mind.last_seen = Some(their_pos.0);
    }
    let target = mind.last_seen.unwrap();
    let here = pos.0;
    let facing = rot.0 * Vec3::NEG_Z;
    // They're lined up on us: sometimes shut our eyes (blinding them) and move.
    let to_me = here - their_pos.0;
    let lined_up = (their_rot.0 * Vec3::NEG_Z).angle_between(to_me.with_y(0.0)) < 0.3
        && !layout.los_blocked(here.xz(), their_pos.0.xz());
    if mind.shut_for == 0 && lined_up && lids.is_some_and(|l| !l.closed && l.cooldown == 0) && rng.random_bool(0.04) {
        mind.shut_for = rng.random_range(40..90);
    }
    let eyes_shut = mind.shut_for > 0;
    mind.shut_for = mind.shut_for.saturating_sub(1);
    if mind.retarget == 0 {
        mind.aim_error = rng.random_range(-0.12..0.12);
        mind.retarget = rng.random_range(20..60);
    }
    mind.retarget -= 1;

    let to = (target - here).with_y(0.0);
    let clear_shot = !layout.los_blocked(here.xz(), target.xz());
    // In sight: face them (a little off, like a person). Out of sight: head for a spot that sees them.
    let goal = if clear_shot {
        mind.waypoint = None;
        Quat::from_rotation_y(mind.aim_error) * to
    } else {
        let stale = mind.waypoint.is_none_or(|(p, ticks)| ticks == 0 || p.distance(here.xz()) < 1.0);
        if stale {
            let spot = layout
                .random_point(&mut rng, 0.6, |p| !layout.los_blocked(here.xz(), p) && !layout.los_blocked(p, target.xz()))
                .or_else(|| layout.random_point(&mut rng, 0.6, |p| !layout.los_blocked(here.xz(), p)));
            mind.waypoint = spot.map(|p| (p, 240));
        }
        if let Some((_, ticks)) = &mut mind.waypoint {
            *ticks = ticks.saturating_sub(1);
        }
        mind.waypoint.map_or(to, |(p, _)| (p - here.xz()).extend(0.0).xzy())
    };
    let err = facing.with_y(0.0).angle_between(goal);
    let left = facing.cross(goal).y > 0.0;
    let mut turn = if err > 0.04 { if left { -1.0 } else { 1.0 } } else { 0.0 };
    let mut throttle = match () {
        _ if !clear_shot => if err < 0.8 { 1.0 } else { 0.2 },
        _ if to.length() > 9.0 => 1.0,
        _ if to.length() < 5.0 => -1.0,
        _ => 0.0,
    };
    if mind.escape.is_none() && throttle > 0.0 && !layout.is_clear(here.xz() + facing.xz() * 1.0, 0.35) {
        mind.escape = Some((40, if rng.random_bool(0.5) { 1.0 } else { -1.0 }));
    }
    if let Some((left, side)) = &mut mind.escape {
        turn = *side;
        throttle = if *left > 20 { -1.0 } else { 0.0 };
        *left -= 1;
        if *left == 0 {
            mind.escape = None;
        }
    }
    if eyes_shut {
        // Break line of sight instead of standing there.
        turn = 1.0;
        throttle = 1.0;
    }
    actions.set_axis_pair(&ShooterAction::Drive, Vec2::new(turn, throttle));
    if eyes_shut {
        actions.press(&ShooterAction::CloseEyes);
    } else {
        actions.release(&ShooterAction::CloseEyes);
    }
    let shoot = !eyes_shut && clear_shot && err < 0.15 && to.length() < 35.0;
    // Fire is on the press, so let go in between.
    if shoot && !actions.pressed(&ShooterAction::Fire) {
        actions.press(&ShooterAction::Fire);
    } else {
        actions.release(&ShooterAction::Fire);
    }
}
