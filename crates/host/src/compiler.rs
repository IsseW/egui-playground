//! The host side of the in-browser compiler.
//!
//! `public/runner.js` owns the toolchain download and a pool of compile workers. This side
//! submits source text and collects what comes back: the built module, rustc's diagnostics and
//! the phase timings.
//!
//! A submission cancels the one before it, so a call can resolve as cancelled. Those are dropped
//! here and never reach the app.

use std::{cell::Cell, cell::RefCell, rc::Rc};

use serde::Deserialize;
use wasm_bindgen::{closure::Closure, prelude::wasm_bindgen, JsValue};

use crate::editor::{Diagnostic, Severity};

#[wasm_bindgen]
extern "C" {
    /// Builds `source` into a `wasm32-wasip1` module. Resolves `{ json, wasm }`, where `wasm` is
    /// null when rustc emitted nothing. `status` takes progress strings.
    #[wasm_bindgen(js_namespace = window, catch)]
    async fn compileRust(source: String, status: &JsValue) -> Result<JsValue, JsValue>;

    /// Type-checks `source`. Resolves `{ json, wasm: null }`.
    #[wasm_bindgen(js_namespace = window, catch)]
    async fn checkRust(source: String, status: &JsValue) -> Result<JsValue, JsValue>;

    /// Downloads and stages the toolchain. `on_progress` takes received and total bytes.
    #[wasm_bindgen(js_namespace = window, catch)]
    async fn preloadRust(on_progress: &JsValue) -> Result<JsValue, JsValue>;
}

/// One message from rustc, as `compile-worker.js` parsed it out of the JSON diagnostics.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawDiagnostic {
    pub level: String,

    /// One-based, and none for a diagnostic that points at no span.
    pub line: Option<usize>,
    pub col: Option<usize>,
    pub end_line: Option<usize>,
    pub end_col: Option<usize>,

    /// rustc's own rendering of the message, with the ANSI colors stripped.
    pub rendered: String,
}

/// How long each phase of a build took.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stages {
    pub setup_ms: f64,
    pub rustc_instantiate_ms: f64,
    pub compile_ms: f64,
    pub link_ms: Option<f64>,
    pub total_ms: f64,
}

/// What one build or check produced.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CompileResult {
    pub ok: bool,

    /// A newer submission took over before this one ran.
    pub cancelled: bool,

    pub diagnostics: Vec<RawDiagnostic>,

    /// Output rustc produced outside of its JSON diagnostics.
    pub stderr: String,

    pub compile_ms: f64,
    pub link_ms: Option<f64>,
    pub stages: Stages,
}

impl CompileResult {
    /// The diagnostics that point at a span, in the shape the editor paints.
    pub fn editor_diagnostics(&self, file: &str) -> Vec<Diagnostic> {
        self.diagnostics
            .iter()
            .filter_map(|raw| {
                let line = raw.line?;
                let col = raw.col.unwrap_or(1);
                let span = match (raw.end_line, raw.end_col) {
                    (Some(end_line), Some(end_col)) if end_line == line && end_col > col => {
                        end_col - col
                    }
                    _ => 1,
                };
                Some(Diagnostic {
                    file: file.to_owned(),
                    line,
                    col,
                    span,
                    severity: match raw.level.as_str() {
                        "error" => Severity::Error,
                        _ => Severity::Warning,
                    },
                    message: raw.rendered.clone(),
                })
            })
            .collect()
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.level == "error")
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics.len() - self.error_count()
    }

    /// A one line summary of where the time went.
    pub fn timings(&self) -> String {
        let mut text = format!("compile {:.0} ms", self.compile_ms);
        if let Some(link_ms) = self.link_ms {
            text += &format!(", link {link_ms:.0} ms");
        }
        if self.stages.rustc_instantiate_ms > 0.0 {
            text += &format!(", rustc start {:.0} ms", self.stages.rustc_instantiate_ms);
        }
        text
    }
}

/// What the compiler finished doing, in submission order.
pub enum Outcome {
    /// Progress text, from the download or from a running job.
    Status(String),

    Built {
        result: CompileResult,

        /// The module, when rustc emitted one.
        wasm: Option<Vec<u8>>,
    },

