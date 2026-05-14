use crate::app::App;
use crate::config::{INIT_HEIGHT, INIT_WIDTH, RUN_UNCAPPED};
use pixels::{Pixels, SurfaceTexture};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use web_time::Instant as WebInstant;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::Window;

pub(crate) fn setup_window(
    window: Arc<Window>,
    pixels_cell: Rc<RefCell<Option<Pixels<'static>>>>,
    surface_width: u32,
    surface_height: u32,
) {
    let surface_texture = SurfaceTexture::new(surface_width, surface_height, window);
    let mut pixels = Pixels::new(INIT_WIDTH as u32, INIT_HEIGHT as u32, surface_texture).unwrap();

    if RUN_UNCAPPED {
        pixels.enable_vsync(false);
    } else {
        pixels.enable_vsync(true);
        pixels.set_present_mode(pixels::wgpu::PresentMode::Fifo);
    }

    *pixels_cell.borrow_mut() = Some(pixels);
}

pub(crate) fn set_control_flow(event_loop: &ActiveEventLoop, next_tick_deadline: WebInstant) {
    if RUN_UNCAPPED {
        event_loop.set_control_flow(ControlFlow::Poll);
    } else {
        event_loop.set_control_flow(ControlFlow::WaitUntil(next_tick_deadline));
    }
}

pub(crate) fn maybe_init_audio(app: &mut App) {
    if app.audio.is_none() {
        app.try_init_audio();
    }
}

pub(crate) fn poll_runtime_state(_app: &mut App) -> bool {
    true
}

pub(crate) fn handle_redraw(app: &mut App) {
    if let Some(pixels) = app.pixels.borrow_mut().as_mut() {
        let frame = pixels.frame_mut();
        frame.copy_from_slice(app.latched_buffer.as_ref());
        let _ = pixels.render();
    }
}
