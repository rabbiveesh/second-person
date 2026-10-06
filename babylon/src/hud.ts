// HUD with Babylon GUI (a retained-mode control tree drawn into a fullscreen texture), on a
// dedicated overlay camera so it draws after the radar. Bevy's version is immediate-mode egui.

import { Color3, FreeCamera, Vector3 } from "@babylonjs/core";
import {
  AdvancedDynamicTexture,
  Control,
  Rectangle,
  StackPanel,
  TextBlock,
} from "@babylonjs/gui";
import type { Game } from "./game";
import type { Radar } from "./radar";
import { SHOOTER_MAX_HP } from "./shooter";
import { TARGET_MAX_HP, type Activity } from "./target";

const HUD_MASK = 0x4;

const DOING: Record<Activity, string> = {
  Scanning: "looking around",
  Wandering: "wandering",
  Investigating: "investigating a noise",
  TakingCover: "RUNNING FOR COVER",
  Engaging: "SHOOTING AT YOU",
};

/** The overlay camera the HUD renders with. Sees no meshes. */
export function hudCamera(game: Game): FreeCamera {
  const cam = new FreeCamera("hud", new Vector3(0, -100, 0), game.scene);
  cam.layerMask = HUD_MASK;
  cam.inputs.clear();
  return cam;
}

export class Hud {
  private flashHurt = 0;
  private flashHit = 0;

