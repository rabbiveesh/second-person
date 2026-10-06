// The target: owns the main camera (his eyes), perceives the shooter, and is driven by a
// behaviour tree (mistreevous, defined in its MDSL text format).
//
// Tree actions are thin: they set intent (`lookGoal`, `moveTo`, `activity`), and the per-step
// methods `perceive` → `gaze` → `walk` turn intent into motion. Preemption is declarative here:
// each branch carries a `while(...)` guard, and a running branch fails the moment its guard
// does (Bevy's version had each task check `interrupted()` by hand).
//
// Once engaged he breaks line of sight: runs to cover (no shooting while running), then fights
// from it, peeking out to shoot and ducking back. Getting hit makes him re-plan.

import {
  Color3,
  FreeCamera,
  Mesh,
  MeshBuilder,
  PhysicsAggregate,
  PhysicsMotionType,
  PhysicsRaycastResult,
  PhysicsShapeType,
  Quaternion,
  type Scene,
  TransformNode,
  Vector2,
  Vector3,
  VertexData,
} from "@babylonjs/core";
import { BehaviourTree, State } from "mistreevous";
import { ARENA_HALF, type Cover, findCover, xz } from "./arena";
import { Filter, Mask, physicsOf } from "./layers";
import type { Nav } from "./nav";
import type { Shooter } from "./shooter";
import { unlit } from "./shooter";

/** Body capsule centre height; eyes sit `EYE_OFFSET` above it (~1.6m). */
export const BODY_CENTER = 0.9;
const EYE_OFFSET = 0.7;
export const TARGET_MAX_HP = 3;

export const VIEW_RANGE = 40;
/** ~34°, a bit narrower than the camera so "seen" means clearly on screen. */
export const VIEW_HALF_ANGLE = 0.6;
const FOV = (70 * Math.PI) / 180;
const WALK_SPEED = 2.2;
const RUN_SPEED = 5.5;
/** "Close enough" for every arrival check (walk itself homes in to 0.2m). */
const ARRIVE = 0.5;

export const TARGET_RADAR_COLOR = new Color3(1, 0.2, 0.2);

export type Activity = "Scanning" | "Wandering" | "Investigating" | "TakingCover" | "Engaging";

/** How sure he is someone's out there. At 1.0 he engages, and keeps engaging until it drains to 0. */
export class Suspicion {
  level = 0;
  seesShooter = false;
  engaged = false;
  /** Where he last saw or heard the shooter. */
  lastKnown: Vector3 | undefined;

  bump(amount: number) {
    this.level = Math.min(this.level + amount, 1);
    if (this.level >= 1) this.engaged = true;
  }
}

export interface MoveTo {
  dest: Vector3;
  speed: number;
  /** Move without turning to face the path (side-stepping while watching a threat). */
  strafe: boolean;
}

/** Something got his attention: look toward `at` until the time runs out. */
interface Alert {
  at: Vector3;
  remaining: number;
}

const TREE = `
root {
  selector {
    sequence while(IsEngaged) {
      action [TakeCover] entry(PlanCover) exit(StopMoving)
      action [Fight] entry(StartFight) exit(EndFight)
    }
    action [Investigate] while(IsAlertedOnly)
    action [Wander] while(MayWander) entry(PlanWander) exit(StopMoving)
    action [Scan] while(IsCalm) entry(PlanScan)
  }
}`;

export class Target {
  hp = TARGET_MAX_HP;
  readonly suspicion = new Suspicion();
  activity: Activity | undefined;
  alert: Alert | undefined;
  /** Whether he may walk around (vs. standing still and looking around). Owned by the game. */
  mobile = false;

  readonly body: Mesh;
  readonly head: TransformNode;
  readonly camera: FreeCamera;
  readonly liveBlips: Mesh[];
  readonly radarColor = TARGET_RADAR_COLOR;
  readonly aggregate: PhysicsAggregate;

  yaw = 0;
  private pitch = 0;
  private elapsed = 0;
  lookGoal = { point: new Vector3(0, BODY_CENTER + EYE_OFFSET, -10), turnSpeed: 1 };
  moveTo: MoveTo | undefined;
  route: Vector3[] = [];
  coverPlan: Cover | undefined;
  private fighting: { peeking: boolean; timer: number; hpAtStart: number } | undefined;
  private scanPlan: { yaw: number; dwell: number } | undefined;
  private tree: BehaviourTree;
  private dt = 0;
  private ray = new PhysicsRaycastResult();

