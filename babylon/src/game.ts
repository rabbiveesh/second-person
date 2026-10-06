// The simulation: scene, physics, arena, navmesh, round lifecycle and the fixed-step update.
// Runs on a NullEngine in tests, so nothing here may assume a canvas, GUI or audio.

import {
  type AbstractEngine,
  Color3,
  Color4,
  DirectionalLight,
  HavokPlugin,
  HemisphericLight,
  Scene,
  TransformNode,
  Vector3,
} from "@babylonjs/core";
import { buildArena, type ArenaMeshes } from "./arena";
import { Combat } from "./combat";
import { GameEvents } from "./events";
import { Input } from "./input";
import { Mask } from "./layers";
import { Nav } from "./nav";
import { Shooter, randomStart } from "./shooter";
import { Target } from "./target";

export type GameState = "Playing" | "Won" | "Lost";
/** The difficulty knob. Lives here (not in radar) so headless gameplay has it too. */
export type RadarMode = "Full" | "Sonar" | "Off";
const RADAR_MODES: RadarMode[] = ["Full", "Sonar", "Off"];

export const FIXED_DT = 1 / 60;

/** Everything that belongs to one round; disposed on restart. */
export class Round {
  readonly root: TransformNode;
  readonly shooter: Shooter;
  readonly target: Target;
  readonly combat: Combat;

  constructor(game: Game) {
    const { scene } = game;
    this.root = new TransformNode("round", scene);
    this.target = new Target(scene, this.root, game.nav, game.rng);
    this.target.mobile = game.targetMobile;
    this.shooter = new Shooter(scene, this.root, randomStart(game.rng), game.events);
    this.combat = new Combat(scene, this.root, game.events);
  }

  dispose() {
    this.shooter.dispose();
    // Disposing the root disposes every mesh, camera and physics body parented under it.
    this.root.dispose();
  }
}

export class Game {
  readonly events = new GameEvents();
  readonly input = new Input();
  state: GameState = "Playing";
  radarMode: RadarMode = "Sonar";
  targetMobile = false;
  round!: Round;
  readonly sun: DirectionalLight;

  private constructor(
    readonly scene: Scene,
    readonly arena: ArenaMeshes,
    readonly nav: Nav,
    readonly rng: () => number,
  ) {
    const sun = new DirectionalLight("sun", new Vector3(-20, -40, -10).normalize(), scene);
    sun.position = new Vector3(20, 40, 10);
    sun.intensity = 1.3;
    const sky = new HemisphericLight("sky", new Vector3(0, 1, 0), scene);
    sky.intensity = 0.45;
    sky.groundColor = new Color3(0.25, 0.25, 0.3);
    // Main view only: radar meshes are unlit, and this keeps it that way if one isn't.
    for (const l of [sun, sky]) l.includeOnlyWithLayerMask = Mask.World;
    this.sun = sun;
    this.startRound();
  }

  /**
   * `havok` is the initialised Havok wasm module (browser and Node load it differently).
   * The engine should be created with `deterministicLockstep` so gameplay runs at a fixed 60 Hz.
   */
  static async create(engine: AbstractEngine, havok: unknown, rng: () => number = Math.random): Promise<Game> {
    const scene = new Scene(engine);
    // Same handedness as Bevy, so the ported maths (yaw, forward = -Z) carry over unchanged.
    scene.useRightHandedSystem = true;
    scene.clearColor = new Color4(0.55, 0.7, 0.85, 1);
    scene.enablePhysics(new Vector3(0, -9.81, 0), new HavokPlugin(false, havok));
    scene.getPhysicsEngine()!.setTimeStep(FIXED_DT);
    const arena = buildArena(scene);
    const nav = await Nav.build([arena.ground, ...arena.obstacles]);
    const game = new Game(scene, arena, nav, rng);
    scene.onBeforeStepObservable.add(() => game.step(FIXED_DT));
    return game;
  }

  get shooter() {
    return this.round.shooter;
  }

  get target() {
    return this.round.target;
  }

  startRound() {
    this.round?.dispose();
    this.round = new Round(this);
    this.scene.activeCamera = this.round.target.camera;
    this.state = "Playing";
    // Physics pauses outside Playing.
    this.scene.physicsEnabled = true;
    this.events.roundStarted.notifyObservers();
  }

  /** One fixed step of gameplay. Physics advances right after (Babylon's lockstep order). */
  step(dt: number) {
    this.metaInput();
    if (this.state === "Playing") {
      const { shooter, target, combat } = this.round;
      target.mobile = this.targetMobile;
      shooter.step(dt, this.input);
      target.step(dt, shooter);
      combat.step(dt, this.input, shooter, target);
      this.checkOutcome();
    }
    this.input.endStep();
  }

  private metaInput() {
    const input = this.input;
    if (input.justPressed("cycleRadar")) {
      this.radarMode = RADAR_MODES[(RADAR_MODES.indexOf(this.radarMode) + 1) % RADAR_MODES.length];
    }
    if (input.justPressed("restart") && this.state !== "Playing") this.startRound();
    if (input.justPressed("toggleMobility")) this.targetMobile = !this.targetMobile;
  }

  private checkOutcome() {
    if (this.target.hp === 0) this.end("Won");
    else if (this.shooter.hp <= 0) this.end("Lost");
  }

  private end(state: GameState) {
    this.state = state;
    this.scene.physicsEnabled = false;
  }
}
