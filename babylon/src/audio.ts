// Spatial audio with Babylon's AudioV2 (Web Audio underneath). The listener is the target's
// head, so everything is heard from *his* position: your footsteps and gunshots tell you where
// you are relative to the view. The WAVs are the Bevy game's sfxr-generated ones (shared via
// Vite's publicDir).

import {
  type AudioEngineV2,
  CreateAudioEngineAsync,
  CreateSoundAsync,
  CreateSoundBufferAsync,
  type StaticSoundBuffer,
  type Vector3,
} from "@babylonjs/core";
import type { Game } from "./game";

const FILES = ["shot", "impact", "target_hit", "return_fire", "step"] as const;
type Sfx = (typeof FILES)[number];

export async function startAudio(game: Game): Promise<void> {
  // Browsers keep audio suspended until a user gesture; the engine resumes on the first one.
  const engine = await CreateAudioEngineAsync({ resumeOnInteraction: true });
  const buffers = Object.fromEntries(
    await Promise.all(FILES.map(async (f) => [f, await CreateSoundBufferAsync(`sfx/${f}.wav`, {}, engine)])),
  ) as Record<Sfx, StaticSoundBuffer>;

  const attach = () => engine.listener.attach(game.target.camera);
  game.events.roundStarted.add(attach);
  attach();

  const play = (sfx: Sfx, at: Vector3, radius: number) => {
    if (game.state !== "Playing") return;
    void playAt(engine, buffers[sfx], at, radius);
  };
  const ev = game.events;
  ev.gunshot.add((e) => play("shot", e.muzzle, 80));
  ev.bulletImpact.add((e) => play("impact", e.at, 30));
  ev.targetHit.add((e) => play("target_hit", e.at, 10));
  ev.shooterHit.add((e) => play("return_fire", e.from, 30));
  ev.footstep.add((e) => play("step", e.at, 24));
}

/** One-shot spatial sound, disposed when it ends. Linear falloff to silence at `radius`. */
async function playAt(engine: AudioEngineV2, buffer: StaticSoundBuffer, at: Vector3, radius: number) {
  const sound = await CreateSoundAsync(
    "sfx",
    buffer,
    {
      spatialPosition: at.clone(),
      spatialDistanceModel: "linear",
      spatialMinDistance: 1,
      spatialMaxDistance: radius,
    },
    engine,
  );
  sound.onEndedObservable.addOnce(() => sound.dispose());
  sound.play();
}
