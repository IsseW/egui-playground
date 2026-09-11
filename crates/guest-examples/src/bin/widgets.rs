use egui_playground::{egui, run, App};

struct Widgets {
    name: String,
    age: f32,
    enabled: bool,
    choice: Choice,
}

#[derive(PartialEq)]
enum Choice {
    First,
    Second,
    Third,
}

impl Default for Widgets {
    fn default() -> Self {
        Self {
            name: "Arthur".to_owned(),
            age: 42.0,
            enabled: true,
            choice: Choice::First,
        }
    }
}

impl App for Widgets {
    fn ui(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Widgets");

            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.name);
            });

            ui.add(egui::Slider::new(&mut self.age, 0.0..=120.0).text("age"));
            ui.checkbox(&mut self.enabled, "enabled");

            ui.add_enabled_ui(self.enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.choice, Choice::First, "first");
                    ui.selectable_value(&mut self.choice, Choice::Second, "second");
                    ui.selectable_value(&mut self.choice, Choice::Third, "third");
                });
            });

            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                for row in 0..50 {
                    ui.label(format!("row {row}"));
                }
            });
        });
    }
}

fn main() {
    run(Widgets::default());
}
