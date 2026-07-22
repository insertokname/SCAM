use crate::app::App;
use crate::config::{INIT_HEIGHT, INIT_WIDTH, RUN_UNCAPPED};
use pixels::{Pixels, SurfaceTexture};
use scamu::devices::nes::Nes;
use scamu::hardware::cartrige::Cartrige;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_time::Instant as WebInstant;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::platform::web::WindowExtWebSys;
use winit::window::Window;

thread_local! {
    static NES_HANDLE: RefCell<Option<Rc<RefCell<Nes>>>> = RefCell::new(None);
    static EMULATION_STARTED: Cell<bool> = Cell::new(false);
    static PENDING_ROM: RefCell<Option<Vec<u8>>> = RefCell::new(None);
    static RESET_TIMING: Cell<bool> = Cell::new(false);
    static PAUSED: Cell<bool> = Cell::new(false);
}

pub(crate) fn set_nes_handle(nes: Rc<RefCell<Nes>>) {
    NES_HANDLE.with(|h| *h.borrow_mut() = Some(nes));
}

#[wasm_bindgen]
pub fn start_emulation() {
    EMULATION_STARTED.with(|f| f.set(true));
}

#[wasm_bindgen]
pub fn set_paused(paused: bool) {
    PAUSED.with(|f| f.set(paused));
}

#[wasm_bindgen]
pub fn reset_timestamp() {
    // Called by JS when the tab becomes visible again (visibilitychange / focus).
    // Sets the flag that makes run_due_ticks() call reset_timing() on its next
    // iteration, which moves emulation_anchor to "now" and zeroes completed_ticks.
    // This discards the time-debt that built up while the tab was hidden, preventing
    // the emulator from trying to catch up with a huge burst of ticks on return.
    RESET_TIMING.with(|f| f.set(true));
}

#[wasm_bindgen]
pub fn load_rom(bytes: &[u8]) -> Result<(), JsValue> {
    Cartrige::from_bytes(bytes)
        .map_err(|err| JsValue::from_str(&format!("rom load failed: {err:?}")))?;

    PENDING_ROM.with(|p| *p.borrow_mut() = Some(bytes.to_vec()));
    RESET_TIMING.with(|f| f.set(true));
    Ok(())
}

pub(crate) fn setup_window(
    window: Arc<Window>,
    pixels_cell: Rc<RefCell<Option<Pixels<'static>>>>,
    _surface_width: u32,
    _surface_height: u32,
) {
    attach_canvas(&window);

    let window_for_pixels = window.clone();
    spawn_local(async move {
        // The emulator always renders 256x240 pixels. Keeping the browser's
        // backing surface at that size lets CSS handle presentation scaling
        // without feeding canvas layout changes back into winit's
        // ResizeObserver. Reconfiguring the surface here on every browser
        // resize also rewrites the canvas width/height attributes, which can
        // create a resize loop when Firefox zoom or responsive mode changes
        // the device-pixel ratio.
        let surface_texture = SurfaceTexture::new(
            INIT_WIDTH as u32,
            INIT_HEIGHT as u32,
            window_for_pixels.clone(),
        );

        let mut pixels =
            pixels::PixelsBuilder::new(INIT_WIDTH as u32, INIT_HEIGHT as u32, surface_texture)
                .surface_texture_format(pixels::wgpu::TextureFormat::Bgra8Unorm)
                .texture_format(pixels::wgpu::TextureFormat::Rgba8Unorm)
                .build_async()
                .await
                .expect("pixels init failed");

        pixels.enable_vsync(true);
        *pixels_cell.borrow_mut() = Some(pixels);
        window_for_pixels.request_redraw();
    });
}

pub(crate) fn resize_surface(
    _pixels_cell: Rc<RefCell<Option<Pixels<'static>>>>,
    _width: u32,
    _height: u32,
) {
    // CSS scales the fixed-resolution canvas on the web. Calling
    // Pixels::resize_surface() would mutate its backing dimensions and feed
    // another size into winit's ResizeObserver.
}

pub(crate) fn set_control_flow(event_loop: &ActiveEventLoop, _next_tick_deadline: WebInstant) {
    if RUN_UNCAPPED {
        event_loop.set_control_flow(ControlFlow::Poll);
    } else {
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

pub(crate) fn maybe_init_audio(_app: &mut App) {}

pub(crate) fn poll_runtime_state(app: &mut App) -> bool {
    if !EMULATION_STARTED.with(|f| f.get()) {
        return false;
    }

    if PAUSED.with(|f| f.get()) {
        return false;
    }

    if let Some(rom_bytes) = PENDING_ROM.with(|p| p.borrow_mut().take()) {
        let cartrige = Cartrige::from_bytes(&rom_bytes)
            .expect("rom validation passed in load_rom but failed here");

        let new_nes = Rc::new(RefCell::new(Nes::new()));
        {
            let mut nes = new_nes.borrow_mut();
            nes.insert_cartrige(cartrige);
            nes.reset();
        }

        app.nes = new_nes.clone();
        app.apu_source.apu = Some(new_nes.borrow().apu.clone());
        set_nes_handle(new_nes);

        app.audio = None;
        app.try_init_audio();

        app.draw_buffer.fill(0);
        app.latched_buffer.fill(0);
    }

    let reset_timing = RESET_TIMING.with(|f| {
        let v = f.get();
        if v {
            f.set(false);
        }
        v
    });

    if reset_timing {
        app.reset_timing();
    }

    true
}

pub(crate) fn handle_redraw(app: &mut App) {
    app.run_due_ticks();

    if let Some(pixels) = app.pixels.borrow_mut().as_mut() {
        let frame = pixels.frame_mut();
        frame.copy_from_slice(app.latched_buffer.as_ref());
        let _ = pixels.render();
    }

    if let Some(window) = &app.window {
        window.request_redraw();
    }
}

fn attach_canvas(window: &Window) {
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
