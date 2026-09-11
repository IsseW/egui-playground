//! The prelude a playground program links against.
//!
//! A program calls [`run`] from `main` to hand over its app, and the host then drives it one
//! frame at a time through the exported [`pg_frame`].
//!
//! ```no_run
//! use egui_playground::{run, App};
//!
//! #[derive(Default)]
//! struct Counter {
//!     clicks: u32,
//! }
//!
//! impl App for Counter {
//!     fn ui(&mut self, ui: &mut egui::Ui) {
//!         egui::CentralPanel::default().show(ui, |ui| {
//!             if ui.button("click me").clicked() {
//!                 self.clicks += 1;
//!             }
//!             ui.label(format!("{} clicks", self.clicks));
//!         });
//!     }
//! }
//!
//! fn main() {
//!     run(Counter::default());
//! }
//! ```

mod exports;

pub use egui;
pub use exports::{pg_alloc, pg_frame, pg_free};

use std::cell::RefCell;

use egui::{Context, Ui, ViewportId};
use pg_abi::{FrameOutput, Input};

/// A playground program.
pub trait App {
    /// Called once per frame. The [`Ui`] covers the whole screen and has no margin or
    /// background, so wrap the contents in a panel or a [`egui::Frame`].
    fn ui(&mut self, ui: &mut Ui);
}

impl<F: FnMut(&mut Ui)> App for F {
    fn ui(&mut self, ui: &mut Ui) {
        self(ui);
    }
}

/// Hands the app to the host and returns straight away.
///
/// Call this at the end of `main`. The host drives the frames.
pub fn run(app: impl App + 'static) {
    STATE.with_borrow_mut(|state| {
        *state = Some(State {
            ctx: Context::default(),
            app: Box::new(app),
            encoded: Vec::new(),
        });
    });
}

struct State {
    ctx: Context,
    app: Box<dyn App>,
    /// Kept alive between calls because the host reads it after [`pg_frame`] returns.
    encoded: Vec<u8>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Runs one frame and returns a pointer to a `u32` byte count followed by the encoded output.
///
/// # Panics
/// If `main` never called [`run`], or the input buffer does not decode.
pub(crate) fn frame(input_bytes: &[u8]) -> *const u8 {
    STATE.with_borrow_mut(|state| {
        let state = state
            .as_mut()
            .expect("no app registered: call egui_playground::run(app) from main");

        let input = pg_abi::decode_input(input_bytes).expect("could not decode frame input");
        let mut output = state.run_frame(&input);

        state.encoded.clear();
        state.encoded.extend_from_slice(&0u32.to_le_bytes());
        pg_abi::encode_output(&output, &mut state.encoded);
        let payload_len = (state.encoded.len() - 4) as u32;
        state.encoded[..4].copy_from_slice(&payload_len.to_le_bytes());

        // `TexturesDelta` asserts on drop that every delta was applied.
        output.textures_delta.clear();

        state.encoded.as_ptr()
    })
}

impl State {
    fn run_frame(&mut self, input: &Input) -> FrameOutput {
        let app = &mut self.app;
        let full_output = self
            .ctx
            .run_ui(input.to_raw_input(), |ui| app.ui(ui));

        let primitives = self
            .ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point)
            .into_iter()
            .filter_map(|primitive| match primitive.primitive {
                egui::epaint::Primitive::Mesh(mesh) => Some((primitive.clip_rect, mesh)),
                egui::epaint::Primitive::Callback(_) => None,
            })
            .collect();

        let mut copied_text = None;
        let mut open_url = None;
        for command in full_output.platform_output.commands {
            match command {
                egui::OutputCommand::CopyText(text) => copied_text = Some(text),
                egui::OutputCommand::OpenUrl(url) => open_url = Some(url.url),
                egui::OutputCommand::CopyImage(_) => {}
            }
        }

        let repaint_delay = full_output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map(|viewport| viewport.repaint_delay);
        let repaint_after_ms = match repaint_delay {
            Some(delay) => u32::try_from(delay.as_millis()).ok(),
            None => None,
        };

        FrameOutput {
            pixels_per_point: full_output.pixels_per_point,
            primitives,
            textures_delta: full_output.textures_delta,
            cursor_icon: full_output.platform_output.cursor_icon,
            copied_text,
            open_url,
            repaint_after_ms,
        }
    }
}
