# Second Person Shooter, in Godot + C++

The same game as the Bevy version at the repo root, rewritten for **Godot 4.6** with all game
logic in **C++** as a GDExtension (a shared library the engine loads). Scenes (`scenes/*.tscn`)
hold the layout: arena blocks, actor bodies, HUD controls. The Bevy game and the Babylon port
are untouched.

```sh
git submodule update --init                     # godot-cpp + BehaviorTree.CPP
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build                             # -> bin/libsecond_person.so (first build ~5 min)
godot --path . --import                         # once, or just open the project in the editor
godot --path .                                  # play (or press F5 in the editor)
godot --headless --path . --fixed-fps 60 -s res://tests/gameplay.gd   # tests (~2s)
```

Use a Godot 4.6.x editor from godotengine.org. On Windows/macOS the CMake build produces a
`.dll`/`.dylib`, already listed in `second_person.gdextension`. Rebuilding the library while the
editor is open hot-reloads it.

Controls are the same: arrows to drive, Space to fire, Tab cycles the radar, M lets the target
walk, R restarts after a round ends.

## What maps to what

| Bevy version | Godot version |
|---|---|
| ECS: entities + components + systems | Node tree. Each actor is one node class (`Shooter : CharacterBody3D`, `Target : CharacterBody3D`) with `_physics_process`, plus child nodes for its parts |
| Hand-placed arena in `arena.rs` | `scenes/arena.tscn`: StaticBody3D blocks you can drag around in the editor |
| `gameplay` / `presentation` plugin groups | `Game`, `Arena`, `Shooter`, `Target` vs `Radar`, `Hud`, `Fx`, `Audio` nodes in `main.tscn` |
| `Message`s (`Gunshot`, `TargetHit`, …) | Signals on the `Game` node; presentation nodes `connect` to them |
| Queries (`Query<&Target>`) | Groups (`get_first_node_in_group("shooter")`) and `%UniqueName` / path lookups |
| avian3d | Jolt (built into Godot 4.4+): CharacterBody3D actors, RigidBody3D bullets with CCD, `intersect_ray` for line of sight |
| vleue_navigator | NavigationRegion3D baked from the arena's colliders, `NavigationServer3D::map_get_path` |
| bevior_tree | BehaviorTree.CPP 4.10, tree in XML. `ReactiveFallback`/`ReactiveSequence` give preemption |
| leafwing-input-manager | Input map in `project.godot` (`Input::is_action_just_pressed("fire")`) |
| bevy_firework (vendored) | CPUParticles3D |
| bevy_kira_audio | AudioStreamPlayer3D; the current camera (his eyes) is the listener |
| bevy_egui | Control nodes in `hud.tscn` (containers, StyleBoxes), updated from `Hud::_process` |
| `RenderLayers` + per-camera light | Render layer bits + `cull_mask`; the radar is a SubViewport with its own camera and an unshaded flat map |
| `States` + `RoundEntity` despawn | Everything per-round is a child of the `Round` node; restart frees its children |
| Fixed 60 Hz test clock | `--fixed-fps 60`; tests `await physics_frame` |

## Lines of code

| | Godot | Babylon | Bevy |
|---|---|---|---|
| Game code | 1,668 C++ | 1,547 TS | 1,891 Rust |
| target AI | 549 | 333 | 551 |
| combat + rounds (`game`) | 255 | 120 combat + 192 glue | 187 combat + 89 round |
| radar, HUD, fx, audio | 504 | 538 (160 + 199 + 134 + 45) | 651 (227 + 163 + 176 + 85) |
| Scenes (data, editor-owned) | 694 `.tscn` | none | none |
| Tests | 315 GDScript | 244 | 284 |

The C++ count includes headers. In the other two the arena layout, actor meshes and HUD layout
are code; here they moved into the 694 lines of scene files, which you'd normally edit in the
editor rather than by hand.

## Trade-offs I ran into

### Where Godot was better

