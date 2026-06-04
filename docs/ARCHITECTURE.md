# Architecture

This document explains how `bevy-worker` is put together: the thread split, how
a full Bevy app is driven without `winit`, how the `OffscreenCanvas` is bridged
to Bevy's renderer, the Comlink bridge between the page and the worker, and the
build pipeline. File paths are relative to the repository root, and key
functions and types are named so you can grep for them.

## The one idea

A complete [Bevy](https://bevyengine.org) `0.16` app — ECS, render graph, PBR —
runs inside a **web worker**, never on the browser's main thread. The worker
owns an `OffscreenCanvas`, drives the Bevy schedule with `requestAnimationFrame`,
and renders a lit, spinning cube through WebGPU. The main thread is a plain
**TypeScript** page that transfers the canvas to the worker once, forwards input
events, and displays stats. There is no `winit`.

The payoff is the "Jam main thread for 3 s" button: it busy-loops the main
thread, and Bevy keeps rendering at full framerate because the entire engine
lives on another thread.

This follows Nick Babcock's
[write-up on running a Bevy app off the main thread](https://nickb.dev/blog/a-bevy-app-entirely-off-the-main-thread/).
For the same architecture without an engine, see
[webgpu-worker](https://github.com/matthewjberger/webgpu-worker) (raw wgpu, same
TypeScript + Comlink frontend) and its all-Rust sibling
[webgpu-worker-leptos](https://github.com/matthewjberger/webgpu-worker-leptos).
[bevy-worker-leptos](https://github.com/matthewjberger/bevy-worker-leptos) is
this project with the TypeScript page and Comlink replaced by an all-Rust
[Leptos](https://leptos.dev) frontend.

## Layout

A single Rust crate plus a TypeScript web frontend — not a workspace.

| Path | Runs on | Role |
|---|---|---|
| `src/lib.rs` | worker | The whole Bevy app: `BevyApp` wasm-bindgen handle, ECS systems, the `OffscreenWindowHandle` bridge, picking. |
| `web/src/worker.ts` | worker | Bootstraps the wasm module, drives `app.update()` with `requestAnimationFrame`, exposes the worker API over Comlink. |
| `web/src/main.ts` | main thread | Transfers the canvas, spawns the worker, captures input, displays stats. |
| `web/index.html` | main thread | The page: placeholder canvas + control panel (plain HTML/CSS). |

The Rust crate is `crate-type = ["cdylib", "rlib"]` and depends on `bevy` with a
trimmed feature set: `bevy_core_pipeline`, `bevy_pbr`, `bevy_render`,
`bevy_window`, and `webgpu`. It compiles to one wasm module; `wasm-bindgen`
generates the JS glue and TypeScript types (`tsify-next` derives the shared
structs) that `worker.ts` imports.

## The thread split

```
┌─────────────────────────── MAIN THREAD ──────────────────────────────┐
│  TypeScript page  (web/)                                              │
│                                                                       │
│   web/index.html   placeholder <canvas> + control panel              │
│   web/src/main.ts  transfer canvas, spawn worker, capture input      │
│                                                                       │
│   transferControlToOffscreen()  ── OffscreenCanvas ──┐               │
│   Comlink.wrap<WorkerApi>(worker)                     │               │
│   game.orbit(), game.zoom(), game.pick() ────────────┼──────────────▶ │
│   sink.onReady(), sink.onStats()         ◀───────────┘               │
└───────────────────────────────────────────────────────────────────────┘
                              │  Comlink RPC over postMessage
                              ▼
┌─────────────────────────── WEB WORKER ───────────────────────────────┐
│  web/src/worker.ts                                                    │
│   • init({ module_or_path }) — explicit wasm-bindgen init            │
│   • new BevyApp(canvas, size)                                         │
│   • requestAnimationFrame(update) → app.update()                     │
│   • Comlink.expose({ createGame })                                    │
│                                                                       │
│  src/lib.rs  (the Bevy app, all in the worker)                        │
│   • BevyApp::update() pumps the plugin lifecycle, then ticks          │
│   • setup_added_window installs the OffscreenWindowHandle             │
│   • DefaultPlugins → bevy_render → wgpu → the OffscreenCanvas         │
└───────────────────────────────────────────────────────────────────────┘
```

After the one-time canvas transfer, the main thread can never draw to the canvas
again. From then on the two sides communicate only through Comlink.

## The Comlink bridge

[Comlink](https://github.com/GoogleChromeLabs/comlink) turns method calls into
async `postMessage` round-trips, so there is no hand-written message enum — the
contract is the TypeScript interfaces in `web/src/worker.ts`. There are two
directions:

**Page → worker** is the `GameApi` (also `WorkerApi.createGame`). The worker
`expose`s these and the page `wrap`s them, so each call becomes a
Promise-returning RPC:

```ts
type GameApi = {
  resize, setSpeed, setColor, orbit, zoom,   // one-way fire-and-forget in practice
  pick: (x, y) => PickResult | undefined,     // request/response (awaited)
  stats: () => Stats,                         // request/response (the jam poll)
  context: () => string,                      // request/response (one-time)
};
```

Each maps directly onto a `#[wasm_bindgen]` method on `BevyApp`
(`resize`, `set_speed`, `set_color`, `orbit`, `zoom`, `pick`, `stats`,
`context`).

**Worker → page** is the `EventSink`. The page builds it and passes it into
`createGame` wrapped in `Comlink.proxy`, so the worker holds a callable handle
and can push up:

```ts
type EventSink = {
  onReady: (info: AdapterInfo) => void;   // fires once, when the GPU adapter exists
  onStats: (stats: Stats) => void;        // streamed, throttled to every 250 ms
};
```

`onReady` carries the GPU adapter name and backend that only the worker knows
(the panel's "renderer" line). `onStats` makes the fps/frame readout push-driven
instead of polled. The jam button still *pulls* `stats()` on demand so it gets
an exact before/after count.

The shared data types (`CanvasSize`, `Stats`, `AdapterInfo`, `PickResult`) are
Rust structs in `src/lib.rs` deriving `tsify_next::Tsify`, so `wasm-bindgen`
emits matching TypeScript definitions and neither side hand-writes the shapes.

## Startup handshake

1. **`main.ts` runs.** It sizes the canvas by `devicePixelRatio`, calls
   `canvas.transferControlToOffscreen()`, and spawns the worker as an ES module
   (`new Worker(new URL("./worker.ts", …), { type: "module" })`). It wraps the
   worker with `Comlink.wrap<WorkerApi>`.
2. **`worker.ts` starts wasm init early but does not block.** At module top it
   kicks off `const initialized = init({ module_or_path: wasmPath })`, where
   `wasmPath` comes from a Vite `?url` import (no `vite-plugin-wasm`).
3. **The page calls `createGame`**, passing the transferred canvas
   (`Comlink.transfer(offscreenCanvas, [offscreenCanvas])`), the pixel size, and
   `Comlink.proxy(sink)`.
4. **`createGame` awaits `initialized`, then `new BevyApp(canvas, size)`.** The
   constructor builds the Bevy `App`, sets the primary window resolution, inserts
   the `Controls`/`OrbitCamera`/`Pick`/`FrameStats` resources, registers the
   systems, and stashes the canvas as a non-send resource.
5. **The worker starts its own `requestAnimationFrame(update)` loop.** Each tick
   calls `app.update()`, fires `onReady` once the adapter is available, and
   throttles `onStats` to every 250 ms.
6. **The page gets the `GameApi` proxy back**, queries `context()` (which returns
   `"DedicatedWorkerGlobalScope"` — proof the engine runs off-thread), wires up
   input listeners, and starts its own main-thread heartbeat loop.

## Driving Bevy without winit

This is the part unique to running Bevy in a worker. Normally `winit` owns the
event loop and calls `app.update()` for you, and the plugin lifecycle advances
as windowing events flow. There is no `winit` here, so `BevyApp::update()` drives
the lifecycle by hand:

```rust
if self.app.plugins_state() != PluginsState::Cleaned {
    if self.app.plugins_state() == PluginsState::Ready {
        self.app.finish();
        self.app.cleanup();
    }
} else {
    self.app.update();
}
```

The plugin state machine walks `Init → Loaded → Ready → Cleaned`, one transition
per call. Crucially the WebGPU device initializes **asynchronously**, so `Ready`
only lands a few frames in. Until everything reports `Ready`, the worker's rAF
loop is just pumping the state machine; once it does, `finish()` + `cleanup()`
run once, and only then does each tick become a real schedule update. The
worker's `update()` calls this every frame regardless — the branch inside
decides whether it's still initializing or steady-state.

## Bridging the canvas to wgpu

Bevy's renderer reaches the GPU through `bevy_window`'s `RawHandleWrapper`, which
expects a window handle. There is no real window, so `bevy-worker` fabricates one
from the `OffscreenCanvas`.

The `setup_added_window` system (`PreStartup`) waits for Bevy's `WindowPlugin` to
spawn the primary `Window` entity, then attaches a handle to it:

```rust
fn setup_added_window(mut commands, canvas: NonSendMut<OffscreenCanvas>, new_windows: Query<Entity, Added<Window>>) {
    let entity = …;                                  // the primary window
    let handle = OffscreenWindowHandle::new(&canvas);
    let handle = RawHandleWrapper::new(&WindowWrapper::new(handle)).expect(…);
    commands.entity(entity).insert(handle);
}
```

`OffscreenWindowHandle` (in `src/lib.rs`) implements `HasWindowHandle` and
`HasDisplayHandle`. It wraps a `RawWindowHandle::WebOffscreenCanvas` built from a
`NonNull` pointer to the canvas, plus a web `DisplayHandle`. From there Bevy's
`bevy_render` calls `create_surface` on it exactly as it would for a native
window, and renders into the offscreen canvas.

**The unsafe bit.** `RawHandleWrapper` requires `Send + Sync`, but a JS
`OffscreenCanvas` pointer is neither. The wrapper asserts both with
`unsafe impl Send/Sync`, and backs the assertion with a runtime guard: it records
the `ThreadId` it was built on, and `window_handle()` returns
`HandleError::NotSupported` if it is ever dereferenced from a different thread.
The worker is single-threaded, so the handle is only ever touched on its home
thread and the promise always holds. The canvas itself is stored with
`insert_non_send_resource` so Bevy never tries to move it across threads.

This whole apparatus exists only to satisfy Bevy's window-shaped renderer.
[webgpu-worker](https://github.com/matthewjberger/webgpu-worker) talks to wgpu
directly via `wgpu::SurfaceTarget::OffscreenCanvas` and needs none of it.

## The scene and systems

`setup` (`Startup`) builds the scene: a `Cuboid` (1.5³) tagged `Spinner` with a
`StandardMaterial`, a child unlit sphere tagged `PickMarker` (hidden until
something is picked), a `DirectionalLight`, and a `Camera3d` with
`Tonemapping::None`. A `CubeMaterial` resource keeps the cube's material handle
so color changes can reach it.

The `Update` systems:

- `spin` — rotates the `Spinner` on two axes scaled by `Controls.speed`.
- `apply_controls` — pushes `Controls.color` onto the cube material.
- `track_stats` — counts frames and computes fps over a 0.25 s window.
- `orbit_camera` — positions the camera from `OrbitCamera` (yaw/pitch/distance
  spherical coordinates).
- `apply_pick` — moves the marker sphere to the last pick point and shows it.

## Input handling and coalescing

The offscreen canvas can't receive DOM events, so `main.ts` captures them on the
placeholder canvas. Pointer-move and wheel handlers accumulate into `pendingYaw`,
`pendingPitch`, `pendingZoom` rather than messaging immediately. A main-thread
`requestAnimationFrame` tick flushes them once per frame — at most one `orbit`
and one `zoom` call per frame regardless of event volume — and increments the
`heartbeat` counter that visibly freezes during the jam test.

A `pointerup` that moved less than 4 px is treated as a click: the position is
converted to normalized device coordinates and sent through `game.pick(x, y)`.
A `ResizeObserver` forwards DPR-scaled `game.resize(...)` on layout changes.

## Picking

Picking is a CPU ray cast inside the worker — no GPU readback. `BevyApp::pick`:

1. queries the `Camera` and its `GlobalTransform` and builds a world-space ray
   from the click via `camera.viewport_to_world`,
2. transforms the ray into the cube's local space with the inverse of the
   `Spinner`'s model matrix,
3. runs a slab ray/AABB intersection (`ray_cube_hit`) against the half-extent
   cube, returning the hit point and face name (`+X`, `-Y`, …),
4. stores the point in the `Pick` resource so `apply_pick` shows the marker next
   frame, and returns a `PickResult` to the page.

## Per-frame data flow

```
MAIN THREAD                                    WEB WORKER
───────────                                    ──────────
pointer/wheel events
    │ accumulate into pending*
    ▼
rAF tick (once per frame)
    │ game.orbit(), game.zoom()  ────────────▶ app.orbit() / app.zoom()
    │ heartbeat += 1
                                               rAF loop (once per frame)
                                                   │ app.update()
                                                   │   pump lifecycle OR tick schedule
                                                   │   spin / orbit_camera / … + render
                                                   │ every 250 ms:
    fps / frames  ◀───────── sink.onStats(stats) ──┘

click ── game.pick(x,y) ────────────────────▶ BevyApp::pick (ray/AABB)
    pick text  ◀──────── (PickResult return) ──┘

jam button:
  game.stats() ──────────────────────────────▶ BevyApp::stats()
  busy-loop 3s  (worker keeps rendering throughout)
  game.stats() ──────────────────────────────▶ BevyApp::stats()
  report advanced frames
```

The two `requestAnimationFrame` loops are independent. The main thread's only
batches input and ticks the heartbeat; the worker's drives Bevy. Blocking the
former does nothing to the latter — which is the entire demonstration.

## Build pipeline

`just run` (`justfile`):

1. `cargo build --release --target wasm32-unknown-unknown` — compile the crate
   to wasm.
2. `wasm-bindgen --target web --out-dir web/src/wasm --out-name bevy_worker …` —
   emit the JS glue and `.d.ts` types into `web/src/wasm/`.
3. `wasm-opt -Oz …` — shrink the wasm in place.
4. `cd web; npm run dev` — Vite serves the page (and bundles `worker.ts` as an ES
   module worker) at `http://localhost:5173`.

The wasm build is a bare `cargo build` → `wasm-bindgen --target web` →
`wasm-opt -Oz`, deliberately without `wasm-pack` or `vite-plugin-wasm`; the
worker loads the module with an explicit `init({ module_or_path })` against a
`?url` asset path. The GitHub Pages deploy (`.github/workflows/deploy.yml`) runs
the same wasm steps, then `npm run build`, and publishes `web/dist`.

Requires a browser with WebGPU and `OffscreenCanvas`-in-workers support
(Chromium 113+, Firefox 141+).

## Why it is shaped this way

- **A real engine in a worker, not a bespoke renderer** — the point is that an
  entire Bevy app, unmodified in spirit, can live off the main thread. The cost
  is the winit-replacement plumbing below.
- **Manual plugin-lifecycle pumping** — without winit nothing advances Bevy's
  startup, so the worker's rAF loop does it explicitly until the async WebGPU
  device is `Ready`.
- **`OffscreenWindowHandle` with a `ThreadId` guard** — Bevy's renderer is
  window-shaped and demands a `Send + Sync` handle the canvas can't provide. The
  unsafe assertion is made safe by single-threadedness plus a runtime check.
  Raw wgpu would skip this entirely.
- **Comlink instead of a message enum** — RPC method calls and proxied callbacks
  keep the TS side ergonomic; the tradeoff is that the page and worker are
  coupled at the API surface, with no shared compile-time type guarantee across
  the language boundary (the all-Rust
  [bevy-worker-leptos](https://github.com/matthewjberger/bevy-worker-leptos)
  closes that gap with a shared `protocol` crate).
</content>
