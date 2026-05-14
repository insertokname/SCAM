mod app;
mod config;
mod platform;

use app::App;
#[cfg(not(target_arch = "wasm32"))]
use scamu::hardware::cartrige::Cartrige;
use winit::event_loop::EventLoop;

#[cfg(target_arch = "wasm32")]
use crate::platform::set_nes_handle;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use winit::platform::web::EventLoopExtWebSys;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let event_loop = EventLoop::new().unwrap();
    let mut app = App::new();

    let cartrige = Cartrige::from_bytes(include_bytes!("./nestest.nes")).unwrap();
    // let cartrige = Cartrige::from_bytes(include_bytes!("./AccuracyCoin.nes")).unwrap();
    // let cartrige = Cartrige::from_bytes(include_bytes!("./gitignored_games/smb.nes")).unwrap();
    // let cartrige = Cartrige::from_bytes(include_bytes!("./gitignored_games/pacman.nes")).unwrap();
    // let cartrige = Cartrige::from_bytes(include_bytes!("./gitignored_games/dk.nes")).unwrap();
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
    let app = App::new();
    set_nes_handle(app.nes.clone());

    event_loop.spawn_app(app);
    Ok(())
}
