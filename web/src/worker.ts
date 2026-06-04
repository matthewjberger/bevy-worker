import init, {
  BevyApp,
  type AdapterInfo,
  type CanvasSize,
  type Stats,
} from "./wasm/bevy_worker.js";

// Explicit wasm-bindgen (target web) initialization. The ?url import hands Vite
// the digested asset path; we feed it to init via module_or_path. No
// vite-plugin-wasm and no bundler-target glue involved.
import wasmPath from "./wasm/bevy_worker_bg.wasm?url";

import { expose, proxy } from "comlink";

const initialized = init({ module_or_path: wasmPath });

const createGame = async (
  canvas: OffscreenCanvas,
  size: CanvasSize,
  events: EventSink,
) => {
  await initialized;

  const app = new BevyApp(canvas, size);

  // We dropped winit, so nothing drives the schedule for us. requestAnimationFrame
  // in the worker is the replacement loop (winit uses rAF under the hood too).
  //
  // The worker also pushes events up over the Comlink callback the page handed in.
  // onReady fires once the render adapter exists, carrying the adapter name only
  // the worker knows; onStats streams the frame counters, throttled to stay well
  // under one message per frame.
  let reportedReady = false;
  let lastStatsPush = 0;

  function update() {
    app.update();

    const info = reportedReady ? undefined : app.adapter_info();
    if (info) {
      reportedReady = true;
      events.onReady(info);
    }

    const now = performance.now();
    if (now - lastStatsPush > 250) {
      lastStatsPush = now;
      events.onStats(app.stats());
    }

    requestAnimationFrame(update);
  }
  requestAnimationFrame(update);

  // Expose worker-side handlers the main thread can call to forward events.
  return proxy({
    resize: (size: CanvasSize) => app.resize(size),
    setSpeed: (speed: number) => app.set_speed(speed),
    setColor: (red: number, green: number, blue: number) =>
      app.set_color(red, green, blue),
    orbit: (deltaYaw: number, deltaPitch: number) =>
      app.orbit(deltaYaw, deltaPitch),
    zoom: (amount: number) => app.zoom(amount),
    stats: () => app.stats(),
    context: () => app.context(),
  });
};

// The local shape the worker implements. Comlink.wrap<WorkerApi> on the main
// thread applies Remote<> over this, turning each method into a Promise-returning
// proxy call, so we must NOT wrap it here.
// The main thread implements these and passes them in (wrapped with Comlink.proxy);
// the worker calls them to push events up. This is the worker -> main direction,
// the counterpart to the main -> worker calls on GameApi.
export type EventSink = {
  onReady: (info: AdapterInfo) => void;
  onStats: (stats: Stats) => void;
};

export type GameApi = {
  resize: (size: CanvasSize) => void;
  setSpeed: (speed: number) => void;
  setColor: (red: number, green: number, blue: number) => void;
  orbit: (deltaYaw: number, deltaPitch: number) => void;
  zoom: (amount: number) => void;
  stats: () => Stats;
  context: () => string;
};

export type WorkerApi = {
  createGame: (
    canvas: OffscreenCanvas,
    size: CanvasSize,
    events: EventSink,
  ) => Promise<GameApi>;
};

expose({ createGame } satisfies WorkerApi);
