//! Round lifecycle: playing / won / lost, restart, and global (non-entity) input.

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;

use crate::{arena::ArenaMode, radar::RadarMode, shooter::ControlScheme};

#[derive(States, Default, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GameState {
    #[default]
    Playing,
    Won,
    Lost,
}

/// Marks everything that belongs to a single round; despawned on restart.
#[derive(Component, Default)]
pub struct RoundEntity;

/// Inputs that aren't tied to the shooter.
#[derive(Actionlike, PartialEq, Eq, Clone, Copy, Hash, Debug, Reflect)]
pub enum MetaAction {
    Restart,
    ToggleTargetMobility,
    CycleRadar,
    /// Switch between the classic and generated arenas (starts a new round).
    CycleArena,
    /// Try the next `ControlScheme`.
    CycleControls,
}

/// Whether the target is allowed to walk around (vs. standing still and looking around).
#[derive(Resource, Default)]
pub struct TargetMobile(pub bool);

/// A demo round with no shooter in it: him going about his business behind the title screen.
#[derive(Resource, Default)]
pub struct Attract(pub bool);

/// A duel server runs the rounds: the local restart, arena and control keys are off, and nothing
/// spawns the single-player cast (him, or a shooter you control locally).
#[derive(Resource)]
pub struct Online;

/// Systems that spawn round entities run in this set on `OnEnter(Playing)`, after cleanup.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct SpawnRound;

pub fn plugin(app: &mut App) {
    app.init_state::<GameState>()
        .init_resource::<TargetMobile>()
        .init_resource::<Attract>()
        // Difficulty setting; lives here (not in radar) so headless gameplay has it too.
        .init_resource::<RadarMode>()
        .add_plugins(InputManagerPlugin::<MetaAction>::default())
        .add_systems(Startup, |mut commands: Commands| {
            // leafwing 0.21: action state lives on an entity, not a resource.
            commands.spawn((
                Name::new("Meta input"),
                InputMap::new([
                    (MetaAction::Restart, KeyCode::KeyR),
                    (MetaAction::ToggleTargetMobility, KeyCode::KeyM),
                    (MetaAction::CycleRadar, KeyCode::Tab),
                    (MetaAction::CycleArena, KeyCode::KeyL),
                    (MetaAction::CycleControls, KeyCode::KeyC),
                ]),
            ));
        })
        .add_systems(OnEnter(GameState::Playing), (cleanup_round, resume_physics).before(SpawnRound))
        .add_systems(OnExit(GameState::Playing), pause_physics)
        .add_systems(Update, meta_input);
}

fn cleanup_round(mut commands: Commands, q: Query<Entity, With<RoundEntity>>) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

fn pause_physics(mut time: ResMut<Time<Physics>>) {
    time.pause();
}

fn resume_physics(mut time: ResMut<Time<Physics>>) {
    time.unpause();
}

fn meta_input(
    actions: Single<&ActionState<MetaAction>>,
    state: Res<State<GameState>>,
    mut next: ResMut<NextState<GameState>>,
    mut mobile: ResMut<TargetMobile>,
    mut radar: ResMut<RadarMode>,
    mut arena: ResMut<ArenaMode>,
    mut controls: ResMut<ControlScheme>,
    online: Option<Res<Online>>,
) {
    if actions.just_pressed(&MetaAction::CycleRadar) {
        *radar = radar.next();
    }
    if online.is_some() {
        return;
    }
    if actions.just_pressed(&MetaAction::CycleControls) {
        *controls = controls.next();
    }
    if actions.just_pressed(&MetaAction::CycleArena) {
        *arena = arena.next();
        // `set` re-enters Playing even mid-round, which rebuilds the arena.
        next.set(GameState::Playing);
    }
    if actions.just_pressed(&MetaAction::Restart) && *state.get() != GameState::Playing {
        next.set(GameState::Playing);
    }
    if actions.just_pressed(&MetaAction::ToggleTargetMobility) {
        mobile.0 = !mobile.0;
    }
}