- **An editor and scenes.** The arena, actors and HUD are data you can open, drag and tweak
  without recompiling. Neither Bevy nor Babylon has that (Bevy's editor isn't there yet). For a
  level-based game this is the biggest difference.
- **Everything is built in and works together.** Physics (Jolt), navmesh baking, particles,
  spatial audio, UI with layout containers, sub-viewports and render layers all come with the
  engine, on one version. No vendoring, no waiting on a crate to catch up.
- **The radar was easy.** A SubViewport with its own camera and `cull_mask` is a stock node; it
  sizes itself inside a container. Render layers default to layer 1 only, so the actors stayed
  off the radar without opting out (Babylon's default made every mesh visible to every camera).
- **UI layout.** Containers size to their content, so the HUD needed none of the hand-set widths
  Babylon GUI did. It's retained-mode like Babylon, but much nicer to lay out.
- **Tests were cheap to get going.** `godot --headless -s script.gd` runs the real scene with no
  GPU. 15 of 16 ported tests passed on the first run, and the whole suite takes ~2s.

### Where it was worse

- **C++ is a second-class citizen.** GDScript and C# are the main languages; GDExtension C++ goes
  through generated bindings (`godot-cpp`). Every method a script or signal can call needs
  `ClassDB::bind_method` boilerplate. `godot-cpp` has no lambda Callables, so every signal
  handler is a member function. Several names differ from the engine's own C++ that the docs
  and forum answers use (`Math_TAU` is `Math::TAU`, `Color8` is `Color::from_rgba8`).
- **Footguns the compiler can't see.** `String("RADAR · ")` decodes as Latin-1 and showed
  "RADAR Â· SONAR" on screen; you need `String::utf8`. Node lookups like `get_node("Head/Eyes")`
  and signal names are strings, checked only at runtime. The behaviour tree is XML in a string,
  also checked only at runtime. Bevy catches nearly all of this at compile time.
- **Heavy first build, then fast.** godot-cpp generates and compiles ~1,100 files of bindings
  for every engine class (a few minutes, once). After that, rebuilding the game library takes
  seconds and the editor hot-reloads it. That's better than Bevy, though slower than Vite.
- **Tests live in a different language.** Unit-testing the C++ directly would need a separate
  harness. Gameplay tests are GDScript driving the scene, so anything they inspect has to be
  bound and exposed to scripts first (`get_suspicion`, `sample_start`, …).
- **Two sources of truth.** Some behaviour lives in code and some in scene files (collision
  layers, groups, node names). A renamed node in the editor breaks C++ at runtime.

### Surprises either way

- **Input in a fixed-step test.** Pressing an action and checking it on the same physics frame
  missed the press. Holding it for one frame fixed it, so `press()` in the tests does that.
- **Navmesh precision.** Godot warns unless the agent radius and height are whole multiples of
  the navmesh cell size, and the map's cell size must match (`project.godot` sets 0.2).
- **Colour space.** Albedo colours are sRGB, so the colours copied from the Bevy game came out
  washed out until they were darkened.
- **No web export needed here.** Godot 4 can export to the web, but C++ extensions there need
  an Emscripten build of the library. This port is desktop only.

## Layout

- `native/src/`: the C++ (`arena`, `shooter`, `target` + its behaviour tree, `game` for rounds
  and combat, `presentation` for radar/HUD/fx/audio, `register_types` for the entry point).
- `scenes/`: `main.tscn` (root `Game` node), `arena`, `shooter`, `target`, `bullet`, `hud`.
- `tests/gameplay.gd`: ports of `tests/gameplay.rs` plus the Babylon port's driving and wall
  tests. `ONLY=<name>` runs a subset; `TRACE=1` prints the target's state during the long fight.
- `tests/smoke.gd`: plays a few seconds with a real renderer and saves screenshots
  (`godot --path . -s res://tests/smoke.gd -- <out dir>`).
- `thirdparty/`: git submodules for godot-cpp and BehaviorTree.CPP.
- `sfx/`: copies of `../assets/sfx` (Godot only loads files inside the project).
