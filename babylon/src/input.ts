// Keyboard → actions. Babylon only gives raw key events (no leafwing-style action maps), so
// this is the small layer on top: bindings, held state, and "just pressed" latched until the
// next fixed step consumes it.

import { KeyboardEventTypes, type Scene } from "@babylonjs/core";

export const Bindings = {
  forward: "ArrowUp",
  back: "ArrowDown",
  turnLeft: "ArrowLeft",
  turnRight: "ArrowRight",
  fire: "Space",
  restart: "KeyR",
  toggleMobility: "KeyM",
  cycleRadar: "Tab",
} as const;

export type Action = keyof typeof Bindings;

export class Input {
  private held = new Set<string>();
  private fresh = new Set<string>();

  /** Feed from a scene's keyboard events (omitted in headless tests, which call press/release). */
  attach(scene: Scene) {
    scene.onKeyboardObservable.add(({ type, event }) => {
      const code = event.code;
      // Keep arrows/space from scrolling and Tab from moving focus off the canvas.
      if ((Object.values(Bindings) as string[]).includes(code)) event.preventDefault();
      if (type === KeyboardEventTypes.KEYDOWN) {
        if (!event.repeat) this.press(code);
      } else {
        this.release(code);
      }
    });
  }

  press(code: string) {
    if (!this.held.has(code)) this.fresh.add(code);
    this.held.add(code);
  }

  release(code: string) {
    this.held.delete(code);
  }

  pressed(a: Action): boolean {
    return this.held.has(Bindings[a]);
  }

  justPressed(a: Action): boolean {
    return this.fresh.has(Bindings[a]);
  }

  axis(neg: Action, pos: Action): number {
    return Number(this.pressed(pos)) - Number(this.pressed(neg));
  }

  /** Called at the end of each fixed step. */
  endStep() {
    this.fresh.clear();
  }
}
