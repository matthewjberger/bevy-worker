use bevy::app::PluginsState;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::window::{
    ExitCondition, RawHandleWrapper, WindowResized, WindowResolution, WindowWrapper,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use serde::{Deserialize, Serialize};
use std::ptr::NonNull;
use std::thread::ThreadId;
use wasm_bindgen::prelude::*;
use web_sys::OffscreenCanvas;

#[wasm_bindgen]
pub struct BevyApp {
    app: App,
}

#[wasm_bindgen]
impl BevyApp {
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: OffscreenCanvas, canvas_size: CanvasSize) -> Self {
        let mut app = App::new();

        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                resolution: WindowResolution::new(canvas_size.width, canvas_size.height),
                ..default()
            }),
            exit_condition: ExitCondition::DontExit,
            ..default()
        }))
        .insert_resource(Controls::default())
        .insert_resource(OrbitCamera::default())
        .init_resource::<FrameStats>()
        .add_systems(PreStartup, setup_added_window)
        .add_systems(Startup, setup)
        .add_systems(Update, (spin, apply_controls, track_stats, orbit_camera));

        app.insert_non_send_resource(canvas);

        BevyApp { app }
    }

    #[wasm_bindgen]
    pub fn update(&mut self) {
        // finish/cleanup here are about plugin initialization, not app exit.
        // Without winit driving the loop we must advance the plugin lifecycle
        // ourselves: once every plugin reports Ready, run finish + cleanup once,
        // and only then start pumping regular update ticks. The webgpu device
        // initializes asynchronously, so Ready only lands a few frames in.
        if self.app.plugins_state() != PluginsState::Cleaned {
            if self.app.plugins_state() == PluginsState::Ready {
                self.app.finish();
                self.app.cleanup();
            }
        } else {
            self.app.update();
        }
    }

    #[wasm_bindgen]
    pub fn resize(&mut self, size: CanvasSize) {
        let world = self.app.world_mut();

        let mut resized_windows = Vec::new();
        let mut query = world.query::<(Entity, &mut Window)>();
        for (entity, mut window) in query.iter_mut(world) {
            window.resolution.set(size.width, size.height);
            resized_windows.push(entity);
        }

        for window in resized_windows {
            world.send_event(WindowResized {
                window,
                width: size.width,
                height: size.height,
            });
        }
    }

    #[wasm_bindgen]
    pub fn set_speed(&mut self, speed: f32) {
        self.app.world_mut().resource_mut::<Controls>().speed = speed;
    }

    #[wasm_bindgen]
    pub fn set_color(&mut self, red: f32, green: f32, blue: f32) {
        self.app.world_mut().resource_mut::<Controls>().color = Color::srgb(red, green, blue);
    }

    #[wasm_bindgen]
    pub fn orbit(&mut self, delta_yaw: f32, delta_pitch: f32) {
        let mut camera = self.app.world_mut().resource_mut::<OrbitCamera>();
        camera.yaw -= delta_yaw * ORBIT_SENSITIVITY;
        camera.pitch =
            (camera.pitch + delta_pitch * ORBIT_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    #[wasm_bindgen]
    pub fn zoom(&mut self, amount: f32) {
        let mut camera = self.app.world_mut().resource_mut::<OrbitCamera>();
        camera.distance = (camera.distance + amount * ZOOM_SENSITIVITY).clamp(2.0, 20.0);
    }

    #[wasm_bindgen]
    pub fn stats(&self) -> Stats {
        let stats = self.app.world().resource::<FrameStats>();
        Stats {
            frames: stats.frames as f64,
            fps: stats.fps,
        }
    }

    #[wasm_bindgen]
    pub fn ready(&self) -> bool {
        self.app
            .world()
            .get_resource::<RenderAdapterInfo>()
            .is_some()
    }

    #[wasm_bindgen]
    pub fn adapter_info(&self) -> AdapterInfo {
        let info = self.app.world().resource::<RenderAdapterInfo>();
        AdapterInfo {
            adapter: info.name.clone(),
            backend: format!("{:?}", info.backend),
        }
    }

    // Reports the name of the JavaScript global scope this wasm module is
    // executing in. In a worker it returns "DedicatedWorkerGlobalScope", on the
    // main thread it would return "Window". This is the direct proof that the
    // Bevy app itself runs off the main thread, not just the canvas.
    #[wasm_bindgen]
    pub fn context(&self) -> String {
        let global = js_sys::global();
        js_sys::Reflect::get(&global, &JsValue::from_str("constructor"))
            .ok()
            .and_then(|constructor| {
                js_sys::Reflect::get(&constructor, &JsValue::from_str("name")).ok()
            })
            .and_then(|name| name.as_string())
            .unwrap_or_else(|| "unknown".to_string())
    }
}

#[derive(Resource, Copy, Clone, Debug, Deserialize, Serialize, tsify_next::Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct CanvasSize {
    width: f32,
    height: f32,
}

