use egui_playground::{egui, run, App};

struct TextEditor {
    text: String,
}

impl Default for TextEditor {
    fn default() -> Self {
        Self {
            text: "Type here.\nSelection, copy and paste all go through the host.".to_owned(),
        }
    }
}

impl App for TextEditor {
    fn ui(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("stats").show(ui, |ui| {
            ui.label(format!(
                "{} chars, {} lines",
                self.text.chars().count(),
                self.text.lines().count()
            ));
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_sized(
                ui.available_size(),
                egui::TextEdit::multiline(&mut self.text).code_editor(),
            );
        });
    }
}

fn main() {
    run(TextEditor::default());
}
