//! The match server: owns both bodies, the bullets and every hit, and ends itself when nobody's
//! playing (a match machine costs money for as long as it runs).

use std::{collections::VecDeque, net::SocketAddr};

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;
use lightyear::{
    interpolation::plugin::InterpolationDelay,
    netcode::NetcodeServer,
    prelude::{server::*, *},
    websocket::prelude::server::{ServerConfig, WebSocketServerIo},
};
use rand::Rng;

use super::{
    ArenaSeed, AuthoritySet, Eyelids, Events, Health, KEY, NetEvent, PORT, PROTOCOL_ID, PlayerId, Score,
    conditioner, muzzle,
};
use crate::{
    arena::{self, ArenaMode, Layout},
    combat::{BULLET_SPEED, FIRE_COOLDOWN, HitKind, magnetised},
    difficulty::Tuning,
    round::{GameState, SpawnRound},
    shooter::{self, SHOOTER_MAX_HP, Shooter, ShooterAction, Sidestep, Stagger, Stunned},
    target::VIEW_RANGE,
};

/// Lag compensation never rewinds further than this (250 ms).
const MAX_LAG_COMP_TICKS: u32 = 15;
/// Seconds between a kill and the next round (the death plays out, then the banner).
const ROUND_BREAK: f32 = 4.0;
const BULLET_LIFE: f32 = 2.0;
/// A bullet hits a body this close to its centre line.
const HIT_RADIUS: f32 = 0.35 + 0.12;
/// Eyes stay shut at most this many ticks per press, then need this long before closing again.
const EYES_SHUT_TICKS: u32 = 90;
const EYES_REST_TICKS: u32 = 240;

#[derive(Component, Default)]
struct Gun {
    cooldown: f32,
}

/// Where this body was on recent ticks, to rewind it to what a shooter saw.
#[derive(Component, Default)]
struct History(VecDeque<(Tick, Vec3)>);

#[derive(Component, Default)]
struct ShutFor(u32);

#[derive(Component)]
struct Bullet {
    owner: PeerId,
    origin: Vec3,
    at: Vec3,
    vel: Vec3,
    lag_ticks: u32,
    life: f32,
}

/// Counts down the break between rounds.
#[derive(Resource, Default)]
struct Break(f32);

pub fn plugin(app: &mut App, lag: u64) {
    let seed = rand::rng().random_range(0..1_000_000);
    app.insert_resource(ArenaMode::Seed(seed))
        .init_resource::<Break>()
        .init_resource::<Liveness>()
        .add_systems(Startup, move |mut commands: Commands| {
            commands.spawn((ArenaSeed(seed), Replicate::to_clients(NetworkTarget::All)));
            let server = commands
                .spawn((
                    Name::new("Server"),
                    Server::new(conditioner(lag)),
                    NetcodeServer::new(lightyear::netcode::server_plugin::NetcodeConfig {
                        protocol_id: PROTOCOL_ID,
                        private_key: KEY,
                        // Behind a UDP proxy (Fly) we bind a private address while clients' tokens
                        // name the public one, so the address check would turn everyone away.
                        server_addr_check: false,
                        ..default()
                    }),
                    LocalAddr(bind_addr()),
                    // Plain ws: in production the host's proxy terminates TLS (wss) in front of it.
                    WebSocketServerIo { config: ServerConfig::builder().with_bind_address(bind_addr()).with_no_encryption() },
                ))
                .id();
            commands.trigger(Start { entity: server });
            info!("duel server listening on ws://{}", bind_addr());
        })
        .add_observer(|t: On<Add, LinkOf>, mut commands: Commands| {
            commands.entity(t.entity).insert(ReplicationSender);
        })
        .add_observer(join)
        .add_systems(
            OnEnter(GameState::Playing),
            place_players.in_set(AuthoritySet).in_set(SpawnRound).after(arena::spawn_arena),
        )
        .add_systems(
            FixedUpdate,
            (
                (record_history, eyelids, fire).after(shooter::Movement),
                (fly_bullets, report_health).chain().in_set(AuthoritySet),
            )
                .chain()
                .run_if(in_state(GameState::Playing)),
        )
        .add_systems(Update, (next_round.in_set(AuthoritySet), liveness, log_players));
}

/// Where to listen (`DUEL_BIND`, default all interfaces on `PORT`).
pub fn bind_addr() -> SocketAddr {
    let spec = std::env::var("DUEL_BIND").unwrap_or(format!("0.0.0.0:{PORT}"));
    super::resolve(&spec).unwrap_or_else(|e| panic!("DUEL_BIND: {e}"))
}

