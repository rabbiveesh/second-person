// Drives the built game in headless Chromium (software WebGL): loads it, plays a little and
// saves screenshots. Usage: npm run build && npx vite preview --port 4173 & node scripts/smoke.mjs <outdir>
import { chromium } from "@playwright/test";

const out = process.argv[2] ?? ".";
const url = process.env.URL ?? "http://localhost:4173/";
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM, // e.g. a preinstalled Chromium
  args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"],
});
const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("response", (r) => r.status() >= 400 && errors.push(`${r.status()} ${r.url()}`));
page.on("console", (m) => (m.type() === "error" || m.type() === "warning") && errors.push(`${m.type()}: ${m.text()}`));
await page.goto(url);
await page.waitForFunction(() => window.game?.scene?.isReady(), null, { timeout: 60_000 });
await page.waitForTimeout(2500);
await page.screenshot({ path: `${out}/start.png` });

const state = () =>
  page.evaluate(() => {
    const g = window.game;
    return { state: g.state, radar: g.radarMode, activity: g.target.activity, shooter: g.shooter.position.asArray().map((v) => +v.toFixed(2)), fps: g.scene.getEngine().getFps().toFixed(0) };
  });
console.log("start", await state());

// Put the shooter in front of him, then fire and cycle the radar to Full.
await page.evaluate(() => {
  const g = window.game;
  const Vector3 = g.shooter.position.constructor;
  g.target.place(new Vector3(0, 0.9, 0), 0);
  g.shooter.place(new Vector3(2, 0.9, -9), Math.atan2(2, -9));
});
await page.locator("canvas").click();
// Separate frames: the game reads "just pressed" once per fixed step.
await page.keyboard.press("Tab");
await page.waitForTimeout(300);
await page.keyboard.press("Tab");
await page.keyboard.down("Space");
await page.waitForTimeout(60);
await page.keyboard.up("Space");
await page.waitForTimeout(40);
await page.screenshot({ path: `${out}/firing.png` });
await page.keyboard.down("ArrowLeft");
await page.waitForTimeout(500);
await page.keyboard.up("ArrowLeft");
await page.waitForTimeout(1500);
await page.screenshot({ path: `${out}/full-radar.png` });
console.log("after", await state());

// Lose on purpose to see the end banner.
await page.evaluate(() => (window.game.shooter.hp = 0));
await page.waitForTimeout(500);
await page.screenshot({ path: `${out}/lost.png` });
console.log("lost", await state());
console.log(errors.length ? errors.join("\n") : "no console errors");
await browser.close();