  constructor(
    private scene: Scene,
    parent: TransformNode,
    private nav: Nav,
    private rng: () => number = Math.random,
  ) {
    // Invisible: we only ever see out of his eyes. The mesh carries his kinematic Havok body.
    this.body = MeshBuilder.CreateCapsule("target", { radius: 0.4, height: 1.8 }, scene);
    this.body.parent = parent;
    this.body.isVisible = false;
    this.body.layerMask = Mask.World;
    this.body.position.set(0, BODY_CENTER, 0);
    this.body.rotationQuaternion = Quaternion.Identity();
    this.aggregate = new PhysicsAggregate(this.body, PhysicsShapeType.CAPSULE, { mass: 0 }, scene);
    this.aggregate.body.setMotionType(PhysicsMotionType.ANIMATED);
    // Let the mesh transform drive the body each step (kinematic, like avian's RigidBody::Kinematic).
    this.aggregate.body.disablePreStep = false;
    this.aggregate.shape.filterMembershipMask = Filter.Target;

    this.head = new TransformNode("head", scene);
    this.head.parent = this.body;
    this.head.position.y = EYE_OFFSET;
    this.head.rotationQuaternion = Quaternion.Identity();

    this.camera = new FreeCamera("eyes", Vector3.Zero(), scene);
    this.camera.parent = this.head;
    // Right-handed scenes default cameras to a half-turn (looking +Z); face -Z like his body.
    this.camera.rotation.setAll(0);
    this.camera.fov = FOV;
    this.camera.fovMode = FreeCamera.FOVMODE_VERTICAL_FIXED;
    this.camera.minZ = 0.05;
    this.camera.maxZ = 200;
    this.camera.layerMask = Mask.World;
    this.camera.inputs.clear();

    const blip = MeshBuilder.CreateSphere("target blip", { diameter: 1.8 }, scene);
    blip.position.y = 4 - BODY_CENTER;
    blip.material = unlit(scene, "target blip", TARGET_RADAR_COLOR);
    // Flat view-cone sector pointing along -Z.
    const cone = viewCone(scene, VIEW_RANGE * 0.5, VIEW_HALF_ANGLE);
    cone.position.y = 3 - BODY_CENTER;
    cone.material = unlit(scene, "view cone", new Color3(1, 0.9, 0.3), 0.25);
    this.liveBlips = [blip, cone];
    for (const m of this.liveBlips) {
      m.parent = this.body;
      m.layerMask = Mask.Radar;
    }

    this.tree = new BehaviourTree(TREE, this.agent(), { random: rng });
  }

  get position(): Vector3 {
    return this.body.position;
  }

  get eye(): Vector3 {
    return this.camera.globalPosition;
  }

  /** Where his eyes point (world space). */
  eyeForward(): Vector3 {
    return this.camera.getDirection(new Vector3(0, 0, -1));
  }

  /** Teleport and face `yaw` (tests). */
  place(pos: Vector3, yaw: number) {
    this.body.position.copyFrom(pos);
    this.yaw = yaw;
    this.body.rotationQuaternion = Quaternion.RotationAxis(Vector3.Up(), yaw);
    this.lookGoal.point = pos.add(new Vector3(-Math.sin(yaw), EYE_OFFSET, -Math.cos(yaw)).scale(10));
    this.body.computeWorldMatrix(true);
    this.head.computeWorldMatrix(true);
    this.camera.computeWorldMatrix();
  }

  raiseAlert(at: Vector3, secs: number) {
    this.alert = { at: at.clone(), remaining: secs };
  }

  setMoveTo(m: MoveTo | undefined) {
    this.moveTo = m;
    // Plan immediately (Bevy reacted to `Changed<MoveTo>` a system later). Fall back to a
    // straight line if the point is off-mesh.
    this.route = m ? this.nav.path(this.position, m.dest) : [];
    if (m && this.route.length === 0) this.route = [m.dest];
  }

  /** One fixed step: senses, decide, act. */
  step(dt: number, shooter: Shooter) {
    this.dt = dt;
    this.elapsed += dt;
    this.perceive(dt, shooter);
    this.tree.step();
    this.gaze(dt);
    this.walk(dt);
  }

  // -------------------------------------------------------------------------
  // Behaviour tree agent
  // -------------------------------------------------------------------------

