//! The playground host: an eframe web app that runs a guest program in a worker and draws its
//! frames into a panel.

mod app;
mod editor;
mod guest;
mod runtime;

use wasm_bindgen::JsCast as _;

fn main() {
    console_error_panic_hook::set_once();
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let canvas = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("playground_canvas"))
        .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .expect("no canvas with id playground_canvas");

    wasm_bindgen_futures::spawn_local(async {
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(app::PlaygroundApp::new(cc)))),
            )
            .await;

        if let Err(error) = result {
            log::error!("could not start the playground: {error:?}");
        }
    });
}
