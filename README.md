# bevy-worker

A minimal [Bevy](https://bevyengine.org) `0.16` app that runs in a web worker via WebAssembly. No `winit`, and no Bevy code on the main thread: the worker owns an `OffscreenCanvas`, drives the schedule with `requestAnimationFrame`, and renders through WebGPU. The main thread only transfers the canvas and forwards events over [Comlink](https://github.com/GoogleChromeLabs/comlink).

![A lit, spinning cube rendered by Bevy inside a web worker, with the control panel reporting the worker scope, fps, and a picked face marker.](docs/screenshot.png)

Based on Nick Babcock's [write-up on running a Bevy app off the main thread](https://nickb.dev/blog/a-bevy-app-entirely-off-the-main-thread/).

For the same architecture without an engine, see [webgpu-worker](https://github.com/matthewjberger/webgpu-worker): the same worker, build, and deploy setup, but rendering with wgpu from scratch instead of Bevy. [webgpu-worker-leptos](https://github.com/matthewjberger/webgpu-worker-leptos) takes that wgpu version further and drops the TypeScript page and Comlink for an all-Rust frontend with [Leptos](https://leptos.dev) and a shared `protocol` crate.

## Live demo

[matthewberger.dev/bevy-worker](https://matthewberger.dev/bevy-worker/). Needs a browser with WebGPU and `OffscreenCanvas`-in-workers support (Chromium 113+, Firefox 141+).

## Proving the work runs off the main thread

The page renders a spinning 3D cube and a control panel that demonstrates the cube, ECS, and renderer all live in the worker:

- The wasm module reports its own JavaScript global scope (`DedicatedWorkerGlobalScope`), so the Bevy code itself confirms where it runs.
- A "Jam main thread for 3 s" button synchronously blocks the page. The main-thread heartbeat counter freezes, but the cube keeps spinning and the panel reports how many frames Bevy advanced while the page was stalled.
- Dragging orbits the camera and the wheel zooms; rotation-speed and color controls also send events into the worker and update the running scene.

## How it works

- `src/lib.rs` exposes a `BevyApp` (`new` / `update` / `resize` / control methods) through `wasm-bindgen` (`--target web`). The constructor takes the transferred `OffscreenCanvas` and its pixel size.
- Without `winit` nothing advances Bevy's plugin lifecycle, so `update()` pumps `plugins_state()` until `Ready`, runs `finish()` + `cleanup()` once, then ticks the schedule. The WebGPU device initializes asynchronously, so `Ready` lands a few frames in.
- A `PreStartup` system wraps the canvas in a custom `OffscreenWindowHandle` and attaches it as a `RawHandleWrapper`, bridging Bevy's renderer to the canvas. The handle is `unsafe impl Send + Sync`, guarded at runtime by a `ThreadId` check; the worker is single-threaded, so the promise always holds.
- `web/src/worker.ts` initializes the module explicitly with `init({ module_or_path })` (a `?url` import, no `vite-plugin-wasm`) and drives `app.update()` from `requestAnimationFrame`.
- `web/src/main.ts` transfers the canvas with `Comlink.transfer` and forwards control and resize events.

## The page / worker bridge

All communication runs over [Comlink](https://github.com/GoogleChromeLabs/comlink), in both directions:

- **Page to worker:** the worker exposes methods the page calls to forward events. The offscreen canvas can't receive DOM input, so the page captures pointer drag (orbit) and wheel (zoom) on the placeholder canvas, coalesces them to at most one message per frame, and forwards them alongside the resize, rotation-speed, and color controls.
- **Worker to page:** the page hands the worker a callback wrapped with `Comlink.proxy`, which the worker calls to push state up. It fires once with the GPU adapter name and backend that only the worker knows (shown as the panel's "renderer" line), and streams the frame counters so the fps and frame readout is push-driven rather than polled. The jam measurement still pulls `stats()` on demand for an exact before and after.

## Quickstart

Tooling is pinned in [`mise.toml`](mise.toml): node, rust with the `wasm32-unknown-unknown` target, [`wasm-bindgen`](https://github.com/rustwasm/wasm-bindgen), and [`wasm-opt`](https://github.com/WebAssembly/binaryen). Install [mise](https://mise.jdx.dev) and [just](https://github.com/casey/just), then:

```bash
mise install     # fetch the pinned toolchain
just run         # build, optimize, and serve at http://localhost:5173
```

Run `just` with no arguments to list every recipe. The wasm pipeline is a bare `cargo build` -> `wasm-bindgen --target web` -> `wasm-opt -Oz`, the way [Nick describes deconstructing wasm-pack](https://nickb.dev/blog/life-after-wasm-pack-an-opinionated-deconstruction/).

## License

Dual-licensed under MIT or Apache-2.0, at your option.
