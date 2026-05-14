use crate::config::{INIT_HEIGHT, INIT_WIDTH, RUN_UNCAPPED};
use crate::platform;
use pixels::Pixels;
use rodio::Source;
use scamu::devices::nes::Nes;
use scamu::hardware::apu::Apu;
use scamu::hardware::constants::clock_rates::{APU_SAMPLE_RATE, MASTER_CLOCK};
use scamu::hardware::constants::controller::buttons;
use scamu::hardware::constants::ppu::COLORS;
use std::cell::RefCell;
use std::num::{NonZeroU16, NonZeroU32};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use web_time::{Duration as WebDuration, Instant as WebInstant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, KeyEvent, StartCause, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const NANOS_PER_SECOND: u128 = 1_000_000_000;
const LAST_VISIBLE_X: u32 = 255;
const LAST_VISIBLE_Y: u32 = 239;
// Hard cap on how many master-clock ticks can be run in a single call to
// run_due_ticks(). Without this, any gap in rAF scheduling (alt-tab, tab
// switch, OS sleep) causes a massive catch-up burst that skips frames and
// can hang the browser. Two frames at ~60 Hz is plenty of slack for normal
// jitter while still being invisible to the player.
const MAX_CATCHUP_TICKS: u64 = MASTER_CLOCK as u64 / 30; // ≈ 2 frames at 60 Hz

pub(crate) struct AudioState {
    _handle: rodio::MixerDeviceSink,
    _player: rodio::Player,
}

#[derive(Default, Clone)]
pub(crate) struct ApuSource {
    pub last_val: f32,
    pub apu: Option<Arc<Mutex<Apu>>>,
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

pub(crate) struct App {
    pub(crate) window: Option<Arc<Window>>,
    pub(crate) pixels: Rc<RefCell<Option<Pixels<'static>>>>,
    emulation_anchor: WebInstant,
    completed_ticks: u64,
    next_tick_deadline: WebInstant,
    pub(crate) nes: Rc<RefCell<Nes>>,
    pub(crate) apu_source: ApuSource,
    pub(crate) audio: Option<AudioState>,
    pub(crate) draw_buffer: Box<[u8; INIT_WIDTH * INIT_HEIGHT * 4]>,
    pub(crate) latched_buffer: Box<[u8; INIT_WIDTH * INIT_HEIGHT * 4]>,
}

impl ApplicationHandler for App {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        match cause {
            StartCause::Init
            | StartCause::ResumeTimeReached { .. }
            | StartCause::WaitCancelled { .. }
            | StartCause::Poll => {
                self.run_due_ticks();
                platform::set_control_flow(event_loop, self.next_tick_deadline);
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
                        .with_inner_size(LogicalSize::new(INIT_WIDTH as f64, INIT_HEIGHT as f64)),
                )
                .unwrap(),
        );

        self.window = Some(window.clone());

        let initial_size = window.inner_size();
        let surface_width = initial_size.width.max(1);
        let surface_height = initial_size.height.max(1);

        platform::setup_window(
            window.clone(),
            self.pixels.clone(),
            surface_width,
            surface_height,
        );
        platform::maybe_init_audio(self);

        self.reset_timing();
        platform::set_control_flow(event_loop, self.next_tick_deadline);

        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        platform::set_control_flow(event_loop, self.next_tick_deadline);
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
                platform::maybe_init_audio(self);

                let pressed = state == ElementState::Pressed;
                if let PhysicalKey::Code(keycode) = code {
                    self.handle_controller_key(keycode, pressed);
                }
            }
            WindowEvent::RedrawRequested => {
                platform::handle_redraw(self);
            }
            _ => {}
        }
    }
}

impl App {
    pub(crate) fn new() -> App {
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

    pub(crate) fn reset_timing(&mut self) {
        self.emulation_anchor = WebInstant::now();
        self.completed_ticks = 0;
        self.next_tick_deadline = self.deadline_for_tick(1);
    }

    pub(crate) fn try_init_audio(&mut self) {
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

    pub(crate) fn run_due_ticks(&mut self) {
        if self.window.is_none() {
            return;
        }

        if !platform::poll_runtime_state(self) {
            return;
        }

        if RUN_UNCAPPED {
            while !self.tick_once() {}
            return;
        }

        let elapsed_nanos = WebInstant::now()
            .saturating_duration_since(self.emulation_anchor)
            .as_nanos();
        let target_ticks = (elapsed_nanos * MASTER_CLOCK as u128 / NANOS_PER_SECOND) as u64;

        // Clamp the catch-up window. If the browser throttled rAF while the
        // tab was hidden (alt-tab, tab switch, OS sleep), elapsed_nanos will be
        // enormous and target_ticks will be millions of ticks ahead of
        // completed_ticks. Running all of them at once causes a huge skip and
        // can lock up the browser. We cap the burst to MAX_CATCHUP_TICKS; any
        // larger gap is silently dropped — the JS side should also call
        // reset_timestamp() on visibilitychange/focus to zero the debt cleanly.
        let target_ticks = target_ticks.min(self.completed_ticks + MAX_CATCHUP_TICKS);

        while self.completed_ticks < target_ticks {
            self.tick_once();
        }

        self.next_tick_deadline = self.deadline_for_tick(self.completed_ticks + 1);
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

    fn handle_controller_key(&mut self, key: KeyCode, pressed: bool) {
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
        }
    }
}
