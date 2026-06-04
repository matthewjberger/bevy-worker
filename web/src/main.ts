import * as Comlink from "comlink";
import type { WorkerApi } from "./worker";

const canvas = document.getElementById("canvas") as HTMLCanvasElement;
const container = document.getElementById("container")!;

const contextEl = document.getElementById("context")!;
const fpsEl = document.getElementById("fps")!;
const framesEl = document.getElementById("frames")!;
const heartbeatEl = document.getElementById("heartbeat")!;
const mainStatEl = document.getElementById("mainStat")!;
const speedEl = document.getElementById("speed") as HTMLInputElement;
const speedValueEl = document.getElementById("speedValue")!;
const colorEl = document.getElementById("color") as HTMLInputElement;
const jamEl = document.getElementById("jam") as HTMLButtonElement;
const jamResultEl = document.getElementById("jamResult")!;

const dpr = window.devicePixelRatio;
const bounds = container.getBoundingClientRect();
canvas.width = bounds.width * dpr;
canvas.height = bounds.height * dpr;

// Hand the canvas to the worker. After this call the main thread can no longer
// draw to it; the worker (and Bevy's renderer) owns it.
const offscreenCanvas = canvas.transferControlToOffscreen();

const webWorker = new Worker(new URL("./worker.ts", import.meta.url), {
  type: "module",
});

const worker = Comlink.wrap<WorkerApi>(webWorker);

const hexToRgb = (hex: string): [number, number, number] => [
  parseInt(hex.slice(1, 3), 16) / 255,
  parseInt(hex.slice(3, 5), 16) / 255,
  parseInt(hex.slice(5, 7), 16) / 255,
];

const initializeApp = async () => {
  const game = await worker.createGame(
    Comlink.transfer(offscreenCanvas, [offscreenCanvas]),
    { width: bounds.width * dpr, height: bounds.height * dpr },
  );

  // Ask the wasm module which JS global scope it is running in. This is the
  // proof: the Bevy app reports "DedicatedWorkerGlobalScope", not "Window".
  contextEl.textContent = `${await game.context()} (off the main thread)`;

  // Main-thread heartbeat. This freezes the instant the main thread is blocked,
  // unlike the worker frame counter, which keeps climbing.
  let heartbeat = 0;
  const tick = () => {
    heartbeat += 1;
    heartbeatEl.textContent = String(heartbeat);
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);

  // Poll the worker for its frame count / fps and render them in the panel.
  setInterval(async () => {
    const stats = await game.stats();
    fpsEl.textContent = stats.fps.toFixed(0);
    framesEl.textContent = stats.frames.toLocaleString();
  }, 150);

  speedEl.addEventListener("input", () => {
    speedValueEl.textContent = Number(speedEl.value).toFixed(1);
    game.setSpeed(Number(speedEl.value));
  });

  colorEl.addEventListener("input", () => {
    const [r, g, b] = hexToRgb(colorEl.value);
    game.setColor(r, g, b);
  });

  // Synchronously block the main thread. The DOM and main-thread
  // heartbeat freeze for the full duration, but the worker keeps rendering the
  // spinning cube. Afterwards we report how many frames Bevy advanced while the
  // main thread was completely stalled.
  jamEl.addEventListener("click", async () => {
    const blockMs = 3000;
    const before = await game.stats();

    jamResultEl.textContent = "";
    mainStatEl.classList.add("jammed");

    const start = performance.now();
    while (performance.now() - start < blockMs) {
      // Busy-wait, fully occupying the main thread.
    }
    const blocked = performance.now() - start;

    const after = await game.stats();
    mainStatEl.classList.remove("jammed");
    jamResultEl.textContent = `Main thread blocked ${Math.round(
      blocked,
    )} ms. Bevy advanced ${(
      after.frames - before.frames
    ).toLocaleString()} frames meanwhile.`;
  });

  // Keep the canvas matched to its container, forwarding the new pixel size to
  // the worker so Bevy can reconfigure its render surface.
  let resizeObserverAF = 0;
  const ro = new ResizeObserver(() => {
    const bounds = container.getBoundingClientRect();
    cancelAnimationFrame(resizeObserverAF);
    resizeObserverAF = requestAnimationFrame(() => {
      game.resize({
        width: bounds.width * window.devicePixelRatio,
        height: bounds.height * window.devicePixelRatio,
      });
    });
  });
  ro.observe(container);
};

initializeApp();