    Checked(CompileResult),

    /// The call itself failed, so there is no result to read.
    Failed(String),
}

pub struct Compiler {
    outcomes: Rc<RefCell<Vec<Outcome>>>,

    /// How many submissions have not resolved yet.
    in_flight: Rc<Cell<usize>>,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            outcomes: Rc::new(RefCell::new(Vec::new())),
            in_flight: Rc::new(Cell::new(0)),
        }
    }

    /// Whether a build or a check has not resolved yet.
    pub fn is_busy(&self) -> bool {
        self.in_flight.get() > 0
    }

    /// Takes everything that finished since the last call.
    pub fn take(&self) -> Vec<Outcome> {
        std::mem::take(&mut *self.outcomes.borrow_mut())
    }

    /// Starts the toolchain download, reporting its progress as status lines.
    pub fn preload(&self, ctx: &egui::Context) {
        let outcomes = self.outcomes.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let progress = {
                let outcomes = outcomes.clone();
                let ctx = ctx.clone();
                Closure::<dyn Fn(f64, f64)>::new(move |received: f64, total: f64| {
                    let text = if total > 0.0 {
                        format!("toolchain {:.0}%", received / total * 100.0)
                    } else {
                        format!("toolchain {:.0} MB", received / 1e6)
                    };
                    outcomes.borrow_mut().push(Outcome::Status(text));
                    ctx.request_repaint();
                })
            };

            let result = preloadRust(progress.as_ref()).await;
            let outcome = match result {
                Ok(_) => Outcome::Status("toolchain ready".to_owned()),
                Err(error) => Outcome::Failed(format!("toolchain download failed: {error:?}")),
            };
            outcomes.borrow_mut().push(outcome);
            ctx.request_repaint();
        });
    }

    /// Builds `source` into a module, cancelling whatever is in flight.
    pub fn build(&self, ctx: &egui::Context, source: String) {
        self.submit(ctx, source, true);
    }

    /// Type-checks `source`, cancelling whatever is in flight.
    pub fn check(&self, ctx: &egui::Context, source: String) {
        self.submit(ctx, source, false);
    }

    fn submit(&self, ctx: &egui::Context, source: String, build: bool) {
        let outcomes = self.outcomes.clone();
        let in_flight = self.in_flight.clone();
        let ctx = ctx.clone();
        in_flight.set(in_flight.get() + 1);

        wasm_bindgen_futures::spawn_local(async move {
            let status = {
                let outcomes = outcomes.clone();
                let ctx = ctx.clone();
                Closure::<dyn Fn(String)>::new(move |text: String| {
                    outcomes.borrow_mut().push(Outcome::Status(text));
                    ctx.request_repaint();
                })
            };

            let call = if build {
                compileRust(source, status.as_ref()).await
            } else {
                checkRust(source, status.as_ref()).await
            };
            in_flight.set(in_flight.get().saturating_sub(1));

            let outcome = match call {
                Ok(value) => decode(&value, build),
                Err(error) => Some(Outcome::Failed(format!(
                    "the compiler call failed: {error:?}"
                ))),
            };
            if let Some(outcome) = outcome {
                outcomes.borrow_mut().push(outcome);
            }
            ctx.request_repaint();
        });
    }
}

/// Reads `{ json, wasm }`. Returns none for a submission a newer one took over.
fn decode(value: &JsValue, build: bool) -> Option<Outcome> {
    let json = js_sys::Reflect::get(value, &"json".into())
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default();

    let result: CompileResult = match serde_json::from_str(&json) {
        Ok(result) => result,
        Err(error) => {
            return Some(Outcome::Failed(format!(
                "could not read the compiler result: {error}"
            )))
        }
    };
    if result.cancelled {
        return None;
    }

    if !build {
        return Some(Outcome::Checked(result));
    }

    let wasm = js_sys::Reflect::get(value, &"wasm".into())
        .ok()
        .filter(|value| !value.is_null() && !value.is_undefined())
        .map(|value| js_sys::Uint8Array::new(&value).to_vec());

    Some(Outcome::Built { result, wasm })
}
