//! The host side of the runtime worker.
//!
//! The worker owns the guest module. This side posts the wasm bytes and one frame input at a
//! time, and collects frame output, stdout and stderr lines, and traps.

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use pg_abi::{FrameOutput, Input};
use wasm_bindgen::{closure::Closure, JsCast as _, JsValue};
use web_sys::{MessageEvent, Worker, WorkerOptions, WorkerType};

/// Where the runtime worker script lives under the served root.
const WORKER_URL: &str = "public/runtime-worker.js";

/// How long a frame may take before the worker is treated as hung.
const FRAME_TIMEOUT_MS: f64 = 2000.0;

/// A line the guest wrote to stdout or stderr.
pub struct LogLine {
    pub stderr: bool,
    pub text: String,
}

/// What the worker has posted since the last drain.
#[derive(Default)]
struct Inbox {
    ready: bool,
    outputs: VecDeque<FrameOutput>,
    lines: Vec<LogLine>,
    trapped: Option<String>,
}

pub struct Runtime {
    worker: Worker,
    inbox: Rc<RefCell<Inbox>>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,

    /// When the frame in flight was posted, in milliseconds from `performance.now`.
    sent_at: Option<f64>,

    /// Set once the guest has run `_start` and exports the frame functions.
    loaded: bool,

    /// Why the guest stopped serving frames.
    dead: Option<String>,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.worker.terminate();
    }
}

impl Runtime {
    /// Spawns the worker. It stays idle until [`Runtime::load`] is called.
    pub fn new(ctx: &egui::Context) -> Result<Self, JsValue> {
        let options = WorkerOptions::new();
        options.set_type(WorkerType::Module);
        let worker = Worker::new_with_options(WORKER_URL, &options)?;

        let inbox = Rc::new(RefCell::new(Inbox::default()));
        let on_message = {
            let inbox = inbox.clone();
            let ctx = ctx.clone();
            Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                receive(&inbox, event);
                ctx.request_repaint();
            })
        };
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        let on_error = {
            let inbox = inbox.clone();
            let ctx = ctx.clone();
            Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                let message = event
                    .dyn_ref::<web_sys::ErrorEvent>()
                    .map_or_else(|| "worker error".to_owned(), |event| event.message());
                inbox.borrow_mut().trapped = Some(message);
                ctx.request_repaint();
            })
        };
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));

        Ok(Self {
            worker,
            inbox,
            _on_message: on_message,
            _on_error: on_error,
            sent_at: None,
            loaded: false,
            dead: None,
        })
    }

    /// Hands the guest module to the worker.
    pub fn load(&mut self, wasm: &[u8]) {
        self.loaded = false;
        self.sent_at = None;
        self.dead = None;
        self.inbox.borrow_mut().ready = false;
        self.post("load", "wasm", wasm);
    }

    /// Whether the guest has run `_start` and exports the frame functions.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Why the guest stopped serving frames, if it did.
    pub fn dead(&self) -> Option<&str> {
        self.dead.as_deref()
    }

    /// Whether a frame may be posted right now.
    pub fn is_idle(&self) -> bool {
        self.loaded && self.dead.is_none() && self.sent_at.is_none()
    }

    pub fn send_frame(&mut self, input: &Input) {
        let mut bytes = Vec::new();
        pg_abi::encode_input(input, &mut bytes);
        self.post("frame", "input", &bytes);
        self.sent_at = Some(now_ms());
    }

    /// Takes everything the worker posted since the last call.
    ///
    /// Returns the newest frame output, if any. Older ones are dropped.
    pub fn poll(&mut self, lines: &mut Vec<LogLine>) -> Option<FrameOutput> {
        let mut inbox = self.inbox.borrow_mut();

        if inbox.ready {
            inbox.ready = false;
            self.loaded = true;
        }

        lines.append(&mut inbox.lines);

        let output = inbox.outputs.pop_back();
        inbox.outputs.clear();
        if output.is_some() {
            self.sent_at = None;
        }

        if let Some(message) = inbox.trapped.take() {
            self.dead = Some(message);
            self.sent_at = None;
        }

        drop(inbox);

        let timed_out = self
            .sent_at
            .is_some_and(|sent_at| now_ms() - sent_at > FRAME_TIMEOUT_MS);
        if timed_out {
            self.worker.terminate();
            self.sent_at = None;
            self.dead = Some(format!(
                "the guest did not answer within {FRAME_TIMEOUT_MS:.0} ms"
            ));
        }

        output
    }

    /// Posts `{type, field: bytes}` with the bytes handed over rather than copied.
    fn post(&self, kind: &str, field: &str, bytes: &[u8]) {
        let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
        array.copy_from(bytes);
        let buffer = array.buffer();

        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&message, &"type".into(), &kind.into());
        let _ = js_sys::Reflect::set(&message, &field.into(), &buffer);

        let transfer = js_sys::Array::of1(&buffer);
        if let Err(error) = self.worker.post_message_with_transfer(&message, &transfer) {
            log::error!("could not post {kind} to the runtime worker: {error:?}");
        }
    }
}

fn receive(inbox: &Rc<RefCell<Inbox>>, event: MessageEvent) {
    let data = event.data();
    let kind = js_sys::Reflect::get(&data, &"type".into())
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default();

    let mut inbox = inbox.borrow_mut();
    match kind.as_str() {
        "ready" => inbox.ready = true,

        "frame" => {
            let Ok(buffer) = js_sys::Reflect::get(&data, &"output".into()) else {
                return;
            };
            let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
            match pg_abi::decode_output(&bytes) {
                Ok(output) => inbox.outputs.push_back(output),
                Err(error) => {
                    inbox.trapped = Some(format!("could not decode frame output: {error}"))
                }
            }
        }

        "stdout" | "stderr" => {
            let text = js_sys::Reflect::get(&data, &"text".into())
                .ok()
                .and_then(|value| value.as_string())
                .unwrap_or_default();
            inbox.lines.push(LogLine {
                stderr: kind == "stderr",
                text,
            });
        }

        "trapped" => {
            inbox.trapped = js_sys::Reflect::get(&data, &"message".into())
                .ok()
                .and_then(|value| value.as_string())
                .or_else(|| Some("the guest trapped".to_owned()));
        }

        _ => log::warn!("unknown message from the runtime worker: {kind}"),
    }
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
}
