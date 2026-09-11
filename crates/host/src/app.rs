//! The eframe app: editor on the left, console at the bottom, guest view in the middle.

use std::{cell::RefCell, rc::Rc};

use eframe::egui_wgpu::RenderState;
use pg_abi::{FrameOutput, Input};
use wasm_bindgen::{JsCast as _, JsValue};

use crate::{
    editor::{self, EditorState},
    guest::GuestView,
    runtime::{LogLine, Runtime},
};

/// The programs under `public/examples`, by file stem.
const EXAMPLES: &[&str] = &["hello", "painter", "text_edit", "widgets"];

const PLACEHOLDER: &str = "\
use egui_playground::{egui, run, App};

struct Program;

impl App for Program {
    fn ui(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label(\"hello\");
        });
    }
}

fn main() {
    run(Program);
}
";

/// Where a console line came from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Host,
    Stdout,
    Stderr,
}

struct ConsoleLine {
    source: Source,
    text: String,
}

/// What a background fetch produced.
enum Fetched {
    Example { name: String, wasm: Vec<u8> },
    Source { name: String, code: String },
    Failed(String),
}

pub struct PlaygroundApp {
    editor: EditorState,
    runtime: Option<Runtime>,
    guest: Option<GuestView>,

    console: Vec<ConsoleLine>,

    /// Name of the example the dropdown shows.
    example: String,

    /// The module the worker is running, kept so the restart button can post it again.
    wasm: Option<Rc<Vec<u8>>>,

    /// Output waiting to be painted into the guest texture.
    pending: Option<FrameOutput>,

    /// Events gathered for the next frame the guest takes.
    events: Vec<egui::Event>,

    fetched: Rc<RefCell<Vec<Fetched>>>,
}