#[derive(Copy, Clone, Serialize, Deserialize, tsify_next::Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct Stats {
    frames: f64,
    fps: f32,
}

#[derive(Clone, Serialize, Deserialize, tsify_next::Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct AdapterInfo {
    adapter: String,
    backend: String,
}

#[derive(Resource)]
struct Controls {
    speed: f32,
    color: Color,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            speed: 1.0,
            color: Color::srgb(0.3, 0.5, 0.9),
        }
    }
}

const ORBIT_SENSITIVITY: f32 = 0.005;
const ZOOM_SENSITIVITY: f32 = 0.01;
const PITCH_LIMIT: f32 = 1.5;

#[derive(Resource)]
struct OrbitCamera {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.32,
            distance: 6.3,
        }
    }
}

fn orbit_camera(camera: Res<OrbitCamera>, mut query: Query<&mut Transform, With<Camera3d>>) {
    let eye = Vec3::new(
        camera.distance * camera.pitch.cos() * camera.yaw.sin(),
        camera.distance * camera.pitch.sin(),
        camera.distance * camera.pitch.cos() * camera.yaw.cos(),
    );
    for mut transform in &mut query {
        *transform = Transform::from_translation(eye).looking_at(Vec3::ZERO, Vec3::Y);
    }
}

#[derive(Resource, Default)]
struct FrameStats {
    frames: u64,
    fps: f32,
    accumulator: f32,
    window_frames: u32,
}

#[derive(Resource)]
struct CubeMaterial(Handle<StandardMaterial>);

#[derive(Component)]
struct Spinner;

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    controls: Res<Controls>,
) {
    let material = materials.add(StandardMaterial {
        base_color: controls.color,
        perceptual_roughness: 0.4,
        ..default()
    });
    commands.insert_resource(CubeMaterial(material.clone()));

    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.5, 1.5, 1.5))),
        MeshMaterial3d(material),
        Transform::default(),
        Spinner,
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        Transform::from_xyz(0.0, 2.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spin(time: Res<Time>, controls: Res<Controls>, mut query: Query<&mut Transform, With<Spinner>>) {
    for mut transform in &mut query {
        transform.rotate_y(controls.speed * time.delta_secs());
        transform.rotate_x(controls.speed * 0.4 * time.delta_secs());
    }
}

fn apply_controls(
    controls: Res<Controls>,
    cube: Res<CubeMaterial>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if let Some(material) = materials.get_mut(&cube.0) {
        material.base_color = controls.color;
    }
}

fn track_stats(time: Res<Time>, mut stats: ResMut<FrameStats>) {
    stats.frames += 1;
    stats.accumulator += time.delta_secs();
    stats.window_frames += 1;
    if stats.accumulator >= 0.25 {
        stats.fps = stats.window_frames as f32 / stats.accumulator;
        stats.accumulator = 0.0;
        stats.window_frames = 0;
    }
}

fn setup_added_window(
    mut commands: Commands,
    canvas: NonSendMut<OffscreenCanvas>,
    mut new_windows: Query<Entity, Added<Window>>,
) {
    let Some(entity) = new_windows.iter_mut().next() else {
        return;
    };

    let handle = OffscreenWindowHandle::new(&canvas);

    let handle = RawHandleWrapper::new(&WindowWrapper::new(handle)).expect(
        "to create offscreen raw handle wrapper. If this fails, multiple threads are trying to access the same canvas!",
    );

    commands.entity(entity).insert(handle);
}

pub(crate) struct OffscreenWindowHandle {
    window_handle: raw_window_handle::RawWindowHandle,
    display_handle: raw_window_handle::DisplayHandle<'static>,
    thread_id: ThreadId,
}

impl OffscreenWindowHandle {
    pub(crate) fn new(canvas: &OffscreenCanvas) -> Self {
        let ptr = NonNull::from(canvas).cast();
        let handle = raw_window_handle::WebOffscreenCanvasWindowHandle::new(ptr);
        let window_handle = raw_window_handle::RawWindowHandle::WebOffscreenCanvas(handle);
        let display_handle = raw_window_handle::DisplayHandle::web();

        Self {
            window_handle,
            display_handle,
            thread_id: std::thread::current().id(),
        }
    }
}

// RawHandleWrapper demands Send + Sync, but the OffscreenCanvas handle is not
// thread safe. We assert it manually and back the assertion with a runtime
// guard: the handle is only ever dereferenced on the thread that built it
// (the worker), which is single threaded, so the unsafe promise always holds.
unsafe impl Send for OffscreenWindowHandle {}
unsafe impl Sync for OffscreenWindowHandle {}

impl HasWindowHandle for OffscreenWindowHandle {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        if self.thread_id != std::thread::current().id() {
            return Err(raw_window_handle::HandleError::NotSupported);
        }

        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(self.window_handle) })
    }
}

impl HasDisplayHandle for OffscreenWindowHandle {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(self.display_handle)
    }
}
