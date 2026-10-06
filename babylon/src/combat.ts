// Bullets, hits, hearing, and the target shooting back.
//
// Bullets are swept raycasts (each step casts from where the bullet was to where it will be)
// rather than CCD rigid bodies: Babylon's Havok plugin doesn't expose per-body CCD the way
// avian's `SweptCcd` does, and a ray per bullet is cheaper and can't tunnel anyway.

import {
  Color3,
  type Mesh,
  MeshBuilder,
  PhysicsRaycastResult,
  type Scene,
  StandardMaterial,
  type TransformNode,
  Vector3,
} from "@babylonjs/core";
import type { GameEvents } from "./events";
import type { Input } from "./input";
import { Filter, Mask, physicsOf } from "./layers";
import type { Shooter } from "./shooter";
import type { Target } from "./target";

const BULLET_SPEED = 45;
const BULLET_LIFETIME = 3;
const FIRE_COOLDOWN = 0.3;
export const HEARING_RANGE = 18;
const NEAR_MISS_RANGE = 7;
const RETURN_FIRE_INTERVAL = 0.8;
export const RETURN_FIRE_DAMAGE = 8;
const MUZZLE = new Vector3(0.3, 0.15, -0.9);

export interface Bullet {
  mesh: Mesh;
  velocity: Vector3;
  /** Where it was fired from: the target looks back here if it lands nearby. */
  origin: Vector3;
  age: number;
}

export class Combat {
  readonly bullets: Bullet[] = [];
  private cooldown = 0;
  private returnFireTimer = 0;
  private ray = new PhysicsRaycastResult();
  private bulletMat: StandardMaterial;

  constructor(
    private scene: Scene,
    private parent: TransformNode,
    private events: GameEvents,
  ) {
    this.bulletMat = new StandardMaterial("bullet", scene);
    this.bulletMat.disableLighting = true;
    this.bulletMat.emissiveColor = new Color3(1, 0.9, 0.3);
  }

  step(dt: number, input: Input, shooter: Shooter, target: Target) {
    this.fire(dt, input, shooter, target);
    this.moveBullets(dt, target);
    this.returnFire(dt, shooter, target);
  }

  private fire(dt: number, input: Input, shooter: Shooter, target: Target) {
    this.cooldown = Math.max(this.cooldown - dt, 0);
    if (!input.justPressed("fire") || this.cooldown > 0) return;
    this.cooldown = FIRE_COOLDOWN;

    const forward = shooter.forward();
    const muzzle = shooter.local(MUZZLE);
    const mesh = MeshBuilder.CreateSphere("bullet", { diameter: 0.24, segments: 6 }, this.scene);
    mesh.parent = this.parent;
    mesh.position.copyFrom(muzzle);
    mesh.material = this.bulletMat;
    mesh.layerMask = Mask.World;
    this.bullets.push({ mesh, velocity: forward.scale(BULLET_SPEED), origin: shooter.position.clone(), age: 0 });
    this.events.gunshot.notifyObservers({ muzzle, dir: forward });

    // Gunshots are loud.
    if (Vector3.Distance(target.position, shooter.position) < HEARING_RANGE) {
      target.suspicion.bump(0.25);
      target.suspicion.lastKnown = shooter.position.clone();
      target.raiseAlert(shooter.position, 3);
    }
  }

  private moveBullets(dt: number, target: Target) {
    const physics = physicsOf(this.scene);
    for (let i = this.bullets.length - 1; i >= 0; i--) {
      const b = this.bullets[i];
      b.age += dt;
      const from = b.mesh.position;
      const to = from.add(b.velocity.scale(dt));
      physics.raycastToRef(from, to, this.ray, { collideWith: Filter.World | Filter.Target });
      if (!this.ray.hasHit) {
        b.mesh.position.copyFrom(to);
        if (b.age > BULLET_LIFETIME) this.despawn(i);
        continue;
      }
      const at = this.ray.hitPointWorld.clone();
      this.despawn(i);
      const s = target.suspicion;
      if (this.ray.body === target.aggregate.body) {
        target.hp = Math.max(target.hp - 1, 0);
        s.bump(0.6);
        s.lastKnown = b.origin;
        target.raiseAlert(b.origin, 3);
        this.events.targetHit.notifyObservers({ at: target.position.clone() });
      } else {
        this.events.bulletImpact.notifyObservers({ at });
        // Near miss: he hears the impact and looks toward where it came from.
        if (Vector3.Distance(target.position, at) < NEAR_MISS_RANGE) {
          s.bump(0.3);
          s.lastKnown = b.origin;
          target.raiseAlert(b.origin, 2.5);
        }
      }
    }
  }

  private despawn(i: number) {
    this.bullets[i].mesh.dispose();
    this.bullets.splice(i, 1);
  }

  /** While engaging and able to see the shooter, the target fires back (hitscan). */
  private returnFire(dt: number, shooter: Shooter, target: Target) {
    if (target.activity !== "Engaging" || !target.suspicion.seesShooter) {
      this.returnFireTimer = 0;
      return;
    }
    this.returnFireTimer += dt;
    if (this.returnFireTimer < RETURN_FIRE_INTERVAL) return;
    this.returnFireTimer -= RETURN_FIRE_INTERVAL;
    shooter.hp = Math.max(shooter.hp - RETURN_FIRE_DAMAGE, 0);
    // Start the tracer just below the eyes so it's visible from his own view.
    const cam = target.camera;
    const from = cam.globalPosition
      .add(cam.getDirection(new Vector3(0, -1, 0)).scale(0.3))
      .add(cam.getDirection(new Vector3(1, 0, 0)).scale(0.2));
    this.events.shooterHit.notifyObservers({ from, to: shooter.position.clone() });
  }
}
