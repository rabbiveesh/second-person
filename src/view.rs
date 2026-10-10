//! Whose eyes you see through when there are several targets (a spike for comparing rules).
//!
//! `ViewRule` is cycled with V (starts a new round):
//! - `Single`: one target, the original game.
//! - `HopOnKill`: three targets; when the one you're in dies, you jump to the nearest survivor.
//! - `Switch`: as `HopOnKill`, and Q cycles through the survivors.
//! - `Threat`: as `HopOnKill`, but the view follows whoever is most suspicious of you.
//!
//! Switching just moves the `Viewed` marker and reparents the one `MainCamera` onto the new
//! target's head, so audio, HUD and everything keyed off the camera follow along.

use bevy::prelude::*;
use leafwing_input_manager::prelude::*;

use crate::{
    round::{GameState, MetaAction},
    target::{Dead, MainCamera, Suspicion, Target, TargetHead, Viewed},
};

/// Q can't flick between views faster than this.
const SWITCH_COOLDOWN: f32 = 0.6;
/// Threat cam: how much more suspicious another target must be to steal the view...
const THREAT_MARGIN: f32 = 0.15;
/// ...and how long the view stays put after a switch.
const THREAT_DWELL: f32 = 1.0;

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug, Reflect)]
#[reflect(Resource)]
pub enum ViewRule {
    #[default]
    Single,
    HopOnKill,
    Switch,
    Threat,
}

impl ViewRule {
    pub fn next(self) -> Self {
        match self {
            Self::Single => Self::HopOnKill,
            Self::HopOnKill => Self::Switch,
            Self::Switch => Self::Threat,
            Self::Threat => Self::Single,
        }
    }

    pub fn target_count(self) -> usize {
        if self == Self::Single { 1 } else { 3 }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Single => "one target",
            Self::HopOnKill => "hop on kill",
            Self::Switch => "Q to switch",
            Self::Threat => "threat cam",
        }
    }
}

/// Sent whenever your view moves to another target.
#[derive(Message, Clone, Copy, Debug)]
pub struct ViewSwitched {
    pub from: Entity,
    pub to: Entity,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<ViewRule>()
        .add_message::<ViewSwitched>()
        .add_systems(Update, choose_view.run_if(in_state(GameState::Playing)));
}

#[allow(clippy::too_many_arguments)]
fn choose_view(
    mut commands: Commands,
    time: Res<Time>,
    rule: Res<ViewRule>,
    mut since_switch: Local<f32>,
    actions: Single<&ActionState<MetaAction>>,
    viewed: Option<Single<(Entity, &Transform, &Suspicion, Has<Dead>), (With<Target>, With<Viewed>)>>,
    alive: Query<(Entity, &Transform, &Suspicion), (With<Target>, Without<Dead>)>,
    heads: Query<(Entity, &ChildOf), With<TargetHead>>,
    camera: Single<Entity, With<MainCamera>>,
    mut switched: MessageWriter<ViewSwitched>,
) {
    *since_switch += time.delta_secs();
    let Some(viewed) = viewed else { return };
    let (current, here, suspicion, dead) = *viewed;

    let next = if dead {
        // Hop to the nearest survivor (if any; otherwise the round is over).
        alive
            .iter()
            .min_by(|a, b| {
                let (da, db) = (a.1.translation.distance(here.translation), b.1.translation.distance(here.translation));
                da.total_cmp(&db)
            })
            .map(|(e, ..)| e)
    } else {
        match *rule {
            ViewRule::Switch
                if actions.just_pressed(&MetaAction::SwitchView) && *since_switch >= SWITCH_COOLDOWN =>
            {
                // Next survivor after the current one, in a stable order.
                let mut order: Vec<Entity> = alive.iter().map(|(e, ..)| e).collect();
                order.sort();
                let i = order.iter().position(|&e| e == current).unwrap_or(0);
                order.get((i + 1) % order.len().max(1)).copied().filter(|&e| e != current)
            }
            ViewRule::Threat if *since_switch >= THREAT_DWELL => alive
                .iter()
                .filter(|(_, _, s)| s.level > suspicion.level + THREAT_MARGIN)
                .max_by(|a, b| a.2.level.total_cmp(&b.2.level))
                .map(|(e, ..)| e),
            _ => None,
        }
    };

    let Some(next) = next else { return };
    let Some((head, _)) = heads.iter().find(|(_, parent)| parent.parent() == next) else { return };
    commands.entity(current).remove::<Viewed>();
    commands.entity(next).insert(Viewed);
    commands.entity(*camera).insert(ChildOf(head));
    *since_switch = 0.0;
    switched.write(ViewSwitched { from: current, to: next });
}
