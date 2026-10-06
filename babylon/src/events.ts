// Gameplay → presentation messages. Bevy uses typed `Message`s read by systems; here they are
// Babylon `Observable`s that fx, audio, radar and HUD subscribe to. Add feedback by
// subscribing, not by calling across modules.

import { Observable, type Vector3 } from "@babylonjs/core";

/** The shooter fired. Drives muzzle flash, gunshot sound and radar ping. */
export interface Gunshot {
  muzzle: Vector3;
  dir: Vector3;
}

export class GameEvents {
  readonly gunshot = new Observable<Gunshot>();
  /** A bullet hit world geometry. */
  readonly bulletImpact = new Observable<{ at: Vector3 }>();
  /** The target got shot. */
  readonly targetHit = new Observable<{ at: Vector3 }>();
  /** The target shot the shooter (hitscan from `from` to `to`). */
  readonly shooterHit = new Observable<{ from: Vector3; to: Vector3 }>();
  /** The shooter took a step (heard by the target). */
  readonly footstep = new Observable<{ at: Vector3 }>();
  /** A fresh round's actors exist (cameras were recreated). */
  readonly roundStarted = new Observable<void>();
}
