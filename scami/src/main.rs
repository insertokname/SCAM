use pixels::{Pixels, SurfaceTexture};
use rodio::Source;
use scamu::devices::nes::Nes;
use scamu::hardware::apu::Apu;
use scamu::hardware::cartrige::Cartrige;
use scamu::hardware::constants::clock_rates::{APU_SAMPLE_RATE, MASTER_CLOCK};
use scamu::hardware::constants::controller::buttons;
use scamu::hardware::constants::ppu::COLORS;
use std::cell::RefCell;
use std::num::{NonZeroU16, NonZeroU32};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use web_time::{Duration as WebDuration, Instant as WebInstant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

#[cfg(target_arch = "wasm32")]
use std::cell::Cell;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;
#[cfg(target_arch = "wasm32")]
use winit::platform::web::{EventLoopExtWebSys, WindowExtWebSys};

const INIT_WIDTH: usize = 256;
const INIT_HEIGHT: usize = 240;
const NANOS_PER_SECOND: u128 = 1_000_000_000;
const LAST_VISIBLE_X: u32 = 255;
const LAST_VISIBLE_Y: u32 = 239;
const RUN_UNCAPPED: bool = false;
// Hard cap on how many master-clock ticks can be run in a single call to
// run_due_ticks(). Without this, any gap in rAF scheduling (alt-tab, tab
// switch, OS sleep) causes a massive catch-up burst that skips frames and
// can hang the browser.  Two frames at ~60 Hz is plenty of slack for normal
// jitter while still being invisible to the player.
const MAX_CATCHUP_TICKS: u64 = MASTER_CLOCK as u64 / 30; // ≈ 2 frames at 60 Hz

struct AudioState {
    _handle: rodio::MixerDeviceSink,
    _player: rodio::Player,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static NES_HANDLE: RefCell<Option<Rc<RefCell<Nes>>>> = RefCell::new(None);
    static EMULATION_STARTED: Cell<bool> = Cell::new(false);
    static PENDING_ROM: RefCell<Option<Vec<u8>>> = RefCell::new(None);
    static RESET_TIMING: Cell<bool> = Cell::new(false);
    static PAUSED: Cell<bool> = Cell::new(false);
}

#[cfg(target_arch = "wasm32")]
fn set_nes_handle(nes: Rc<RefCell<Nes>>) {
    NES_HANDLE.with(|h| *h.borrow_mut() = Some(nes));
}

#[cfg(target_arch = "wasm32")]
fn emulation_started() -> bool {
    EMULATION_STARTED.with(|f| f.get())
}

#[cfg(target_arch = "wasm32")]
fn is_paused() -> bool {
    PAUSED.with(|f| f.get())
}

#[cfg(target_arch = "wasm32")]
fn take_pending_rom() -> Option<Vec<u8>> {
    PENDING_ROM.with(|p| p.borrow_mut().take())
}

#[cfg(target_arch = "wasm32")]
fn take_timing_reset() -> bool {
    RESET_TIMING.with(|f| {
        let v = f.get();
        if v {
            f.set(false);
        }
        v
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_emulation() {
    EMULATION_STARTED.with(|f| f.set(true));
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_paused(paused: bool) {
    PAUSED.with(|f| f.set(paused));
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn reset_timestamp() {
    // Called by JS when the tab becomes visible again (visibilitychange / focus).
    // Sets the flag that makes run_due_ticks() call reset_timing() on its next
    // iteration, which moves emulation_anchor to "now" and zeroes completed_ticks.
    // This discards the time-debt that built up while the tab was hidden, preventing
    // the emulator from trying to catch up with a huge burst of ticks on return.
    RESET_TIMING.with(|f| f.set(true));
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn load_rom(bytes: &[u8]) -> Result<(), JsValue> {
    Cartrige::from_bytes(bytes)
        .map_err(|err| JsValue::from_str(&format!("rom load failed: {err:?}")))?;

    PENDING_ROM.with(|p| *p.borrow_mut() = Some(bytes.to_vec()));
    RESET_TIMING.with(|f| f.set(true));
    Ok(())
}

#[derive(Default, Clone)]
struct ApuSource {
    last_val: f32,
    apu: Option<Arc<Mutex<Apu>>>,
}

impl Iterator for ApuSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let val = self
            .apu
            .as_ref()
            .and_then(|a| a.lock().unwrap().next())
            .unwrap_or(self.last_val);
        self.last_val = val;
        Some(val)
    }
}

impl Source for ApuSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        NonZeroU16::new(1).unwrap()
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        NonZeroU32::new(APU_SAMPLE_RATE as u32).unwrap()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

struct App {
    window: Option<Arc<Window>>,
    pixels: Rc<RefCell<Option<Pixels<'static>>>>,
    emulation_anchor: WebInstant,
    completed_ticks: u64,
    next_tick_deadline: WebInstant,
    nes: Rc<RefCell<Nes>>,
    apu_source: ApuSource,
    audio: Option<AudioState>,
    draw_buffer: Box<[u8; INIT_WIDTH * INIT_HEIGHT * 4]>,
    latched_buffer: Box<[u8; INIT_WIDTH * INIT_HEIGHT * 4]>,
}

impl ApplicationHandler for App {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        match cause {
            StartCause::Init
            | StartCause::ResumeTimeReached { .. }
            | StartCause::WaitCancelled { .. }
            | StartCause::Poll => {
                self.run_due_ticks();
                self.configure_control_flow(event_loop);
            }
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("SCAM")
                        .with_min_inner_size(PhysicalSize::new(
                            INIT_WIDTH as u32,
                            INIT_HEIGHT as u32,
                        ))
                        .with_inner_size(winit::dpi::LogicalSize::new(
                            INIT_WIDTH as f64,
                            INIT_HEIGHT as f64,
                        )),
                )
                .unwrap(),
        );

        #[cfg(target_arch = "wasm32")]
        self.attach_canvas(&window);

        self.window = Some(window.clone());

        let initial_size = window.inner_size();
        let surface_width = initial_size.width.max(1);
        let surface_height = initial_size.height.max(1);

        #[cfg(not(target_arch = "wasm32"))]
        {
            let surface_texture =
                SurfaceTexture::new(surface_width, surface_height, window.clone());
            let mut pixels =
                Pixels::new(INIT_WIDTH as u32, INIT_HEIGHT as u32, surface_texture).unwrap();
            if RUN_UNCAPPED {
                pixels.enable_vsync(false);
            } else {
                pixels.enable_vsync(true);
                pixels.set_present_mode(pixels::wgpu::PresentMode::Fifo);
            }
            *self.pixels.borrow_mut() = Some(pixels);
        }

        #[cfg(target_arch = "wasm32")]
        {
            let pixels_cell = self.pixels.clone();
            let window = window.clone();
            spawn_local(async move {
                let surface_texture =
                    SurfaceTexture::new(surface_width, surface_height, window.clone());

                let mut pixels = pixels::PixelsBuilder::new(
                    INIT_WIDTH as u32,
                    INIT_HEIGHT as u32,
                    surface_texture,
                )
                .surface_texture_format(pixels::wgpu::TextureFormat::Rgba8Unorm)
                .texture_format(pixels::wgpu::TextureFormat::Rgba8Unorm)
                .build_async()
                .await
                .expect("pixels init failed");

                pixels.enable_vsync(true);
                *pixels_cell.borrow_mut() = Some(pixels);
                window.request_redraw();
            });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.audio.is_none() {
                self.try_init_audio();
            }
        }

        self.reset_timing();
        self.configure_control_flow(event_loop);

        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.configure_control_flow(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let window = match &self.window {
            Some(window) if window.id() == window_id => window.clone(),
            _ => return,
        };

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let Some(pixels) = self.pixels.borrow_mut().as_mut() {
                        let _ = pixels.resize_surface(size.width, size.height);
                    }
                }
                window.request_redraw();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: code,
                        state,
                        repeat: false,
                        ..
                    },
                ..
            } => {
                #[cfg(not(target_arch = "wasm32"))]
                if self.audio.is_none() {
                    self.try_init_audio();
                }

                let pressed = state == ElementState::Pressed;
                if let PhysicalKey::Code(keycode) = code {
                    self.handle_controller_key(keycode, pressed);
                }
            }
            WindowEvent::RedrawRequested => {
                #[cfg(target_arch = "wasm32")]
                self.run_due_ticks();

                self.present_buffer();

                #[cfg(target_arch = "wasm32")]
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

impl App {
    fn reset_timing(&mut self) {
        self.emulation_anchor = WebInstant::now();
        self.completed_ticks = 0;
        self.next_tick_deadline = self.deadline_for_tick(1);
    }

    fn try_init_audio(&mut self) {
        let handle = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(h) => h,
            Err(_) => return,
        };
        let player = rodio::Player::connect_new(&handle.mixer());
        player.append(self.apu_source.clone());
        self.audio = Some(AudioState {
            _handle: handle,
            _player: player,
        });
    }

    fn configure_control_flow(&self, event_loop: &ActiveEventLoop) {
        if RUN_UNCAPPED {
            event_loop.set_control_flow(ControlFlow::Poll);
            return;
        }

        #[cfg(target_arch = "wasm32")]
        {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_tick_deadline));
        }
    }

    fn deadline_for_tick(&self, tick_number: u64) -> WebInstant {
        let nanos: u128 = (tick_number as u128 * NANOS_PER_SECOND) / MASTER_CLOCK as u128;
        self.emulation_anchor + WebDuration::from_nanos(nanos.min(u64::MAX as u128) as u64)
    }

    fn tick_once(&mut self) -> bool {
        let mut nes = self.nes.borrow_mut();
        let out = nes.tick();
        self.completed_ticks = self.completed_ticks.saturating_add(1);

        if let Some((x, y, pattern, attrib)) = out {
            let color_index = nes
                .ppu
                .borrow()
                .pallet_memory
                .read_index(attrib as u16, pattern as u16) as usize;

            let color = COLORS[color_index];
            let i = (y as usize * INIT_WIDTH + x as usize) * 4;
            self.draw_buffer[i] = ((color >> 16) & 0xFF) as u8;
            self.draw_buffer[i + 1] = ((color >> 8) & 0xFF) as u8;
            self.draw_buffer[i + 2] = (color & 0xFF) as u8;
            self.draw_buffer[i + 3] = 0xFF;

            if x == LAST_VISIBLE_X && y == LAST_VISIBLE_Y {
                std::mem::swap(&mut self.draw_buffer, &mut self.latched_buffer);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                return true;
            }
        }

        false
    }

    fn run_due_ticks(&mut self) {
        if self.window.is_none() {
            return;
        }

        #[cfg(target_arch = "wasm32")]
        {
            if !emulation_started() {
                return;
            }

            if is_paused() {
                return;
            }

            if let Some(rom_bytes) = take_pending_rom() {
                let cartrige = Cartrige::from_bytes(&rom_bytes)
                    .expect("rom validation passed in load_rom but failed here");

                let new_nes = Rc::new(RefCell::new(Nes::new()));
                {
                    let mut nes = new_nes.borrow_mut();
                    nes.insert_cartrige(cartrige);
                    nes.reset();
                }

                self.nes = new_nes.clone();

                let new_apu = new_nes.borrow().apu.clone();
                self.apu_source.apu = Some(new_apu);

                set_nes_handle(new_nes);

                self.audio = None;
                self.try_init_audio();

                self.draw_buffer.iter_mut().for_each(|b| *b = 0);
                self.latched_buffer.iter_mut().for_each(|b| *b = 0);
            }

            if take_timing_reset() {
                self.reset_timing();
            }
        }

        if RUN_UNCAPPED {
            while !self.tick_once() {}
            return;
        }

        let elapsed_nanos = WebInstant::now()
            .saturating_duration_since(self.emulation_anchor)
            .as_nanos();
        let target_ticks = (elapsed_nanos * MASTER_CLOCK as u128 / NANOS_PER_SECOND) as u64;

        // Clamp the catch-up window.  If the browser throttled rAF while the
        // tab was hidden (alt-tab, tab switch, OS sleep), elapsed_nanos will be
        // enormous and target_ticks will be millions of ticks ahead of
        // completed_ticks.  Running all of them at once causes a huge skip and
        // can lock up the browser.  We cap the burst to MAX_CATCHUP_TICKS; any
        // larger gap is silently dropped — the JS side should also call
        // reset_timestamp() on visibilitychange/focus to zero the debt cleanly.
        let target_ticks = target_ticks.min(self.completed_ticks + MAX_CATCHUP_TICKS);

        while self.completed_ticks < target_ticks {
            self.tick_once();
        }

        self.next_tick_deadline = self.deadline_for_tick(self.completed_ticks + 1);
    }

    fn handle_controller_key(&mut self, key: KeyCode, pressed: bool) -> bool {
        let button = match key {
            KeyCode::KeyW | KeyCode::ArrowUp => Some(buttons::UP),
            KeyCode::KeyA | KeyCode::ArrowLeft => Some(buttons::LEFT),
            KeyCode::KeyS | KeyCode::ArrowDown => Some(buttons::DOWN),
            KeyCode::KeyD | KeyCode::ArrowRight => Some(buttons::RIGHT),
            KeyCode::KeyZ | KeyCode::KeyJ => Some(buttons::A),
            KeyCode::KeyX | KeyCode::KeyK => Some(buttons::B),
            KeyCode::KeyC | KeyCode::Enter => Some(buttons::START),
            KeyCode::KeyV | KeyCode::ShiftRight => Some(buttons::SELECT),
            _ => None,
        };

        if let Some(button) = button {
            self.nes
                .borrow_mut()
                .bus
                .set_controller_button(0, button, pressed);
            return true;
        }

        false
    }

    fn present_buffer(&mut self) {
        if let Some(pixels) = self.pixels.borrow_mut().as_mut() {
            let frame = pixels.frame_mut();
            frame.copy_from_slice(self.latched_buffer.as_ref());
            let _ = pixels.render();
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn attach_canvas(&self, window: &Window) {
        let Some(canvas) = window.canvas() else {
            return;
        };

        canvas.set_id("nes-canvas");
        canvas.set_class_name("nes-canvas");

        let Some(document) = web_sys::window().and_then(|win| win.document()) else {
            return;
        };

        if let Some(host) = document.get_element_by_id("canvas-host") {
            if canvas.parent_node().is_none() {
                let _ = host.append_child(&canvas);
            }
            return;
        }

        if let Some(body) = document.body() {
            if canvas.parent_node().is_none() {
                let _ = body.append_child(&canvas);
            }
        }
    }
}

fn build_app() -> App {
    let now = WebInstant::now();
    let nes = Rc::new(RefCell::new(Nes::new()));
    let apu = nes.borrow().apu.clone();
    let mut apu_source = ApuSource::default();
    apu_source.apu = Some(apu);

    App {
        window: None,
        pixels: Rc::new(RefCell::new(None)),
        emulation_anchor: now,
        completed_ticks: 0,
        next_tick_deadline: now,
        nes,
        apu_source,
        audio: None,
        draw_buffer: Box::new([0; INIT_WIDTH * INIT_HEIGHT * 4]),
        latched_buffer: Box::new([0; INIT_WIDTH * INIT_HEIGHT * 4]),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let event_loop = EventLoop::new().unwrap();
    let mut app = build_app();

    // let cartrige = Cartrige::from_bytes(include_bytes!("./nestest.nes")).unwrap();
    // let cartrige = Cartrige::from_bytes(include_bytes!("./AccuracyCoin.nes")).unwrap();
    // let cartrige = Cartrige::from_bytes(include_bytes!("./gitignored_games/smb.nes")).unwrap();
    // let cartrige = Cartrige::from_bytes(include_bytes!("./gitignored_games/pacman.nes")).unwrap();
    let cartrige = Cartrige::from_bytes(include_bytes!("./gitignored_games/dk.nes")).unwrap();
    // let cartrige = Cartrige::from_bytes(include_bytes!("./gitignored_games/ic.nes")).unwrap();
    // let cartrige = Cartrige::from_bytes(include_bytes!("./gitignored_games/tetris-73.nes")).unwrap();

    {
        let mut nes = app.nes.borrow_mut();
        nes.insert_cartrige(cartrige);
        nes.reset();
    }

    event_loop.run_app(&mut app).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let event_loop = EventLoop::new()
        .map_err(|err| JsValue::from_str(&format!("event loop init failed: {err:?}")))?;
    let app = build_app();
    set_nes_handle(app.nes.clone());

    event_loop.spawn_app(app);
    Ok(())
}