  private agent() {
    const s = this.suspicion;
    const interrupted = () => s.engaged || this.alert !== undefined;
    const goTo = (p: Vector2, speed: number, strafe: boolean) =>
      this.setMoveTo({ dest: new Vector3(p.x, BODY_CENTER, p.y), speed, strafe });
    const here = () => xz(this.position);

    return {
      IsEngaged: () => s.engaged,
      IsAlertedOnly: () => this.alert !== undefined && !s.engaged,
      MayWander: () => this.mobile && !interrupted(),
      IsCalm: () => !interrupted(),

      PlanCover: () => {
        const threat = s.lastKnown ? xz(s.lastKnown) : here().add(xz(this.forwardFlat()).scale(10));
        this.coverPlan = findCover(here(), threat) ?? { spot: here(), peek: here() };
        goTo(this.coverPlan.spot, RUN_SPEED, false);
      },
      TakeCover: () => {
        this.activity = "TakingCover";
        const plan = this.coverPlan!;
        return Vector2.Distance(here(), plan.spot) < ARRIVE ? State.SUCCEEDED : State.RUNNING;
      },
      StopMoving: () => this.setMoveTo(undefined),

      StartFight: () => {
        this.fighting = { peeking: false, timer: 0.8 + this.rng() * 0.8, hpAtStart: this.hp };
      },
      EndFight: () => {
        this.fighting = undefined;
        this.setMoveTo(undefined);
      },
      // Hide behind cover, then peek out to shoot, then hide again. Succeeds (so the tree
      // re-plans cover) when he's no longer engaged or gets hit.
      Fight: () => {
        this.activity = "Engaging";
        const f = this.fighting!;
        const plan = this.coverPlan!;
        if (!s.engaged || this.hp < f.hpAtStart) return State.SUCCEEDED;
        if (s.lastKnown) {
          this.lookGoal.point = s.lastKnown.add(new Vector3(0, EYE_OFFSET, 0));
          this.lookGoal.turnSpeed = 4;
        }
        const goal = f.peeking ? plan.peek : plan.spot;
        if (Vector2.Distance(here(), goal) >= ARRIVE) {
          if (!this.moveTo) goTo(goal, RUN_SPEED, true);
          return State.RUNNING;
        }
        f.timer -= this.dt;
        if (f.timer <= 0) {
          f.peeking = !f.peeking;
          f.timer = f.peeking ? 1.8 : 0.8 + this.rng() * 1.2;
          goTo(f.peeking ? plan.peek : plan.spot, RUN_SPEED, true);
        }
        return State.RUNNING;
      },

      Investigate: () => {
        this.activity = "Investigating";
        const alert = this.alert!;
        this.lookGoal.point = alert.at;
        this.lookGoal.turnSpeed = 3.5;
        alert.remaining -= this.dt;
        if (alert.remaining <= 0) {
          this.alert = undefined;
          return State.SUCCEEDED;
        }
        return State.RUNNING;
      },

      PlanWander: () => {
        const r = ARENA_HALF - 4;
        goTo(new Vector2((this.rng() * 2 - 1) * r, (this.rng() * 2 - 1) * r), WALK_SPEED, false);
      },
      Wander: () => {
        this.activity = "Wandering";
        const m = this.moveTo;
        if (!m) return State.RUNNING;
        return Vector2.Distance(here(), xz(m.dest)) < ARRIVE ? State.SUCCEEDED : State.RUNNING;
      },

      PlanScan: () => {
        const swing = (0.6 + this.rng() * 2) * (this.rng() < 0.5 ? 1 : -1);
        this.scanPlan = { yaw: this.yaw + swing, dwell: 0.6 + this.rng() * 1.6 };
      },
      Scan: () => {
        this.activity = "Scanning";
        const plan = this.scanPlan!;
        const eye = this.position.add(new Vector3(0, EYE_OFFSET, 0));
        this.lookGoal.point = eye.add(new Vector3(-Math.sin(plan.yaw), 0, -Math.cos(plan.yaw)).scale(10));
        this.lookGoal.turnSpeed = 1.2;
        if (Math.abs(angleDiff(this.yaw, plan.yaw)) < 0.05) {
          plan.dwell -= this.dt;
          if (plan.dwell <= 0) return State.SUCCEEDED;
        }
        return State.RUNNING;
      },
    };
  }

  // -------------------------------------------------------------------------
  // Per-step behaviour
  // -------------------------------------------------------------------------

