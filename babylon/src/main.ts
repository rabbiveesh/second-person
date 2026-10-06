// Browser entry point: engine, Havok/Recast wasm, gameplay, then presentation on top.

import HavokPhysics from "@babylonjs/havok";
import havokWasm from "@babylonjs/havok/lib/esm/HavokPhysics.wasm?url";
import { Engine } from "@babylonjs/core";
import { startAudio } from "./audio";
import { Fx } from "./fx";
import { FIXED_DT, Game } from "./game";
import { Hud, hudCamera } from "./hud";
import { Radar } from "./radar";

async function main() {
  const canvas = document.getElementById("game") as HTMLCanvasElement;
  const engine = new Engine(canvas, true, {
    // Gameplay runs in scene.onBeforeStepObservable at a fixed 60 Hz, decoupled from frame rate.
    deterministicLockstep: true,
    lockstepMaxSteps: 4,
    timeStep: FIXED_DT,
  });
  const havok = await HavokPhysics({ locateFile: () => havokWasm });
  const game = await Game.create(engine, havok);
  game.input.attach(game.scene);

  const hud = hudCamera(game);
  const radar = new Radar(game, hud);
  new Fx(game);
  new Hud(game, radar);
  startAudio(game).catch((e) => console.warn("audio unavailable", e));

  // For tests and poking around from the devtools console.
  Object.assign(window, { game });

  document.getElementById("loading")?.remove();
  canvas.focus();
  engine.runRenderLoop(() => game.scene.render());
  window.addEventListener("resize", () => engine.resize());
}

main().catch((e) => {
  console.error(e);
  const el = document.getElementById("loading");
  if (el) el.textContent = `failed to start: ${e}`;
});