impl PlaygroundApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let guest = cc.wgpu_render_state.as_ref().map(GuestView::new);

        let mut app = Self {
            editor: EditorState::new("hello.rs", PLACEHOLDER),
            runtime: None,
            guest,
            console: Vec::new(),
            example: EXAMPLES[0].to_owned(),
            wasm: None,
            pending: None,
            events: Vec::new(),
            fetched: Rc::new(RefCell::new(Vec::new())),
        };
        app.open_example(&cc.egui_ctx, EXAMPLES[0]);
        app
    }

    fn log(&mut self, source: Source, text: impl Into<String>) {
        self.console.push(ConsoleLine {
            source,
            text: text.into(),
        });
    }

    /// Fetches an example's module and source, then loads the module once it arrives.
    fn open_example(&mut self, ctx: &egui::Context, name: &str) {
        self.example = name.to_owned();
        self.editor.file = format!("{name}.rs");
        self.log(Source::Host, format!("loading example {name}"));

        for (suffix, wrap) in [
            (
                "wasm",
                (|name: String, bytes: Vec<u8>| Fetched::Example { name, wasm: bytes })
                    as fn(String, Vec<u8>) -> Fetched,
            ),
            ("rs", |name, bytes| Fetched::Source {
                name,
                code: String::from_utf8_lossy(&bytes).into_owned(),
            }),
        ] {
            let url = format!("public/examples/{name}.{suffix}");
            let fetched = self.fetched.clone();
            let ctx = ctx.clone();
            let name = name.to_owned();
            wasm_bindgen_futures::spawn_local(async move {
                let result = match fetch_bytes(&url).await {
                    Ok(bytes) => wrap(name, bytes),
                    Err(error) => Fetched::Failed(format!("could not fetch {url}: {error:?}")),
                };
                fetched.borrow_mut().push(result);
                ctx.request_repaint();
            });
        }
    }

    fn take_fetched(&mut self, ctx: &egui::Context) {
        let fetched: Vec<Fetched> = std::mem::take(&mut *self.fetched.borrow_mut());
        for item in fetched {
            match item {
                Fetched::Example { name, wasm } => {
                    if name != self.example {
                        continue;
                    }
                    self.log(Source::Host, format!("{} bytes of wasm", wasm.len()));
                    self.wasm = Some(Rc::new(wasm));
                    self.restart(ctx);
                }
                Fetched::Source { name, code } => {
                    if name == self.example {
                        self.editor.code = code;
                    }
                }
                Fetched::Failed(message) => self.log(Source::Stderr, message),
            }
        }
    }

    /// Spawns a new worker and hands it the module that is loaded.
    fn restart(&mut self, ctx: &egui::Context) {
        let Some(wasm) = self.wasm.clone() else {
            return;
        };

        match Runtime::new(ctx) {
            Ok(mut runtime) => {
                runtime.load(&wasm);
                self.runtime = Some(runtime);
            }
            Err(error) => {
                self.runtime = None;
                self.log(
                    Source::Stderr,
                    format!("could not spawn the runtime worker: {error:?}"),
                );
            }
        }
    }

    fn poll_runtime(&mut self) {
        let Some(runtime) = self.runtime.as_mut() else {
            return;
        };

        let was_loaded = runtime.is_loaded();
        let mut lines = Vec::new();
        let output = runtime.poll(&mut lines);
        let started = !was_loaded && runtime.is_loaded();

        if started {
            self.log(Source::Host, "the guest is running");
        }

        for LogLine { stderr, text } in lines {
            let source = if stderr {
                Source::Stderr
            } else {
                Source::Stdout
            };
            self.console.push(ConsoleLine { source, text });
        }

        if let Some(output) = output {
            self.set_pending(output);
        }
    }

    /// Keeps the newest frame output. The one it replaces has its deltas dropped.
    fn set_pending(&mut self, output: FrameOutput) {
        if let Some(mut old) = self.pending.take() {
            old.textures_delta.clear();
        }
        self.pending = Some(output);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::Image::new(egui::include_image!("../data/egui_logo.png"))
                    .fit_to_exact_size(egui::vec2(20.0, 20.0)),
            );
            ui.heading("egui playground");
            ui.separator();

            let run = ui.button("Run").on_hover_text("Ctrl+Enter");
            let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Enter);
            if run.clicked() || ui.input_mut(|i| i.consume_shortcut(&shortcut)) {
                self.log(
                    Source::Host,
                    "in-browser compilation is not wired up yet, reloading the prebuilt example",
                );
                let example = self.example.clone();
                self.open_example(ui.ctx(), &example);
            }

            let mut chosen = None;
            egui::ComboBox::from_id_salt("examples")
                .selected_text(&self.example)
                .show_ui(ui, |ui| {
                    for name in EXAMPLES {
                        if ui.selectable_label(self.example == *name, *name).clicked() {
                            chosen = Some((*name).to_owned());
                        }
                    }
                });
            if let Some(name) = chosen {
                self.open_example(ui.ctx(), &name);
            }

            let dead = self
                .runtime
                .as_ref()
                .and_then(|runtime| runtime.dead())
                .map(str::to_owned);
            if let Some(message) = dead {
                ui.separator();
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("guest died: {message}"),
                );
                if ui.button("Restart").clicked() {
                    let ctx = ui.ctx().clone();
                    self.restart(&ctx);
                }
            }
        });
    }

    fn console(&self, ui: &mut egui::Ui) {
        ui.take_available_space();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in &self.console {
                    let color = match line.source {
                        Source::Host => ui.visuals().weak_text_color(),
                        Source::Stdout => ui.visuals().text_color(),
                        Source::Stderr => ui.visuals().error_fg_color,
                    };
                    ui.label(egui::RichText::new(&line.text).monospace().color(color));
                }
            });
    }

    fn guest_view(&mut self, ui: &mut egui::Ui, render_state: &RenderState) {
        let Some(mut guest) = self.guest.take() else {
            ui.label("no wgpu render state");
            return;
        };
        self.draw_guest(ui, render_state, &mut guest);
        self.guest = Some(guest);
    }

    fn draw_guest(&mut self, ui: &mut egui::Ui, render_state: &RenderState, guest: &mut GuestView) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        if response.clicked() {
            response.request_focus();
        }

        let pixels_per_point = ui.ctx().pixels_per_point();
        let size = [
            (rect.width() * pixels_per_point).round().max(1.0) as u32,
            (rect.height() * pixels_per_point).round().max(1.0) as u32,
        ];
        guest.resize(render_state, size);

        if let Some(mut output) = self.pending.take() {
            guest.paint(render_state, &mut output);

            if let Some(text) = output.copied_text {
                ui.ctx().copy_text(text);
            }
            if let Some(url) = output.open_url {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(output.cursor_icon);
            }
            if let Some(ms) = output.repaint_after_ms {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(ms.into()));
            }
        }

        guest.image(rect.size()).paint_at(ui, rect);

        // Events pile up between sends, since the guest runs one frame at a time.
        if response.hovered() || response.has_focus() {
            let events = ui.input(|i| i.events.clone());
            self.events.extend(
                events
                    .into_iter()
                    .map(|event| translate(event, -rect.min.to_vec2())),
            );
        }

        let Some(runtime) = self.runtime.as_mut() else {
            return;
        };
        if !runtime.is_idle() {
            return;
        }

        let events = std::mem::take(&mut self.events);
        let (time, predicted_dt, max_texture_side) =
            ui.input(|i| (i.time, i.predicted_dt, i.max_texture_side));

        runtime.send_frame(&Input {
            screen_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, rect.size()),
            pixels_per_point,
            time,
            predicted_dt,
            focused: response.has_focus(),
            max_texture_side,
            events,
        });
    }
}

impl eframe::App for PlaygroundApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.take_fetched(&ctx);
        self.poll_runtime();

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));

        egui::Panel::bottom("console")
            .resizable(true)
            .default_size(160.0)
            .show(ui, |ui| self.console(ui));

        egui::Panel::left("editor")
            .resizable(true)
            .default_size(480.0)
            .show(ui, |ui| {
                editor::ui(ui, &mut self.editor);
            });

        egui::CentralPanel::no_frame().show(ui, |ui| match frame.wgpu_render_state() {
            Some(render_state) => {
                let render_state = render_state.clone();
                self.guest_view(ui, &render_state);
            }
            None => {
                ui.label("no wgpu render state");
            }
        });
    }
}

/// Moves the positions in an event into guest-local coordinates.
fn translate(event: egui::Event, offset: egui::Vec2) -> egui::Event {
    match event {
        egui::Event::PointerMoved(pos) => egui::Event::PointerMoved(pos + offset),
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        } => egui::Event::PointerButton {
            pos: pos + offset,
            button,
            pressed,
            modifiers,
        },
        egui::Event::Touch {
            device_id,
            id,
            phase,
            pos,
            force,
        } => egui::Event::Touch {
            device_id,
            id,
            phase,
            pos: pos + offset,
            force,
        },
        other => other,
    }
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let response: web_sys::Response =
        wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
            .await?
            .dyn_into()?;

    if !response.ok() {
        return Err(JsValue::from_str(&format!("status {}", response.status())));
    }

    let buffer = wasm_bindgen_futures::JsFuture::from(response.array_buffer()?).await?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}