  /** Vision: is the shooter inside the view cone with clear line of sight? Feeds `suspicion`. */
  private perceive(dt: number, shooter: Shooter) {
    const s = this.suspicion;
    const eye = this.eye;
    const toShooter = shooter.position.subtract(eye);
    const dist = toShooter.length();
    const angle = Math.acos(clamp(Vector3.Dot(this.eyeForward(), toShooter.normalizeToNew()), -1, 1));

    let sees = dist < VIEW_RANGE && angle < VIEW_HALF_ANGLE;
    if (sees) {
      // The character controller isn't a physics body, so cast against the world only.
      physicsOf(this.scene).raycastToRef(eye, shooter.position, this.ray, {
        collideWith: Filter.World,
      });
      sees = !this.ray.hasHit;
    }
    s.seesShooter = sees;
    if (sees) s.lastKnown = shooter.position.clone();

    if (sees) {
      const closeness = 1 - dist / VIEW_RANGE;
      const centred = 1 - angle / VIEW_HALF_ANGLE;
      const moving = shooter.speed > 0.5 ? 1 : 0.25;
      s.bump((0.08 + 0.6 * closeness) * (0.4 + 0.6 * centred) * moving * dt);
      // Half-sure: glance over.
      if (s.level > 0.5 && !s.engaged) this.raiseAlert(shooter.position, 1.5);
    } else {
      s.level = Math.max(s.level - 0.12 * dt, 0);
      if (s.level <= 0) s.engaged = false;
    }
  }

  /** Turn the body (yaw) and head (pitch) toward the look goal. */
  private gaze(dt: number) {
    const eye = this.position.add(new Vector3(0, EYE_OFFSET, 0));
    const d = this.lookGoal.point.subtract(eye);
    const step = this.lookGoal.turnSpeed * dt;

    const desiredYaw = Math.atan2(-d.x, -d.z);
    this.yaw += clamp(angleDiff(this.yaw, desiredYaw), -step, step);
    this.body.rotationQuaternion = Quaternion.RotationAxis(Vector3.Up(), this.yaw);

    // Pitch, plus a little idle sway so the view feels alive.
    const desiredPitch =
      clamp(Math.atan2(d.y, Math.hypot(d.x, d.z)), -0.6, 0.6) + 0.03 * Math.sin(this.elapsed * 0.7);
    this.pitch += clamp(desiredPitch - this.pitch, -step, step);
    this.head.rotationQuaternion = Quaternion.RotationAxis(Vector3.Right(), this.pitch);
  }

  /** Follow the route planned for `moveTo`, looking where he's going; turn before moving. */
  private walk(dt: number) {
    const m = this.moveTo;
    if (!m) return;
    const here = xz(this.position);
    while (this.route.length > 1 && Vector2.Distance(xz(this.route[0]), here) < 0.35) {
      this.route.shift();
    }
    const w = xz(this.route[0] ?? m.dest);
    const d = w.subtract(here);
    if (d.length() < 0.2) return;
    const dir = new Vector3(d.x, 0, d.y).normalize();
    if (!m.strafe) {
      this.lookGoal.point = new Vector3(w.x, this.position.y + EYE_OFFSET, w.y);
      this.lookGoal.turnSpeed = 7;
      if (Math.acos(clamp(Vector3.Dot(this.forwardFlat(), dir), -1, 1)) >= 0.5) return;
    }
    // Don't overshoot the waypoint in one step.
    this.position.addInPlace(dir.scale(Math.min(m.speed * dt, d.length())));
  }

  private forwardFlat(): Vector3 {
    return new Vector3(-Math.sin(this.yaw), 0, -Math.cos(this.yaw));
  }
}

/** Shortest signed angle from `a` to `b`. */
export function angleDiff(a: number, b: number): number {
  const tau = Math.PI * 2;
  return ((((b - a + Math.PI) % tau) + tau) % tau) - Math.PI;
}

const clamp = (x: number, lo: number, hi: number) => Math.min(Math.max(x, lo), hi);

/** A flat circular sector in XZ, apex at the origin, centred on -Z. */
function viewCone(scene: Scene, radius: number, halfAngle: number): Mesh {
  const segments = 16;
  const positions = [0, 0, 0];
  const indices: number[] = [];
  for (let i = 0; i <= segments; i++) {
    const a = -halfAngle + (2 * halfAngle * i) / segments;
    positions.push(-Math.sin(a) * radius, 0, -Math.cos(a) * radius);
    if (i > 0) indices.push(0, i, i + 1);
  }
  const cone = new Mesh("view cone", scene);
  Object.assign(new VertexData(), { positions, indices }).applyToMesh(cone);
  return cone;
}
