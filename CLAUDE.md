# Second Person Shooter

You see the world through the **target's** eyes (the main camera lives in his head) while
controlling the **shooter** hunting him. A corner radar shows the whole arena.

## Ground rules
- **Use frameworks at every opportunity.** Don't hand-roll what a maintained crate does.
- **Rust everywhere.**
- Versions are pinned to **Bevy 0.19.1**. Model knowledge of Bevy APIs is often stale, so before
  using an API, grep the crate source/examples in `~/.cargo/registry/src/*/<crate>-<ver>/`.
  Read the crate's RELEASES/CHANGELOG when a pattern doesn't compile.
- Loop: `cargo build` (no warnings), then `cargo run` and check logs for panics/`WARN`.
  Screenshot the window with `import -window $(xdotool search --name "Second Person Shooter")`.

## Stack (and why)
| Crate | Role |
|---|---|
| bevy 0.19 | engine / ECS / rendering / multi-camera viewports |
| avian3d 0.7 | physics, collision events, raycasts (line of sight) |
| leafwing-input-manager 0.21 | input → actions. 0.21 has **no resource ActionState**; put global inputs on an entity |
| bevior_tree 0.11 (`default-features = false`, serde feature needs typetag impls) | target AI behaviour tree. Picked over big-brain, which is stuck on Bevy 0.15; vendor big-brain if bevior_tree hurts |
| bevy_egui 0.40 + bevy-inspector-egui 0.37 | HUD + F1 world inspector (inspector pins egui 0.40) |
| rand 0.9 | randomness |

Dev builds: `dev` feature = `bevy/dynamic_linking`; mold via `.cargo/config.toml`; deps at opt-level 3.
System deps (Ubuntu): `libudev-dev`, `libasound2-dev`, `libwayland-dev`, `libxkbcommon-dev`.

Considered alternatives: Godot+gdext (editor, but FFI borrow friction and scenes in .tscn),
Fyrox (small ecosystem), macroquad/three-d (too thin, would mean rolling our own).

## Shape (`src/`)
- `main.rs`: plugin wiring, `Layer` physics layers (World/Shooter/Target/Bullet).
- `round.rs`: `GameState` (Playing/Won/Lost). Per-round entities get `RoundEntity` and are
  despawned on `OnEnter(Playing)` before the `SpawnRound` set. Physics pauses outside Playing.
  `MetaAction` (R restart, M toggle target walking). `TargetMobile` resource.
- `arena.rs`: static ground, walls, crates and pillars (hand-placed, reproducible).
- `target.rs`: target entity → Head (pitch) → `MainCamera`. Behaviour tree:
  `Selector[engaged→Engage, alerted→Investigate, mobile→Wander, Scan]`.
  Tasks only set intent (`LookGoal`, `WanderTo`, `Activity`). Systems `perceive` → `gaze` → `walk`
  do the work. Lower branches **fail** when a higher-priority condition appears (that's preemption).
  `Suspicion` fills while the shooter is in the view cone with line of sight; at 1.0 he's engaged.
  The engine supports a moving target: the camera is parented to him, and walking is just velocity on a kinematic body.
- `shooter.rs`: dynamic capsule, rotation locked, tank controls (arrows), relative to its own facing.
- `combat.rs`: bullets (CCD, collision events), hearing (shots and near misses raise `Alert` +
  suspicion), target hitscan return fire while engaged, win/lose check.
- `radar.rs`: ortho top-down camera in a bottom-right viewport. Sees render layers 0+1; blips and
  the view cone live on `RADAR_LAYER` (1) only, so the main camera never sees them.
- `hud.rs`: egui on a dedicated `Camera2d` overlay (`PrimaryEguiContext`; auto-context disabled
  because game cameras respawn each round). Bars, activity, flashes, radar frame, end banner.

## Tuning knobs
Suspicion rates in `target::perceive`. `VIEW_HALF_ANGLE`/`VIEW_RANGE` in `target.rs`. Return fire
damage/interval, hearing range and near-miss range in `combat.rs`.
