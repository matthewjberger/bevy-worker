import init, {
  BevyApp,
  type CanvasSize,
  type Stats,
} from "./wasm/bevy_worker.js";

// Explicit wasm-bindgen (target web) initialization. The ?url import hands Vite
// the digested asset path; we feed it to init via module_or_path. No
// vite-plugin-wasm and no bundler-target glue involved.
import wasmPath from "./wasm/bevy_worker_bg.wasm?url";

import { expose, proxy } from "comlink";

const initialized = init({ module_or_path: wasmPath });

const createGame = async (...args: ConstructorParameters<typeof BevyApp>) => {
  await initialized;

  // args carries the transferred OffscreenCanvas and its current pixel size.
  const app = new BevyApp(...args);

  // We dropped winit, so nothing drives the schedule for us. requestAnimationFrame
  // in the worker is the replacement loop (winit uses rAF under the hood too).
  function update() {
    app.update();
    requestAnimationFrame(update);
  }
  requestAnimationFrame(update);

  // Expose worker-side handlers the main thread can call to forward events.
  return proxy({
    resize: (size: CanvasSize) => app.resize(size),
    setSpeed: (speed: number) => app.set_speed(speed),
    setColor: (red: number, green: number, blue: number) =>
      app.set_color(red, green, blue),
    stats: () => app.stats(),
    context: () => app.context(),
  });
};

// The local shape the worker implements. Comlink.wrap<WorkerApi> on the main
// thread applies Remote<> over this, turning each method into a Promise-returning
// proxy call, so we must NOT wrap it here.
export type GameApi = {
  resize: (size: CanvasSize) => void;
  setSpeed: (speed: number) => void;
  setColor: (red: number, green: number, blue: number) => void;
  stats: () => Stats;
  context: () => string;
};

export type WorkerApi = {
  createGame: (
    ...args: ConstructorParameters<typeof BevyApp>
  ) => Promise<GameApi>;
};

expose({ createGame } satisfies WorkerApi);