/// A clear spot, as far from `other` as this arena reasonably allows.
fn spawn_point(layout: &Layout, other: Option<Vec3>) -> Transform {
    let mut rng = rand::rng();
    let far = layout.bounds().size().length() * 0.4;
    let p = [far, far * 0.5, 0.0]
        .into_iter()
        .find_map(|min| layout.random_point(&mut rng, 1.5, |p| other.is_none_or(|o| o.xz().distance(p) > min)))
        .unwrap_or_default();
    Transform::from_xyz(p.x, 0.9, p.y).with_rotation(Quat::from_rotation_y(rng.random_range(0.0..std::f32::consts::TAU)))
}

fn join(
    t: On<Add, Connected>,
    remote: Query<&RemoteId, With<ClientOf>>,
    players: Query<&Position, With<PlayerId>>,
    layout: Res<Layout>,
    mut commands: Commands,
) {
    let Ok(id) = remote.get(t.entity) else { return };
    let id = id.0;
    if players.iter().count() >= 2 {
        warn!("{id:?} turned away: a duel is two players");
        return;
    }
    let at = spawn_point(&layout, players.iter().next().map(|p| p.0));
    info!("{id:?} joined at {}", at.translation);
    commands.spawn((
        Name::new("Player"),
        PlayerId(id),
        shooter::body(at),
        Health(SHOOTER_MAX_HP),
        Score::default(),
        (Eyelids::default(), ShutFor::default(), Gun::default(), History::default()),
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::Single(id)),
        InterpolationTarget::to_clients(NetworkTarget::AllExceptSingle(id)),
        ControlledBy { owner: t.entity, lifetime: default() },
    ));
}

/// A new round: both players to fresh, far-apart spots, full health, nothing carried over.
#[allow(clippy::type_complexity)]
fn place_players(
    layout: Res<Layout>,
    mut players: Query<
        (
            (&mut Position, &mut Rotation, &mut Transform),
            &mut LinearVelocity,
            &mut Shooter,
            (&mut Stagger, &mut Stunned, &mut Sidestep),
            &mut History,
            &mut Gun,
        ),
        With<PlayerId>,
    >,
) {
    let mut first = None;
    for ((mut pos, mut rot, mut t), mut vel, mut shooter, (mut stagger, mut stunned, mut side), mut history, mut gun) in
        &mut players
    {
        let at = spawn_point(&layout, first);
        first = Some(at.translation);
        (pos.0, rot.0, *t) = (at.translation, at.rotation, at);
        vel.0 = Vec3::ZERO;
        shooter.hp = SHOOTER_MAX_HP;
        (*stagger, *stunned, *side) = default();
        history.0.clear();
        gun.cooldown = 0.0;
    }
}

fn record_history(timeline: Res<LocalTimeline>, mut q: Query<(&Position, &mut History)>) {
    let tick = timeline.tick();
    for (pos, mut h) in &mut q {
        h.0.push_back((tick, pos.0));
        if h.0.len() > 64 {
            h.0.pop_front();
        }
    }
}

impl History {
    /// Where it was `ticks` ago (or now, if that's before the record starts).
    fn rewind(&self, now: Tick, ticks: u32, fallback: Vec3) -> Vec3 {
        let then = now - ticks;
        self.0.iter().rev().find(|(t, _)| *t <= then).map_or(fallback, |(_, p)| *p)
    }
}

/// Eyes close while the key is held, for at most `EYES_SHUT_TICKS`, then rest.
fn eyelids(mut q: Query<(&ActionState<ShooterAction>, &mut Eyelids, &mut ShutFor, &Shooter)>) {
    for (actions, mut lids, mut shut_for, shooter) in &mut q {
        let held = actions.pressed(&ShooterAction::CloseEyes) && shooter.hp > 0.0;
        let mut l = *lids;
        if l.closed {
            shut_for.0 += 1;
            if !held || shut_for.0 >= EYES_SHUT_TICKS {
                l = Eyelids { closed: false, cooldown: EYES_REST_TICKS };
            }
        } else if l.cooldown > 0 {
            l.cooldown -= 1;
        } else if held {
            l.closed = true;
            shut_for.0 = 0;
        }
        // Only replicate real changes, not every tick of the rest countdown.
        if l.closed != lids.closed || (l.cooldown == 0) != (lids.cooldown == 0) {
            *lids = l;
        } else {
            lids.bypass_change_detection().cooldown = l.cooldown;
        }
    }
}

