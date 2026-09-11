//! The code editor, with line numbers, syntax highlighting and compiler squiggles.

use egui::text::LayoutJob;

#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // The compiler side that builds these lands in phase 2.
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    fn color(self, visuals: &egui::Visuals) -> egui::Color32 {
        match self {
            Self::Error => visuals.error_fg_color,
            Self::Warning => visuals.warn_fg_color,
        }
    }
}

/// One compiler message pointing at a span of the source.
pub struct Diagnostic {
    pub file: String,

    /// One-based line of the first character of the span.
    pub line: usize,

    /// One-based column of the first character of the span.
    pub col: usize,

    /// How many characters the span covers, counted from `line` and `col`.
    pub span: usize,

    pub severity: Severity,

    pub message: String,
}

pub struct EditorState {
    pub code: String,

    /// The file the editor shows, matched against [`Diagnostic::file`].
    pub file: String,

    pub diagnostics: Vec<Diagnostic>,
}

impl EditorState {
    pub fn new(file: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            file: file.into(),
            diagnostics: Vec::new(),
        }
    }
}

/// Draws the editor and returns the `TextEdit` response.
pub fn ui(ui: &mut egui::Ui, state: &mut EditorState) -> egui::Response {
    let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());

    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut job = egui_extras::syntax_highlighting::highlight(
            ui.ctx(),
            ui.style(),
            &theme,
            buffer.as_str(),
            "rs",
        );
        job.wrap.max_width = wrap_width;
        ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
    };

    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let row_height = ui.ctx().fonts_mut(|fonts| fonts.row_height(&font_id));

    ui.take_available_space();
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                line_numbers(ui, &state.code, &font_id, row_height);

                let output = egui::TextEdit::multiline(&mut state.code)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .layouter(&mut layouter)
                    .show(ui);

                squiggles(ui, state, &output);

                output.response.response
            })
            .inner
        })
        .inner
}

/// Draws the line number strip to the left of the text.
fn line_numbers(ui: &mut egui::Ui, code: &str, font_id: &egui::FontId, row_height: f32) {
    let count = code.lines().count().max(1);
    let digits = count.to_string().len();

    let mut job = LayoutJob::default();
    let format = egui::TextFormat {
        font_id: font_id.clone(),
        color: ui.visuals().weak_text_color(),
        ..Default::default()
    };
    for line in 1..=count {
        job.append(&format!("{line:>digits$}\n"), 0.0, format.clone());
    }

    let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));
    let width = galley.size().x;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, count as f32 * row_height),
        egui::Sense::hover(),
    );
    ui.painter().galley(rect.min, galley, egui::Color32::WHITE);
}

/// Underlines every diagnostic span that falls inside the shown text.
fn squiggles(ui: &egui::Ui, state: &EditorState, output: &egui::text_edit::TextEditOutput) {
    let painter = ui.painter_at(output.response.response.rect);

    for diagnostic in &state.diagnostics {
        if diagnostic.file != state.file {
            continue;
        }

        let Some(start) = char_index(&state.code, diagnostic.line, diagnostic.col) else {
            continue;
        };
        let end = (start + diagnostic.span.max(1)).min(state.code.chars().count());

        let from = output
            .galley
            .pos_from_cursor(egui::text::CCursor::new(start));
        let to = output.galley.pos_from_cursor(egui::text::CCursor::new(end));

        let color = diagnostic.severity.color(ui.visuals());
        let origin = output.galley_pos;
        let y = origin.y + from.max.y - 1.0;
        let left = egui::pos2(origin.x + from.min.x, y);
        let right = egui::pos2(origin.x + to.max.x.max(from.min.x + 4.0), y);
        painter.line_segment([left, right], egui::Stroke::new(1.5, color));

        let span = egui::Rect::from_min_max(
            egui::pos2(left.x, origin.y + from.min.y),
            egui::pos2(right.x, y),
        );
        if ui.rect_contains_pointer(span) {
            egui::Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                ui.id().with(span.min.x as i32).with(span.min.y as i32),
                span,
            )
            .at_pointer()
            .show(|ui| ui.label(&diagnostic.message));
        }
    }
}

/// Turns a one-based line and column into an index into the characters of `code`.
fn char_index(code: &str, line: usize, col: usize) -> Option<usize> {
    let mut index = 0;
    for (number, text) in code.lines().enumerate() {
        if number + 1 == line {
            return Some(index + col.saturating_sub(1).min(text.chars().count()));
        }
        index += text.chars().count() + 1;
    }
    None
}
