// The shooter: the body you control, seen (mostly) through the target's eyes.
// Moved by Havok's character controller rather than a dynamic rigid body: Babylon ships one,
// and it handles sliding along walls without the rotation-lock/friction tweaks avian needed.

import {
  Color3,
  type Mesh,
  MeshBuilder,
  PhysicsCharacterController,
  PhysicsShapeCapsule,
  Quaternion,
  type Scene,
  StandardMaterial,
  type TransformNode,
  Vector2,
  Vector3,
} from "@babylonjs/core";
import { ARENA_HALF, isClear } from "./arena";
import type { GameEvents } from "./events";
import type { Input } from "./input";
import { Filter, Mask } from "./layers";

export const SHOOTER_MAX_HP = 100;
/** Minimum start distance from the target (who starts at the origin). */
export const MIN_START_DISTANCE = 12;
const RADIUS = 0.35;
const HEIGHT = 1.7;
/** Capsule centre height above the ground. */
export const SHOOTER_CENTER = 0.9;
const MOVE_SPEED = 5;
const TURN_SPEED = 2.4;
const STEP_INTERVAL = 0.42;
const GRAVITY = new Vector3(0, -9.81, 0);
const DOWN = new Vector3(0, -1, 0);

export const SHOOTER_RADAR_COLOR = new Color3(0.2, 1, 0.3);

export class Shooter {
  hp = SHOOTER_MAX_HP;
  /** Rotation about Y; 0 faces -Z (same convention as Bevy, thanks to the right-handed scene). */
  yaw: number;
  /** Planar speed this step (the target notices movement more). */
  speed = 0;
  readonly mesh: Mesh;
  /** Radar-only decorations, shown in Full mode. */
  readonly liveBlips: Mesh[];
  readonly radarColor = SHOOTER_RADAR_COLOR;
  private controller: PhysicsCharacterController;
  private stepTimer = 0;

  constructor(
    scene: Scene,
    parent: TransformNode,
    start: { pos: Vector3; yaw: number },
    private events: GameEvents,
  ) {
    this.yaw = start.yaw;
    // Our own shape (not capsuleHeight/Radius) so we can set its filter: by default the
    // controller's hidden body collides with, and is hit by rays from, everything.
    const half = HEIGHT / 2 - RADIUS;
    const shape = new PhysicsShapeCapsule(new Vector3(0, -half, 0), new Vector3(0, half, 0), RADIUS, scene);
    shape.filterMembershipMask = Filter.Shooter;
    shape.filterCollideMask = Filter.World | Filter.Target;
    this.controller = new PhysicsCharacterController(start.pos.clone(), { shape }, scene);

    this.mesh = MeshBuilder.CreateCapsule("shooter", { radius: RADIUS, height: HEIGHT }, scene);
    this.mesh.parent = parent;
    this.mesh.material = lit(scene, "shooter", new Color3(0.95, 0.5, 0.1));
    const visor = MeshBuilder.CreateBox("visor", { width: 0.5, height: 0.15, depth: 0.2 }, scene);
    const visorMat = lit(scene, "visor", new Color3(0.1, 0.9, 1));
    visorMat.emissiveColor = new Color3(0.2, 0.9, 1);
    visor.material = visorMat;
    visor.position.set(0, 0.45, -0.28);
    const gun = MeshBuilder.CreateBox("gun", { width: 0.12, height: 0.12, depth: 0.8 }, scene);
    gun.material = lit(scene, "gun", new Color3(0.15, 0.15, 0.15));
    gun.position.set(0.3, 0.15, -0.45);

    const blipMat = unlit(scene, "shooter blip", SHOOTER_RADAR_COLOR);
    const blip = MeshBuilder.CreateSphere("shooter blip", { diameter: 1.6 }, scene);
    blip.position.y = 4;
    const heading = MeshBuilder.CreateBox("shooter heading", { width: 0.4, height: 0.4, depth: 2.2 }, scene);
    heading.position.set(0, 4, -1.3);
    this.liveBlips = [blip, heading];
    for (const m of [visor, gun, blip, heading]) m.parent = this.mesh;
    for (const m of [this.mesh, visor, gun]) m.layerMask = Mask.World;
    for (const m of this.liveBlips) {
      m.material = blipMat;
      m.layerMask = Mask.Radar;
    }
    this.syncMesh();
  }

  get position(): Vector3 {
    return this.mesh.position;
  }

  forward(): Vector3 {
    return new Vector3(-Math.sin(this.yaw), 0, -Math.cos(this.yaw));
  }

  /** World position of a point in the shooter's local frame. */
  local(offset: Vector3): Vector3 {
    return Vector3.TransformCoordinates(offset, this.mesh.computeWorldMatrix(true));
  }

  /** Tank controls, relative to the shooter's own facing. */
  step(dt: number, input: Input) {
    this.yaw += input.axis("turnRight", "turnLeft") * TURN_SPEED * dt;
    const throttle = input.axis("back", "forward");
    const planar = this.forward().scale(throttle * MOVE_SPEED);
    this.speed = planar.length();

    const support = this.controller.checkSupport(dt, DOWN);
    const v = this.controller.getVelocity();
    this.controller.setVelocity(new Vector3(planar.x, support.supportedState === 2 ? 0 : v.y, planar.z));
    this.controller.integrate(dt, support, GRAVITY);
    this.syncMesh();

    if (throttle !== 0) {
      this.stepTimer -= dt;
      if (this.stepTimer <= 0) {
        this.stepTimer = STEP_INTERVAL;
        this.events.footstep.notifyObservers({ at: this.position.clone() });
      }
    } else {
      this.stepTimer = 0;
    }
  }

  /** Teleport (tests, spawning). */
  place(pos: Vector3, yaw: number) {
    this.controller.setPosition(pos.clone());
    this.controller.setVelocity(Vector3.Zero());
    this.yaw = yaw;
    this.syncMesh();
  }

  dispose() {
    this.controller.dispose();
  }

  private syncMesh() {
    this.mesh.position.copyFrom(this.controller.getPosition());
    this.mesh.rotationQuaternion = Quaternion.RotationAxis(Vector3.Up(), this.yaw);
  }
}

/** A random start: clear of cover, away from the target, facing anywhere. */
export function randomStart(rng: () => number = Math.random): { pos: Vector3; yaw: number } {
  const r = ARENA_HALF - 2;
  for (;;) {
    const p = new Vector2((rng() * 2 - 1) * r, (rng() * 2 - 1) * r);
    if (p.length() >= MIN_START_DISTANCE && isClear(p, RADIUS + 0.3)) {
      return { pos: new Vector3(p.x, SHOOTER_CENTER, p.y), yaw: rng() * Math.PI * 2 };
    }
  }
}

export function lit(scene: Scene, name: string, color: Color3): StandardMaterial {
  const m = new StandardMaterial(name, scene);
  m.diffuseColor = color;
  m.specularColor = new Color3(0.1, 0.1, 0.1);
  return m;
}

export function unlit(scene: Scene, name: string, color: Color3, alpha = 1): StandardMaterial {
  const m = new StandardMaterial(name, scene);
  m.disableLighting = true;
  m.emissiveColor = color;
  m.alpha = alpha;
  m.backFaceCulling = false;
  return m;
}