/// Fire on the press, with single player's aim assist bending shots that are on the other player
/// as the shooter saw them.
#[allow(clippy::type_complexity)]
fn fire(
    mut commands: Commands,
    time: Res<Time>,
    timeline: Res<LocalTimeline>,
    layout: Res<Layout>,
    tuning: Res<Tuning>,
    mut shooters: Query<(
        &PlayerId,
        &ActionState<ShooterAction>,
        &Position,
        &Rotation,
        &Stunned,
        &Eyelids,
        &Shooter,
        &mut Gun,
        &ControlledBy,
    )>,
    others: Query<(&PlayerId, &Position, &History)>,
    delays: Query<&InterpolationDelay, With<ClientOf>>,
    mut sender: ServerMultiMessageSender,
    server: Single<&Server>,
) {
    let now = timeline.tick();
    for (id, actions, pos, rot, stunned, lids, shooter, mut gun, controlled) in &mut shooters {
        gun.cooldown -= time.delta_secs();
        let ready = gun.cooldown <= 0.0 && stunned.left <= 0.0 && !lids.closed && shooter.hp > 0.0;
        if !actions.just_pressed(&ShooterAction::Fire) || !ready {
            continue;
        }
        gun.cooldown = FIRE_COOLDOWN;
        // How far behind the shooter saw the world: the opponent is interpolated.
        let lag_ticks =
            delays.get(controlled.owner).map_or(0, |d| d.delay.tick_diff() as u32).min(MAX_LAG_COMP_TICKS);
        let (from, forward) = muzzle(pos.0, rot.0);
        let dir = others
            .iter()
            .find(|(other, ..)| other.0 != id.0)
            .map_or(forward, |(_, p, h)| magnetised(&layout, &tuning.help(), from, forward, h.rewind(now, lag_ticks, p.0)));
        commands.spawn(Bullet {
            owner: id.0,
            origin: pos.0,
            at: from,
            vel: dir * BULLET_SPEED,
            lag_ticks,
            life: BULLET_LIFE,
        });
        let shot = NetEvent::Shot { by: id.0, muzzle: from, dir };
        let _ = sender.send::<_, Events>(&shot, &server, &NetworkTarget::AllExceptSingle(id.0));
    }
}

/// Moves bullets, tested against where each target was in the shooter's view.
#[allow(clippy::too_many_arguments)]
fn fly_bullets(
    mut commands: Commands,
    time: Res<Time>,
    timeline: Res<LocalTimeline>,
    layout: Res<Layout>,
    tuning: Res<Tuning>,
    mut bullets: Query<(Entity, &mut Bullet)>,
    mut players: Query<(&PlayerId, &History, &Position, &Rotation, &mut Shooter, &mut Score)>,
    mut next: ResMut<NextState<GameState>>,
    mut breather: ResMut<Break>,
    mut sender: ServerMultiMessageSender,
    server: Single<&Server>,
) {
    let now = timeline.tick();
    let dt = time.delta_secs();
    let mut events = vec![];
    for (e, mut b) in &mut bullets {
        b.life -= dt;
        let from = b.at;
        let to = from + b.vel * dt;
        let mut spent = b.life <= 0.0;
        'flight: for k in 1..=4 {
            let p = from.lerp(to, k as f32 / 4.0);
            for (id, history, pos, rot, mut victim, _) in &mut players {
                if id.0 == b.owner || victim.hp <= 0.0 {
                    continue;
                }
                let there = history.rewind(now, b.lag_ticks, pos.0);
                if p.xz().distance(there.xz()) > HIT_RADIUS {
                    continue;
                }
                let facing = rot.0 * Vec3::NEG_Z;
                let to_shooter = b.origin - pos.0;
                let sees = facing.xz().angle_to(to_shooter.xz()).abs() <= tuning.him().view_half_angle
                    && to_shooter.length() <= VIEW_RANGE
                    && !layout.los_blocked(pos.0.xz(), b.origin.xz());
                let kind = HitKind::of(sees, facing, to_shooter);
                let damage = kind.damage(rand::rng().random_range(-1.0..1.0));
                victim.hp = (victim.hp - damage).max(0.0);
                info!("{:?} hit {:?} ({kind:?}, {damage:.0}, {} ticks rewound) hp {:.0}", b.owner, id.0, b.lag_ticks, victim.hp);
                events.push(NetEvent::Hit { victim: id.0, from: b.origin, at: pos.0, damage, kind });
                if victim.hp <= 0.0 {
                    events.push(NetEvent::Kill { winner: b.owner });
                }
                spent = true;
                break 'flight;
            }
            if !layout.is_clear(p.xz(), 0.05) {
                events.push(NetEvent::Impact { at: p });
                spent = true;
                break;
            }
        }
        b.at = to;
        if spent {
            commands.entity(e).despawn();
        }
    }
    for ev in events {
        if let NetEvent::Kill { winner } = ev {
            for (id, .., mut score) in &mut players {
                if id.0 == winner {
                    score.0 += 1;
                }
            }
            info!("{winner:?} wins the round");
            next.set(GameState::Won);
            breather.0 = ROUND_BREAK;
        }
        let _ = sender.send::<_, Events>(&ev, &server, &NetworkTarget::All);
    }
}