  constructor(game: Game, radar: Radar) {
    const scene = game.scene;
    const ui = AdvancedDynamicTexture.CreateFullscreenUI("hud", true, scene);
    ui.layer!.layerMask = HUD_MASK;

    // Full-screen flashes: red when you're hit, white when the target is hit.
    const flash = new Rectangle("flash");
    flash.thickness = 0;
    flash.isHitTestVisible = false;
    ui.addControl(flash);

    // Status panel (top left).
    const panel = new Rectangle("status");
    Object.assign(panel, {
      width: "260px",
      adaptHeightToChildren: true,
      left: "16px",
      top: "16px",
      cornerRadius: 6,
      thickness: 1,
      color: "#555",
      background: "#1b1b1bdd",
      horizontalAlignment: Control.HORIZONTAL_ALIGNMENT_LEFT,
      verticalAlignment: Control.VERTICAL_ALIGNMENT_TOP,
    });
    ui.addControl(panel);
    const stack = new StackPanel();
    stack.paddingTop = "8px";
    stack.paddingBottom = "8px";
    panel.addControl(stack);
    const label = (text = "") => {
      const t = new TextBlock(undefined, text);
      Object.assign(t, {
        height: "22px",
        color: "#ddd",
        fontSize: 15,
        textHorizontalAlignment: Control.HORIZONTAL_ALIGNMENT_LEFT,
        paddingLeft: "10px",
      });
      stack.addControl(t);
      return t;
    };
    const bar = (fill: string) => {
      const outer = new Rectangle();
      Object.assign(outer, { height: "20px", width: "240px", thickness: 0, background: "#3a3a3a" });
      const inner = new Rectangle();
      Object.assign(inner, {
        thickness: 0,
        background: fill,
        horizontalAlignment: Control.HORIZONTAL_ALIGNMENT_LEFT,
      });
      const text = new TextBlock();
      Object.assign(text, { color: "white", fontSize: 13 });
      outer.addControl(inner);
      outer.addControl(text);
      stack.addControl(outer);
      return {
        set(frac: number, caption: string, color?: string) {
          inner.width = `${Math.max(0, Math.min(1, frac)) * 100}%`;
          if (color) inner.background = color;
          text.text = caption;
        },
      };
    };
    label("YOU (the shooter)");
    const hp = bar("rgb(60,170,80)");
    const targetLine = label();
    const suspicion = bar("rgb(220,170,40)");
    const doing = label();

    const help = new TextBlock("help");
    Object.assign(help, {
      color: "white",
      fontSize: 14,
      resizeToFit: true,
      left: "16px",
      top: "-16px",
      horizontalAlignment: Control.HORIZONTAL_ALIGNMENT_LEFT,
      verticalAlignment: Control.VERTICAL_ALIGNMENT_BOTTOM,
      textHorizontalAlignment: Control.HORIZONTAL_ALIGNMENT_LEFT,
    });
    const helpBg = new Rectangle("help bg");
    Object.assign(helpBg, {
      adaptWidthToChildren: true,
      adaptHeightToChildren: true,
      thickness: 0,
      background: "#0000008c",
      left: "16px",
      top: "-16px",
      horizontalAlignment: Control.HORIZONTAL_ALIGNMENT_LEFT,
      verticalAlignment: Control.VERTICAL_ALIGNMENT_BOTTOM,
    });
    help.left = help.top = "0px";
    help.paddingLeft = help.paddingRight = "6px";
    helpBg.addControl(help);
    ui.addControl(helpBg);

    // Frame + label for the radar viewport.
    const frame = new Rectangle("radar frame");
    Object.assign(frame, {
      thickness: 2,
      color: "rgb(80,220,120)",
      cornerRadius: 4,
      horizontalAlignment: Control.HORIZONTAL_ALIGNMENT_LEFT,
      verticalAlignment: Control.VERTICAL_ALIGNMENT_TOP,
      isHitTestVisible: false,
    });
    const frameLabel = new TextBlock("radar label");
    Object.assign(frameLabel, {
      color: "white",
      fontFamily: "monospace",
      fontSize: 14,
      left: "6px",
      top: "4px",
      textHorizontalAlignment: Control.HORIZONTAL_ALIGNMENT_LEFT,
      textVerticalAlignment: Control.VERTICAL_ALIGNMENT_TOP,
    });
    frame.addControl(frameLabel);
    ui.addControl(frame);

    // End-of-round banner.
    const banner = new Rectangle("banner");
    Object.assign(banner, {
      // Retained-mode GUI doesn't size containers to their text (egui does): size it by hand.
      width: "720px",
      height: "120px",
      cornerRadius: 8,
      thickness: 1,
      color: "#555",
      background: "#1b1b1bee",
    });
    const bannerStack = new StackPanel();
    const bannerText = new TextBlock(undefined, "");
    Object.assign(bannerText, { height: "60px", fontSize: 36, fontWeight: "bold" });
    const bannerHint = new TextBlock(undefined, "press R to go again");
    Object.assign(bannerHint, { height: "30px", fontSize: 18, color: "#ddd" });
    bannerStack.addControl(bannerText);
    bannerStack.addControl(bannerHint);
    banner.addControl(bannerStack);
    ui.addControl(banner);

    game.events.shooterHit.add(() => (this.flashHurt = 0.6));
    game.events.targetHit.add(() => (this.flashHit = 0.7));

    // Retained mode: update the control tree each frame from game state.
    scene.onBeforeRenderObservable.add(() => {
      const dt = scene.getEngine().getDeltaTime() / 1000;
      this.flashHurt = Math.max(this.flashHurt - dt * 2.5, 0);
      this.flashHit = Math.max(this.flashHit - dt * 2.5, 0);
      if (this.flashHurt > 0) flash.background = `rgba(255,0,0,${(this.flashHurt * 120) / 255})`;
      else if (this.flashHit > 0) flash.background = `rgba(255,255,255,${(this.flashHit * 160) / 255})`;
      flash.isVisible = this.flashHurt > 0 || this.flashHit > 0;

      const { shooter, target } = game;
      const s = target.suspicion;
      hp.set(shooter.hp / SHOOTER_MAX_HP, `${shooter.hp.toFixed(0)} HP`);
      targetLine.text = `TARGET  ${"♥".repeat(target.hp)}${"♡".repeat(TARGET_MAX_HP - target.hp)}`;
      suspicion.set(
        s.level,
        s.engaged ? "ENGAGING" : "suspicion",
        s.engaged ? "rgb(220,50,40)" : "rgb(220,170,40)",
      );
      doing.text = `he's ${target.activity ? DOING[target.activity] : "…"}${s.seesShooter ? " · sees you" : ""}`;
      help.text =
        `Up/Down move   Left/Right turn   Space fire   ` +
        `M target walks: ${game.targetMobile ? "on" : "off"}   Tab radar: ${game.radarMode}`;

      const r = radar.rect;
      frame.isVisible = game.radarMode !== "Off" && r.size > 0;
      // GUI pixels are render pixels scaled by the texture's render scale.
      const k = 1 / ui.renderScale;
      frame.left = `${(r.left - 2) * k}px`;
      frame.top = `${(r.top - 2) * k}px`;
      frame.width = frame.height = `${(r.size + 4) * k}px`;
      frameLabel.text = `RADAR · ${game.radarMode}`.toUpperCase();

      banner.isVisible = game.state !== "Playing";
      if (game.state === "Won") {
        bannerText.text = "TARGET DOWN";
        bannerText.color = Color3.FromInts(90, 230, 110).toHexString();
      } else if (game.state === "Lost") {
        bannerText.text = "YOU WERE SPOTTED. AND SHOT.";
        bannerText.color = Color3.FromInts(240, 70, 60).toHexString();
      }
    });
  }
}
