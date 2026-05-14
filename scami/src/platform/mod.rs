#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;

use crate::app::App;
use pixels::Pixels;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use web_time::Instant as WebInstant;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

#[cfg(target_arch = "wasm32")]
use scamu::devices::nes::Nes;

pub(crate) fn setup_window(
    window: Arc<Window>,
    pixels_cell: Rc<RefCell<Option<Pixels<'static>>>>,
    surface_width: u32,
    surface_height: u32,
) {
    #[cfg(target_arch = "wasm32")]
    wasm::setup_window(window, pixels_cell, surface_width, surface_height);
    #[cfg(not(target_arch = "wasm32"))]
    native::setup_window(window, pixels_cell, surface_width, surface_height);
}

pub(crate) fn set_control_flow(event_loop: &ActiveEventLoop, next_tick_deadline: WebInstant) {
    #[cfg(target_arch = "wasm32")]
    wasm::set_control_flow(event_loop, next_tick_deadline);
    #[cfg(not(target_arch = "wasm32"))]
    native::set_control_flow(event_loop, next_tick_deadline);
}

pub(crate) fn maybe_init_audio(app: &mut App) {
    #[cfg(target_arch = "wasm32")]
    wasm::maybe_init_audio(app);
    #[cfg(not(target_arch = "wasm32"))]
    native::maybe_init_audio(app);
}

pub(crate) fn poll_runtime_state(app: &mut App) -> bool {
    #[cfg(target_arch = "wasm32")]
    return wasm::poll_runtime_state(app);
    #[cfg(not(target_arch = "wasm32"))]
    return native::poll_runtime_state(app);
}

pub(crate) fn handle_redraw(app: &mut App) {
    #[cfg(target_arch = "wasm32")]
    wasm::handle_redraw(app);
    #[cfg(not(target_arch = "wasm32"))]
    native::handle_redraw(app);
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn set_nes_handle(nes: Rc<RefCell<Nes>>) {
    wasm::set_nes_handle(nes);
}