fn report_health(mut q: Query<(&Shooter, &mut Health)>) {
    for (shooter, mut health) in &mut q {
        health.set_if_neq(Health(shooter.hp));
    }
}

/// After the break, a new arena and a new round.
fn next_round(
    time: Res<Time>,
    state: Res<State<GameState>>,
    mut breather: ResMut<Break>,
    mut mode: ResMut<ArenaMode>,
    mut seed: Single<&mut ArenaSeed>,
    mut next: ResMut<NextState<GameState>>,
) {
    if *state.get() == GameState::Playing {
        return;
    }
    breather.0 -= time.delta_secs();
    if breather.0 > 0.0 {
        return;
    }
    seed.0 = rand::rng().random_range(0..1_000_000);
    *mode = ArenaMode::Seed(seed.0);
    info!("next round: arena seed {}", seed.0);
    next.set(GameState::Playing);
}

fn log_players(
    time: Res<Time>,
    mut t: Local<f32>,
    state: Res<State<GameState>>,
    q: Query<(&PlayerId, &Position, &Shooter, &Score, Option<&ActionState<ShooterAction>>)>,
) {
    *t += time.delta_secs();
    if *t < 10.0 {
        return;
    }
    *t = 0.0;
    for (id, pos, shooter, score, actions) in &q {
        let drive = actions.map(|a| a.clamped_axis_pair(&ShooterAction::Drive));
        info!("{:?} at {:.1} hp {:.0} score {} drive {drive:?} ({:?})", id.0, pos.0, shooter.hp, score.0, state.get());
    }
}

// ---------------------------------------------------------------------------
// Liveness: exiting the process stops the machine (restart policy "never"). Each limit can be
// overridden with an env var.
// ---------------------------------------------------------------------------

fn env_secs(name: &str, default: f32) -> f32 {
    std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[derive(Resource)]
struct Liveness {
    /// Waiting for a second player (at start, or after one left).
    wait_for_players: f32,
    /// Both in, nobody touching anything.
    afk: f32,
    /// Everyone left: a short grace for reconnects.
    empty: f32,
    /// A ceiling whatever happens, in case another check has a bug.
    max_match: f32,
    last_full: f32,
    last_input: f32,
    last_connected: Option<f32>,
}

impl Default for Liveness {
    fn default() -> Self {
        Self {
            wait_for_players: env_secs("MATCH_WAIT_S", 120.0),
            afk: env_secs("MATCH_AFK_S", 60.0),
            empty: env_secs("MATCH_EMPTY_S", 10.0),
            max_match: env_secs("MATCH_MAX_S", 900.0),
            last_full: 0.0,
            last_input: 0.0,
            last_connected: None,
        }
    }
}

fn liveness(
    time: Res<Time<Real>>,
    mut l: ResMut<Liveness>,
    clients: Query<(), (With<ClientOf>, With<Connected>)>,
    players: Query<&ActionState<ShooterAction>, With<PlayerId>>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    let connected = clients.iter().count();
    if connected > 0 {
        l.last_connected = Some(now);
    }
    // The AFK clock only runs while both players are in.
    let touched = |a: &ActionState<ShooterAction>| {
        a.get_pressed().len() > 0 || a.clamped_axis_pair(&ShooterAction::Drive) != Vec2::ZERO
    };
    if connected < 2 || players.iter().any(touched) {
        l.last_input = now;
    }
    if connected >= 2 {
        l.last_full = now;
    }
    let reason = if l.last_connected.is_some_and(|t| now - t > l.empty) {
        Some("everyone left")
    } else if now - l.last_full > l.wait_for_players {
        Some("no opponent")
    } else if now - l.last_input > l.afk {
        Some("nobody is playing")
    } else if now > l.max_match {
        Some("match hit the time limit")
    } else {
        None
    };
    if let Some(reason) = reason {
        info!("LIVENESS: shutting down after {now:.0}s: {reason}");
        exit.write(AppExit::Success);
    }
}
